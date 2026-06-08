// Phase 10.2 wires the engine into the preview, but several helpers
// (visual selection range queries, search infrastructure, char-class
// API) are still only used by tests and future sub-phases. Keep the
// allow narrow rather than per-symbol for now; trim in 10.4/10.6.
#![allow(dead_code)]

//! vim-style modal editing engine for the preview editor.
//!
//! Pure state machine — egui-free — so the host (preview pane in Phase 10.2)
//! can feed it `VimEvent`s and read back the buffer / cursor / mode each
//! frame. Unit-testable without standing up egui.
//!
//! Scope (per `docs/plan.md` Phase 10 §3):
//! - Modes: Normal / Insert / Visual
//! - Motions: h/j/k/l, w/b/e, 0/$, gg/G
//! - Edits: i/a/o, x, dd, yy, p, u, Ctrl+R
//! - Search (`/`, n, N) lives in this module too (Phase 10.6 wires the UI).

use std::ops::Range;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
}

/// One input event handed to the engine. The host translates platform
/// key events (egui::Event, etc.) into these.
#[derive(Debug, Clone)]
pub enum VimEvent {
    /// A printable character. In Normal mode this is the command key;
    /// in Insert it's the character to insert.
    Char(char),
    Escape,
    Enter,
    Backspace,
    /// Ctrl+R — redo. Carried separately because we don't want callers
    /// to encode modifier state inside `Char`.
    CtrlR,
}

/// Side-effect summary so the host knows whether to repaint / save.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Action {
    pub buffer_changed: bool,
    pub mode_changed: bool,
    pub cursor_moved: bool,
}

impl Action {
    pub fn buffer_changed() -> Self {
        Self {
            buffer_changed: true,
            cursor_moved: true,
            ..Default::default()
        }
    }
    pub fn cursor_moved() -> Self {
        Self {
            cursor_moved: true,
            ..Default::default()
        }
    }
    pub fn mode_changed() -> Self {
        Self {
            mode_changed: true,
            cursor_moved: true,
            ..Default::default()
        }
    }
}

/// Multi-key command prefixes (dd, yy, gg).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    None,
    G,
    D,
    Y,
}

/// Snapshot taken before every buffer-mutating op so `u` can rewind.
#[derive(Debug, Clone)]
struct Snapshot {
    buffer: String,
    cursor: usize,
}

#[derive(Debug)]
pub struct VimEngine {
    mode: Mode,
    /// The text being edited. Byte-indexed throughout; the engine is
    /// careful to keep `cursor` on a UTF-8 char boundary.
    buffer: String,
    /// Byte offset into `buffer`. May equal `buffer.len()` when the
    /// cursor is "after" the last char.
    cursor: usize,
    /// Start of the visual selection (byte offset). The other end is
    /// `cursor`. `None` when not in Visual mode.
    visual_anchor: Option<usize>,
    /// Last yanked text. Linewise yanks include a trailing `\n` so
    /// `p` reinserts them as a new line.
    yank: String,
    /// True when the yank buffer was filled by a linewise op (`dd`,
    /// `yy`, visual line-mode in future). `p` uses this to decide
    /// where to paste.
    yank_linewise: bool,
    pending: Pending,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Phase 10.6: search state.
    /// `Some(_)` while the user is typing in the `/`-prompt; the
    /// string is the query so far. `None` when not prompting.
    search_prompt: Option<String>,
    /// Most recently committed search query (from `/` + Enter). Used
    /// by `n` and `N` to repeat the search.
    search_query: String,
    /// Byte ranges in `buffer` matching `search_query`. Recomputed
    /// after every commit and on `n` / `N` if the buffer changed.
    /// Empty when there are no matches or no committed query.
    search_matches: Vec<Range<usize>>,
    /// Index into `search_matches` of the currently-highlighted match.
    current_match: Option<usize>,
}

impl VimEngine {
    pub fn new(buffer: String) -> Self {
        Self {
            mode: Mode::Normal,
            buffer,
            cursor: 0,
            visual_anchor: None,
            yank: String::new(),
            yank_linewise: false,
            pending: Pending::None,
            undo: Vec::new(),
            redo: Vec::new(),
            search_prompt: None,
            search_query: String::new(),
            search_matches: Vec::new(),
            current_match: None,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn buffer(&self) -> &str {
        &self.buffer
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    /// Visual selection range, `None` outside Visual mode. Always
    /// returns `start <= end`. Includes the character under the
    /// cursor — matches vim's "inclusive" visual selection.
    pub fn visual_range(&self) -> Option<Range<usize>> {
        let anchor = self.visual_anchor?;
        let (lo, hi) = if anchor <= self.cursor {
            (anchor, self.cursor)
        } else {
            (self.cursor, anchor)
        };
        let end = next_char_boundary(&self.buffer, hi);
        Some(lo..end)
    }

    /// Replace the buffer wholesale (used by the host on reload).
    /// Resets cursor to 0 and clears undo history because the new
    /// content is unrelated to the old.
    pub fn replace_buffer(&mut self, new_buffer: String) {
        self.buffer = new_buffer;
        self.cursor = 0;
        self.visual_anchor = None;
        self.pending = Pending::None;
        self.undo.clear();
        self.redo.clear();
        self.mode = Mode::Normal;
        self.search_prompt = None;
        self.search_query.clear();
        self.search_matches.clear();
        self.current_match = None;
    }

    /// Phase 10.6: read-only access to the live `/`-prompt input.
    /// `None` when the user is not in the search prompt.
    pub fn search_prompt(&self) -> Option<&str> {
        self.search_prompt.as_deref()
    }

    /// Phase 10.6: all match ranges in `buffer` for the last
    /// committed `/` query.
    pub fn search_matches(&self) -> &[Range<usize>] {
        &self.search_matches
    }

    /// Phase 10.6: index into `search_matches()` of the currently
    /// active match, or `None` when the cursor isn't on one.
    pub fn current_match(&self) -> Option<usize> {
        self.current_match
    }

    /// Feed one event. Returns what changed so the host knows whether
    /// to persist / repaint.
    pub fn apply(&mut self, event: VimEvent) -> Action {
        // Phase 10.6: search prompt swallows input until Enter or
        // Escape, regardless of which mode we were in when `/` was
        // pressed. Practically we only enter the prompt from Normal,
        // but routing here keeps that flexibility.
        if self.search_prompt.is_some() {
            return self.apply_search_prompt(event);
        }
        match self.mode {
            Mode::Normal => self.apply_normal(event),
            Mode::Insert => self.apply_insert(event),
            Mode::Visual => self.apply_visual(event),
        }
    }

    // ------------------------------------------------------------------
    // Normal mode
    // ------------------------------------------------------------------

    fn apply_normal(&mut self, event: VimEvent) -> Action {
        // Multi-key sequences first: gg / dd / yy.
        let pending = std::mem::replace(&mut self.pending, Pending::None);
        match (pending, &event) {
            (Pending::G, VimEvent::Char('g')) => return self.motion_buffer_start(),
            (Pending::D, VimEvent::Char('d')) => return self.delete_line(),
            (Pending::Y, VimEvent::Char('y')) => return self.yank_line(),
            // Any other key after a pending prefix aborts the prefix
            // and falls through to normal dispatch.
            _ => {}
        }

        match event {
            VimEvent::Escape => Action::default(),
            VimEvent::CtrlR => self.redo(),
            VimEvent::Char(c) => match c {
                'h' => self.motion_left(),
                'l' => self.motion_right(),
                'j' => self.motion_down(),
                'k' => self.motion_up(),
                '0' => self.motion_line_start(),
                '$' => self.motion_line_end(),
                'w' => self.motion_word_forward(),
                'b' => self.motion_word_backward(),
                'e' => self.motion_word_end(),
                'g' => {
                    self.pending = Pending::G;
                    Action::default()
                }
                'G' => self.motion_buffer_end(),
                'd' => {
                    self.pending = Pending::D;
                    Action::default()
                }
                'y' => {
                    self.pending = Pending::Y;
                    Action::default()
                }
                'p' => self.paste_after(),
                'x' => self.delete_char(),
                'u' => self.undo(),
                'i' => self.enter_insert_at_cursor(),
                'I' => self.enter_insert_line_start(),
                'a' => self.enter_insert_after_cursor(),
                'A' => self.enter_insert_line_end(),
                'o' => self.open_line_below(),
                'O' => self.open_line_above(),
                'v' => self.enter_visual(),
                '/' => self.start_search_prompt(),
                'n' => self.next_match(),
                'N' => self.prev_match(),
                _ => Action::default(),
            },
            _ => Action::default(),
        }
    }

    // ------------------------------------------------------------------
    // Search
    // ------------------------------------------------------------------

    fn start_search_prompt(&mut self) -> Action {
        self.search_prompt = Some(String::new());
        Action::default()
    }

    fn apply_search_prompt(&mut self, event: VimEvent) -> Action {
        match event {
            VimEvent::Escape => {
                self.search_prompt = None;
                Action::default()
            }
            VimEvent::Enter => {
                if let Some(query) = self.search_prompt.take() {
                    self.execute_search(query);
                }
                Action::cursor_moved()
            }
            VimEvent::Backspace => {
                if let Some(q) = self.search_prompt.as_mut() {
                    q.pop();
                }
                Action::default()
            }
            VimEvent::Char(c) => {
                if let Some(q) = self.search_prompt.as_mut() {
                    q.push(c);
                }
                Action::default()
            }
            VimEvent::CtrlR => Action::default(),
        }
    }

    fn execute_search(&mut self, query: String) {
        if query.is_empty() {
            self.search_query.clear();
            self.search_matches.clear();
            self.current_match = None;
            return;
        }
        self.search_query = query;
        self.recompute_matches();
        if self.search_matches.is_empty() {
            self.current_match = None;
            return;
        }
        // Jump to first match at or after the cursor, wrapping to 0.
        let from_cursor = self
            .search_matches
            .iter()
            .position(|r| r.start >= self.cursor);
        let idx = from_cursor.unwrap_or(0);
        self.current_match = Some(idx);
        self.cursor = self.search_matches[idx].start;
    }

    fn recompute_matches(&mut self) {
        self.search_matches.clear();
        if self.search_query.is_empty() {
            return;
        }
        for (start, m) in self.buffer.match_indices(&self.search_query) {
            self.search_matches.push(start..(start + m.len()));
        }
    }

    fn next_match(&mut self) -> Action {
        if self.search_query.is_empty() {
            return Action::default();
        }
        self.recompute_matches();
        if self.search_matches.is_empty() {
            self.current_match = None;
            return Action::default();
        }
        let len = self.search_matches.len();
        let next = match self.current_match {
            Some(i) => (i + 1) % len,
            None => 0,
        };
        self.current_match = Some(next);
        self.cursor = self.search_matches[next].start;
        Action::cursor_moved()
    }

    fn prev_match(&mut self) -> Action {
        if self.search_query.is_empty() {
            return Action::default();
        }
        self.recompute_matches();
        if self.search_matches.is_empty() {
            self.current_match = None;
            return Action::default();
        }
        let len = self.search_matches.len();
        let prev = match self.current_match {
            Some(0) => len - 1,
            Some(i) => i - 1,
            None => len - 1,
        };
        self.current_match = Some(prev);
        self.cursor = self.search_matches[prev].start;
        Action::cursor_moved()
    }

    // ------------------------------------------------------------------
    // Insert mode
    // ------------------------------------------------------------------

    fn apply_insert(&mut self, event: VimEvent) -> Action {
        match event {
            VimEvent::Escape => {
                self.mode = Mode::Normal;
                // Conventional vim: stepping out of insert moves the
                // cursor one to the left, but only if it's not at the
                // start of a line.
                self.step_back_within_line();
                Action::mode_changed()
            }
            VimEvent::Char(c) => self.insert_char(c),
            VimEvent::Enter => self.insert_char('\n'),
            VimEvent::Backspace => self.insert_backspace(),
            VimEvent::CtrlR => Action::default(),
        }
    }

    fn insert_char(&mut self, c: char) -> Action {
        self.snapshot_for_undo();
        self.buffer.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        Action::buffer_changed()
    }

    fn insert_backspace(&mut self) -> Action {
        if self.cursor == 0 {
            return Action::default();
        }
        self.snapshot_for_undo();
        // Find the previous char boundary.
        let prev = prev_char_boundary(&self.buffer, self.cursor);
        let removed = self.buffer.drain(prev..self.cursor).count();
        let _ = removed;
        self.cursor = prev;
        Action::buffer_changed()
    }

    // ------------------------------------------------------------------
    // Visual mode
    // ------------------------------------------------------------------

    fn apply_visual(&mut self, event: VimEvent) -> Action {
        match event {
            VimEvent::Escape => self.exit_visual(),
            VimEvent::Char('h') => self.motion_left(),
            VimEvent::Char('l') => self.motion_right(),
            VimEvent::Char('j') => self.motion_down(),
            VimEvent::Char('k') => self.motion_up(),
            VimEvent::Char('y') => self.yank_selection(),
            VimEvent::Char('d') | VimEvent::Char('x') => self.delete_selection(),
            _ => Action::default(),
        }
    }

    fn enter_visual(&mut self) -> Action {
        self.mode = Mode::Visual;
        self.visual_anchor = Some(self.cursor);
        Action::mode_changed()
    }

    fn exit_visual(&mut self) -> Action {
        self.mode = Mode::Normal;
        self.visual_anchor = None;
        Action::mode_changed()
    }

    fn yank_selection(&mut self) -> Action {
        let Some(range) = self.visual_range() else {
            return Action::default();
        };
        self.yank = self.buffer[range].to_string();
        self.yank_linewise = false;
        self.exit_visual();
        Action::mode_changed()
    }

    fn delete_selection(&mut self) -> Action {
        let Some(range) = self.visual_range() else {
            return Action::default();
        };
        self.snapshot_for_undo();
        self.yank = self.buffer[range.clone()].to_string();
        self.yank_linewise = false;
        self.buffer.drain(range.clone());
        self.cursor = range.start;
        self.exit_visual();
        Action::buffer_changed()
    }

    // ------------------------------------------------------------------
    // Motions
    // ------------------------------------------------------------------

    fn motion_left(&mut self) -> Action {
        if self.cursor == 0 {
            return Action::default();
        }
        let prev = prev_char_boundary(&self.buffer, self.cursor);
        // Don't cross a newline going left (vim's `h` is line-bounded).
        if &self.buffer[prev..self.cursor] == "\n" {
            return Action::default();
        }
        self.cursor = prev;
        Action::cursor_moved()
    }

    fn motion_right(&mut self) -> Action {
        if self.cursor >= self.buffer.len() {
            return Action::default();
        }
        let next = next_char_boundary(&self.buffer, self.cursor);
        if &self.buffer[self.cursor..next] == "\n" {
            return Action::default();
        }
        self.cursor = next;
        Action::cursor_moved()
    }

    fn motion_down(&mut self) -> Action {
        let (line_start, line_end) = line_bounds(&self.buffer, self.cursor);
        let col = column_of(&self.buffer, line_start, self.cursor);
        // Find the next line.
        if line_end >= self.buffer.len() {
            return Action::default();
        }
        let next_line_start = line_end + 1; // skip the '\n'
        let (_, next_line_end) = line_bounds(&self.buffer, next_line_start);
        self.cursor = column_to_offset(&self.buffer, next_line_start, next_line_end, col);
        Action::cursor_moved()
    }

    fn motion_up(&mut self) -> Action {
        let (line_start, _) = line_bounds(&self.buffer, self.cursor);
        if line_start == 0 {
            return Action::default();
        }
        let col = column_of(&self.buffer, line_start, self.cursor);
        // The previous line ends at line_start - 1 (the '\n' just
        // before our line).
        let prev_line_end = line_start - 1;
        let (prev_line_start, _) = line_bounds(&self.buffer, prev_line_end);
        self.cursor = column_to_offset(&self.buffer, prev_line_start, prev_line_end, col);
        Action::cursor_moved()
    }

    fn motion_line_start(&mut self) -> Action {
        let (start, _) = line_bounds(&self.buffer, self.cursor);
        self.cursor = start;
        Action::cursor_moved()
    }

    fn motion_line_end(&mut self) -> Action {
        let (_, end) = line_bounds(&self.buffer, self.cursor);
        self.cursor = end;
        Action::cursor_moved()
    }

    fn motion_word_forward(&mut self) -> Action {
        // Skip current word, then any whitespace, land on next word's
        // first char. Vim's `w` is more nuanced (punctuation class
        // changes), but this minimal version covers common cases.
        let bytes = self.buffer.as_bytes();
        let mut i = self.cursor;
        let class = char_class_at(bytes, i);
        while i < bytes.len() && char_class_at(bytes, i) == class {
            i = next_char_boundary(&self.buffer, i);
        }
        while i < bytes.len() && char_class_at(bytes, i) == CharClass::Whitespace {
            i = next_char_boundary(&self.buffer, i);
        }
        if i == self.cursor {
            return Action::default();
        }
        self.cursor = i;
        Action::cursor_moved()
    }

    fn motion_word_backward(&mut self) -> Action {
        // Mirror of `w`.
        if self.cursor == 0 {
            return Action::default();
        }
        let bytes = self.buffer.as_bytes();
        let mut i = prev_char_boundary(&self.buffer, self.cursor);
        while i > 0 && char_class_at(bytes, i) == CharClass::Whitespace {
            i = prev_char_boundary(&self.buffer, i);
        }
        let class = char_class_at(bytes, i);
        while i > 0 {
            let prev = prev_char_boundary(&self.buffer, i);
            if char_class_at(bytes, prev) != class {
                break;
            }
            i = prev;
        }
        self.cursor = i;
        Action::cursor_moved()
    }

    fn motion_word_end(&mut self) -> Action {
        let bytes = self.buffer.as_bytes();
        let mut i = self.cursor;
        // Step forward at least once so consecutive `e` advances.
        if i < bytes.len() {
            i = next_char_boundary(&self.buffer, i);
        }
        while i < bytes.len() && char_class_at(bytes, i) == CharClass::Whitespace {
            i = next_char_boundary(&self.buffer, i);
        }
        if i >= bytes.len() {
            return Action::default();
        }
        let class = char_class_at(bytes, i);
        while i < bytes.len() {
            let next = next_char_boundary(&self.buffer, i);
            if next >= bytes.len() || char_class_at(bytes, next) != class {
                break;
            }
            i = next;
        }
        self.cursor = i;
        Action::cursor_moved()
    }

    fn motion_buffer_start(&mut self) -> Action {
        self.cursor = 0;
        Action::cursor_moved()
    }

    fn motion_buffer_end(&mut self) -> Action {
        // `G` lands on the last line's first non-blank in real vim;
        // here we just put cursor on the first char of the last line
        // for simplicity.
        let last_line_start = match self.buffer.rfind('\n') {
            Some(idx) if idx + 1 < self.buffer.len() => idx + 1,
            _ => 0,
        };
        self.cursor = last_line_start;
        Action::cursor_moved()
    }

    // ------------------------------------------------------------------
    // Edits
    // ------------------------------------------------------------------

    fn delete_char(&mut self) -> Action {
        if self.cursor >= self.buffer.len() {
            return Action::default();
        }
        let next = next_char_boundary(&self.buffer, self.cursor);
        // `x` shouldn't eat a newline.
        if &self.buffer[self.cursor..next] == "\n" {
            return Action::default();
        }
        self.snapshot_for_undo();
        self.yank = self.buffer[self.cursor..next].to_string();
        self.yank_linewise = false;
        self.buffer.drain(self.cursor..next);
        // Vim keeps cursor on the char that took the removed one's
        // place, except at line end where it steps back.
        if self.cursor >= self.buffer.len()
            || self.buffer.as_bytes().get(self.cursor) == Some(&b'\n')
        {
            self.step_back_within_line();
        }
        Action::buffer_changed()
    }

    fn delete_line(&mut self) -> Action {
        self.snapshot_for_undo();
        let (start, end) = line_bounds(&self.buffer, self.cursor);
        let mut delete_end = end;
        let include_trailing_newline = end < self.buffer.len();
        if include_trailing_newline {
            delete_end += 1;
        }
        self.yank = self.buffer[start..delete_end].to_string();
        if !self.yank.ends_with('\n') {
            self.yank.push('\n');
        }
        self.yank_linewise = true;
        self.buffer.drain(start..delete_end);
        // Cursor goes to the start of the next line (or new last
        // line if we deleted the bottom one).
        self.cursor = start.min(self.buffer.len());
        Action::buffer_changed()
    }

    fn yank_line(&mut self) -> Action {
        let (start, end) = line_bounds(&self.buffer, self.cursor);
        let mut text = self.buffer[start..end].to_string();
        text.push('\n');
        self.yank = text;
        self.yank_linewise = true;
        Action::default()
    }

    fn paste_after(&mut self) -> Action {
        if self.yank.is_empty() {
            return Action::default();
        }
        self.snapshot_for_undo();
        let yank = self.yank.clone();
        if self.yank_linewise {
            let (_, end) = line_bounds(&self.buffer, self.cursor);
            // For a non-final line, the '\n' lives at byte `end`; we
            // want to insert after it. For the final line without a
            // trailing newline, append one first, then drop the yank
            // immediately after.
            let insert_at = if end < self.buffer.len() {
                end + 1
            } else {
                self.buffer.push('\n');
                self.buffer.len()
            };
            self.buffer.insert_str(insert_at, &yank);
            self.cursor = insert_at;
        } else {
            let insert_at = next_char_boundary(&self.buffer, self.cursor);
            self.buffer.insert_str(insert_at, &yank);
            self.cursor = insert_at;
        }
        Action::buffer_changed()
    }

    fn enter_insert_at_cursor(&mut self) -> Action {
        self.mode = Mode::Insert;
        Action::mode_changed()
    }

    fn enter_insert_after_cursor(&mut self) -> Action {
        self.mode = Mode::Insert;
        if self.cursor < self.buffer.len()
            && self.buffer.as_bytes().get(self.cursor) != Some(&b'\n')
        {
            self.cursor = next_char_boundary(&self.buffer, self.cursor);
        }
        Action::mode_changed()
    }

    fn open_line_below(&mut self) -> Action {
        self.snapshot_for_undo();
        let (_, end) = line_bounds(&self.buffer, self.cursor);
        self.buffer.insert(end, '\n');
        self.cursor = end + 1;
        self.mode = Mode::Insert;
        Action {
            buffer_changed: true,
            mode_changed: true,
            cursor_moved: true,
        }
    }

    fn open_line_above(&mut self) -> Action {
        self.snapshot_for_undo();
        let (start, _) = line_bounds(&self.buffer, self.cursor);
        self.buffer.insert(start, '\n');
        self.cursor = start;
        self.mode = Mode::Insert;
        Action {
            buffer_changed: true,
            mode_changed: true,
            cursor_moved: true,
        }
    }

    fn enter_insert_line_start(&mut self) -> Action {
        let (start, _) = line_bounds(&self.buffer, self.cursor);
        self.cursor = start;
        self.mode = Mode::Insert;
        Action::mode_changed()
    }

    fn enter_insert_line_end(&mut self) -> Action {
        let (_, end) = line_bounds(&self.buffer, self.cursor);
        self.cursor = end;
        self.mode = Mode::Insert;
        Action::mode_changed()
    }

    // ------------------------------------------------------------------
    // Undo / redo
    // ------------------------------------------------------------------

    fn snapshot_for_undo(&mut self) {
        self.undo.push(Snapshot {
            buffer: self.buffer.clone(),
            cursor: self.cursor,
        });
        self.redo.clear();
    }

    fn undo(&mut self) -> Action {
        let Some(snap) = self.undo.pop() else {
            return Action::default();
        };
        self.redo.push(Snapshot {
            buffer: self.buffer.clone(),
            cursor: self.cursor,
        });
        self.buffer = snap.buffer;
        self.cursor = snap.cursor;
        Action::buffer_changed()
    }

    fn redo(&mut self) -> Action {
        let Some(snap) = self.redo.pop() else {
            return Action::default();
        };
        self.undo.push(Snapshot {
            buffer: self.buffer.clone(),
            cursor: self.cursor,
        });
        self.buffer = snap.buffer;
        self.cursor = snap.cursor;
        Action::buffer_changed()
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn step_back_within_line(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = prev_char_boundary(&self.buffer, self.cursor);
        if &self.buffer[prev..self.cursor] == "\n" {
            return;
        }
        self.cursor = prev;
    }
}

// --------------------------------------------------------------------------
// Pure byte-level helpers (no `self` so they're trivially testable).
// --------------------------------------------------------------------------

/// Return `(line_start, line_end)` byte offsets in `s` for the line
/// containing `pos`. `line_end` is the offset *before* the `\n`
/// (so it equals `line_start` for an empty line). For the last
/// line without a trailing newline, `line_end == s.len()`.
fn line_bounds(s: &str, pos: usize) -> (usize, usize) {
    let start = s[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = match s[pos..].find('\n') {
        Some(off) => pos + off,
        None => s.len(),
    };
    (start, end)
}

fn column_of(s: &str, line_start: usize, pos: usize) -> usize {
    // Count chars (not bytes) between line_start and pos.
    s[line_start..pos].chars().count()
}

fn column_to_offset(s: &str, line_start: usize, line_end: usize, col: usize) -> usize {
    let line = &s[line_start..line_end];
    let mut byte = 0;
    for (i, c) in line.chars().enumerate() {
        if i == col {
            return line_start + byte;
        }
        byte += c.len_utf8();
    }
    line_end
}

fn next_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut p = pos + 1;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p
}

fn prev_char_boundary(s: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut p = pos - 1;
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharClass {
    Word,
    Punctuation,
    Whitespace,
    End,
}

fn char_class_at(bytes: &[u8], pos: usize) -> CharClass {
    if pos >= bytes.len() {
        return CharClass::End;
    }
    let b = bytes[pos];
    if b.is_ascii_whitespace() {
        CharClass::Whitespace
    } else if b.is_ascii_alphanumeric() || b == b'_' {
        CharClass::Word
    } else {
        // Non-ASCII bytes get bucketed as "Word" — good enough for the
        // initial vim feature set; punctuation handling is approximate.
        if !b.is_ascii() {
            CharClass::Word
        } else {
            CharClass::Punctuation
        }
    }
}
