use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use eframe::egui;

use crate::chat::history_picker::{self, SessionMeta};
use crate::chat::quote;
use crate::chat::session_store::SessionStore;
use crate::cli::CliOptions;
use crate::config::paths::AppPaths;
use crate::preview::loader::{self};
use crate::preview::render::{PreviewState, PreviewStatus};
use crate::preview::watcher::{self, FileWatchEvent, ProjectWatcher};
use crate::project::{self, ProjectInit};
use crate::tab::{ResumeSession, Tab, TabIdGen};
use crate::ui::session_picker::{self, SessionPickerAction};
use crate::ui::tab_bar::{self, TabBarAction, TabBarItem};
use uuid::Uuid;

pub struct App {
    /// Phase 9.5: workspace tabs. Each tab owns a chat session and
    /// the document state it watches. The active tab is the one the
    /// UI draws and the project-event drain routes to.
    tabs: Vec<Tab>,
    active_tab: usize,
    /// Mints `TabId`s for new-tab actions (Cmd+T and the `+` button
    /// on the tab bar).
    tab_id_gen: TabIdGen,
    /// `App` reuses the egui context for spawning new tabs (each
    /// tab needs `wake_ui` to repaint on async events). Stashed on
    /// `App::new` and cloned when a new `Tab` is constructed.
    ctx: egui::Context,

    /// Project-tree watcher (Phase 6.2). Recursive `notify` watcher
    /// rooted at `project_root`; events flow through
    /// `project_events_rx` to `drain_project_events`, which forwards
    /// them to the active tab's auto-follow / image-reload paths.
    _project_watcher: Option<ProjectWatcher>,
    project_events_rx: Option<Receiver<FileWatchEvent>>,

    /// Canonical project root from Phase 6.1's `project::resolve`.
    /// Held for the `Cmd+O` file dialog's initial directory (Phase
    /// 7.1) and as the fallback when the current preview has no
    /// usable parent directory.
    project_root: PathBuf,

    /// Phase 7.4: most recently published window title. Stored so
    /// the per-frame `update_window_title` only sends a
    /// `ViewportCommand::Title` when the computed title has
    /// actually changed.
    last_window_title: String,

    /// `--enable-dev-tools` runtime opt-in. The dev surface (currently
    /// only the `MDPILOT_DEBUG_SCREENSHOT` capture) only activates
    /// when this flag is set and the env var is present.
    debug_screenshot: Option<DebugScreenshot>,

    /// Phase 7.9: start of `App::new`, logged once at the first
    /// `ui()` call so a release-build run can be compared against
    /// the 3-second startup budget.
    startup_started: Option<Instant>,

    /// Phase 9.X "send selection to chat" state machine. Three
    /// frames are involved because egui's selected text is only
    /// reachable via the `Event::Copy` → `OutputCommand::CopyText`
    /// round-trip. See the variants for the per-frame transitions.
    chat_quote_state: ChatQuoteState,

    /// Screen-space anchor for the floating `→ チャットへ` bubble.
    /// Snapped to the pointer position on drag-release.
    chat_quote_anchor: Option<egui::Pos2>,

    /// True when a click outside the bubble dismissed it. Reset
    /// when `has_selection()` flips back to false, so the next
    /// drag-selection can show a fresh bubble.
    chat_quote_dismissed: bool,

    /// Screen rect occupied by the bubble on the previous frame, used
    /// to decide whether the next click landed inside the bubble
    /// (so we don't dismiss while the user is reaching for the button).
    chat_quote_bubble_rect: Option<egui::Rect>,

    /// Phase 9.X.1 F-11: path to `<data_dir>/sessions.json`. `None`
    /// when `AppPaths::resolve()` failed (no home dir), in which
    /// case session persistence is silently disabled.
    session_store_path: Option<PathBuf>,
    /// Phase 9.X.1 F-11: latch tracking whether we've already
    /// written the active session-id to disk this run. We persist
    /// once, only after claude's `system/init` event confirms the
    /// session is real (`Tab::session_confirmed`). Saving before
    /// confirmation would record ids claude never persisted (e.g.
    /// when mdpilot is closed before any messages exchange), and
    /// the next launch would try to resume a ghost session.
    session_persisted_this_run: bool,

    /// Phase 9.X.3: last preview path we wrote to `sessions.json`
    /// for each session-id. Used to detect changes and avoid
    /// writing the store every frame. `None` means "we persisted
    /// an empty preview for that session"; `Some(path)` mirrors the
    /// last-saved markdown source path.
    previews_persisted: std::collections::HashMap<Uuid, Option<PathBuf>>,

    /// Phase 9.X.2: state for the resume-picker modal. `None`
    /// when the modal is closed; `Some(_)` while it's open with
    /// a pre-loaded session list. The list is captured on open
    /// (cheap directory scan) and not refreshed per-frame.
    session_picker: Option<SessionPickerData>,
}

/// Phase 9.X.2: pre-loaded contents of the resume-picker modal.
/// Built when the user clicks 履歴; cleared on close or selection.
#[derive(Debug)]
struct SessionPickerData {
    sessions: Vec<SessionMeta>,
    error: Option<String>,
}

/// Per-frame state for routing a preview selection into the chat
/// input. Idle is the resting state; the bubble button transitions
/// to `PendingInject`. The next frame's `logic()` injects an
/// `Event::Copy` and moves to `AwaitingDrain`; the frame after
/// that drains the resulting `OutputCommand::CopyText` and
/// appends the formatted quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatQuoteState {
    Idle,
    PendingInject,
    AwaitingDrain,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cli: CliOptions, project: ProjectInit) -> Self {
        let startup_started = Instant::now();
        crate::ui::fonts::install_japanese(&cc.egui_ctx);

        let (project_watcher, project_events_rx, startup_watch_error) =
            match start_project_watcher(&cc.egui_ctx, project.root.clone()) {
                Ok((w, rx)) => (Some(w), Some(rx), None),
                Err(err) => {
                    tracing::warn!(
                        root = %project.root.display(),
                        error = %err,
                        "failed to start project watcher (auto-follow disabled)",
                    );
                    (
                        None,
                        None,
                        Some(format!("プロジェクト監視を開始できません: {err}")),
                    )
                }
            };

        let session_store_path = AppPaths::resolve().map(|p| p.data_dir.join("sessions.json"));
        // Read the store once to extract both the resume target and
        // (if found) that session's last preview path.
        let (resume, resume_preview_path) = session_store_path
            .as_ref()
            .map(|path| {
                let store = SessionStore::load_or_default(path);
                let Some(entry) = store.get(&project.root) else {
                    return (None, None);
                };
                let uuid = match Uuid::parse_str(&entry.session_id) {
                    Ok(uuid) => uuid,
                    Err(err) => {
                        tracing::warn!(
                            stored = %entry.session_id,
                            error = %err,
                            "stored session id is not a valid UUID; starting fresh",
                        );
                        return (None, None);
                    }
                };
                tracing::info!(
                    session_id = %uuid,
                    project = %project.root.display(),
                    "resuming previous claude session (F-11)",
                );
                let preview_path = store
                    .get_preview(&entry.session_id)
                    .map(|p| p.to_path_buf());
                (Some(ResumeSession { session_id: uuid }), preview_path)
            })
            .unwrap_or((None, None));

        let mut tab_id_gen = TabIdGen::default();
        let initial_preview = initial_preview_state(&project, resume_preview_path.as_deref());
        let mut initial_tab = Tab::new(
            &cc.egui_ctx,
            &project.root,
            initial_preview,
            tab_id_gen.next(),
            "タブ 1".to_string(),
            resume,
        );
        // Surface the project-watcher startup error on the initial
        // tab's banner if it has no error of its own (the tab's own
        // FileWatcher::start may also have failed).
        if let Some(message) = startup_watch_error {
            if initial_tab.watcher_error.is_none() {
                initial_tab.watcher_error = Some(message);
            }
        }

        Self {
            tabs: vec![initial_tab],
            active_tab: 0,
            tab_id_gen,
            ctx: cc.egui_ctx.clone(),
            _project_watcher: project_watcher,
            project_events_rx,
            project_root: project.root,
            last_window_title: String::new(),
            debug_screenshot: DebugScreenshot::from_env(cli),
            startup_started: Some(startup_started),
            chat_quote_state: ChatQuoteState::Idle,
            chat_quote_anchor: None,
            chat_quote_dismissed: false,
            chat_quote_bubble_rect: None,
            session_store_path,
            session_persisted_this_run: false,
            previews_persisted: std::collections::HashMap::new(),
            session_picker: None,
        }
    }

    /// Phase 9.X.2: scan claude's per-project session dir and
    /// open the picker modal. The scan is synchronous because
    /// `read_dir` + `read_to_string` on a typical session dir
    /// (dozens of files at most) finishes in microseconds; we
    /// don't need a background thread.
    ///
    /// Phase 9.X.3: results are filtered down to sessions for
    /// which mdpilot has recorded a preview path. Older sessions
    /// (created before mdpilot started persisting previews, or
    /// purely conversational sessions where the user never opened
    /// a document) would resume with an empty preview, which is
    /// confusing — better to omit them from the list entirely.
    fn open_session_picker(&mut self) {
        let known_session_ids: std::collections::HashSet<String> = self
            .session_store_path
            .as_ref()
            .map(|path| {
                SessionStore::load_or_default(path)
                    .session_previews
                    .keys()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let data = match directories::BaseDirs::new() {
            Some(base) => {
                let dir = history_picker::project_session_dir(base.home_dir(), &self.project_root);
                match history_picker::list_sessions(&dir) {
                    Ok(mut sessions) => {
                        sessions.retain(|s| known_session_ids.contains(&s.session_id.to_string()));
                        SessionPickerData {
                            sessions,
                            error: None,
                        }
                    }
                    Err(err) => SessionPickerData {
                        sessions: Vec::new(),
                        error: Some(err.to_string()),
                    },
                }
            }
            None => SessionPickerData {
                sessions: Vec::new(),
                error: Some("ホームディレクトリが取得できません".to_string()),
            },
        };
        self.session_picker = Some(data);
    }

    /// Phase 9.X.2: spawn a fresh tab that resumes `session_id`
    /// via `claude --resume <id>`. Mirrors `new_tab()` but with
    /// `ResumeSession` set instead of `None`. Used both by the
    /// picker modal (`SessionPickerAction::Resume`) and any
    /// future shortcut that resumes by id.
    fn open_tab_resuming(&mut self, session_id: Uuid) {
        let id = self.tab_id_gen.next();
        let short: String = session_id.to_string().chars().take(8).collect();
        let label = format!("再開 {short}");
        // Phase 9.X.3: restore whichever preview file was open when
        // mdpilot last persisted this session. The picker isn't
        // useful if the chat resumes but the document context is
        // lost. Falls back to empty when no preview was recorded.
        let initial_preview = self
            .session_store_path
            .as_ref()
            .and_then(|path| {
                let store = SessionStore::load_or_default(path);
                store
                    .get_preview(&session_id.to_string())
                    .map(|p| p.to_path_buf())
            })
            .map(|path| match loader::load_markdown(&path) {
                Ok(document) => PreviewState::loaded(document),
                Err(error) => {
                    tracing::warn!(
                        path = %path.display(),
                        ?error,
                        "failed to load resumed session's preview target",
                    );
                    let mut state = PreviewState::default();
                    state.set_error(path.to_string_lossy().into_owned(), error);
                    state
                }
            })
            .unwrap_or_default();
        let tab = Tab::new(
            &self.ctx,
            &self.project_root,
            initial_preview,
            id,
            label,
            Some(ResumeSession { session_id }),
        );
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        self.last_window_title.clear();
    }

    /// Phase 9.X.1: write the currently-active tab's session id to
    /// `<data_dir>/sessions.json`, keyed by project root, but only
    /// once claude has confirmed the session via `system/init`.
    /// Silent no-op when:
    ///
    /// - the store path could not be resolved (`AppPaths::resolve()`
    ///   returned `None`),
    /// - the active tab has no live session (claude spawn failed),
    /// - the session is not yet confirmed (Init not received),
    /// - we already saved this run (idempotent flag).
    ///
    /// Saving repeatedly with the same id is harmless (upsert is
    /// idempotent) but wastes IO, so the latch keeps it to one
    /// write per launch.
    fn maybe_persist_active_session(&mut self) {
        if self.session_persisted_this_run {
            return;
        }
        let Some(path) = self.session_store_path.as_ref() else {
            return;
        };
        let tab = &self.tabs[self.active_tab];
        if tab.session.is_none() || !tab.session_confirmed {
            return;
        }
        let mut store = SessionStore::load_or_default(path);
        store.upsert(
            &self.project_root,
            tab.session_id.to_string(),
            "unknown".to_string(),
        );
        if let Err(err) = store.save_atomic(path) {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "failed to save session store",
            );
            // Don't latch — retry next frame in case it was a
            // transient IO error.
            return;
        }
        tracing::info!(
            session_id = %tab.session_id,
            project = %self.project_root.display(),
            "persisted session-id (F-11)",
        );
        self.session_persisted_this_run = true;
    }

    /// Phase 9.X.3: write every confirmed tab's current preview
    /// path into the session store. Skips no-op writes by comparing
    /// against `previews_persisted` so the typical "preview didn't
    /// change" frame costs zero IO.
    fn maybe_persist_session_previews(&mut self) {
        let Some(path) = self.session_store_path.as_ref() else {
            return;
        };
        // Snapshot what we want to write without holding a borrow
        // into `self.tabs`, so the subsequent `previews_persisted`
        // update doesn't conflict with the borrow checker.
        let mut to_write: Vec<(Uuid, Option<PathBuf>)> = Vec::new();
        for tab in &self.tabs {
            if !tab.session_confirmed {
                continue;
            }
            let current = tab.current_preview_path().map(|p| p.to_path_buf());
            let cached = self.previews_persisted.get(&tab.session_id);
            if cached != Some(&current) {
                to_write.push((tab.session_id, current));
            }
        }
        if to_write.is_empty() {
            return;
        }
        let mut store = SessionStore::load_or_default(path);
        for (session_id, preview_path) in &to_write {
            store.set_preview(&session_id.to_string(), preview_path.as_deref());
        }
        if let Err(err) = store.save_atomic(path) {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "failed to save session preview paths",
            );
            // Don't update cache — retry next frame.
            return;
        }
        for (session_id, preview_path) in to_write {
            self.previews_persisted.insert(session_id, preview_path);
        }
    }

    /// Create a fresh tab with an empty preview and append it to
    /// `tabs`. The new tab becomes active. Per `docs/preview.md`
    /// §9.1.4: starting empty matches the "no positional arg, no
    /// README" startup path. Phase 9.X.1: `Cmd+T` always mints a
    /// new claude session — only the startup tab is eligible for
    /// `--continue` resume.
    fn new_tab(&mut self) {
        let id = self.tab_id_gen.next();
        let label = format!("タブ {}", id.raw());
        let tab = Tab::new(
            &self.ctx,
            &self.project_root,
            PreviewState::default(),
            id,
            label,
            None,
        );
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        // Reset the cached window title; the next frame will
        // recompute it for the new active tab and emit a fresh
        // ViewportCommand::Title.
        self.last_window_title.clear();
    }

    /// Close the tab at `idx`. Idempotent at boundaries: refuses
    /// to close the last remaining tab (mdpilot always has at
    /// least one workspace). Adjusts `active_tab` so that:
    ///
    /// - closing the active tab moves focus to its left neighbor
    ///   (or the new tab at index 0 if we closed index 0)
    /// - closing a non-active tab to the *left* of active
    ///   decrements active_tab by 1 (otherwise the index would
    ///   skip)
    fn close_tab(&mut self, idx: usize) {
        if self.tabs.len() <= 1 || idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        if idx < self.active_tab {
            self.active_tab -= 1;
        } else if idx == self.active_tab {
            self.active_tab = idx.min(self.tabs.len() - 1);
        }
        self.last_window_title.clear();
    }

    /// Switch focus to `idx`. No-op if the index is invalid.
    fn select_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() || idx == self.active_tab {
            return;
        }
        self.active_tab = idx;
        self.last_window_title.clear();
    }

    fn active(&self) -> &Tab {
        &self.tabs[self.active_tab]
    }

    fn active_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active_tab]
    }

    /// `docs/ui.md` §6.2: `Cmd+R` (mac) / `Ctrl+R` (Win/Linux) forces
    /// the active tab's preview to reload from disk.
    fn consume_reload_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::R);
        let pressed = ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        if pressed {
            self.active_mut().reload_current();
        }
        pressed
    }

    /// `docs/ui.md` §6.2: `Cmd+O` / `Ctrl+O` opens a Markdown
    /// picker. Selection replaces the active tab's preview and
    /// disables that tab's auto-follow (`docs/preview.md` §9.1.1).
    fn consume_open_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
        let pressed = ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        if !pressed {
            return false;
        }

        let start_dir = self.file_picker_start_dir();
        let picked = rfd::FileDialog::new()
            .add_filter("Markdown", &["md", "markdown"])
            .set_directory(&start_dir)
            .pick_file();
        let Some(path) = picked else {
            return true;
        };

        let label = path.to_string_lossy().into_owned();
        let tab = self.active_mut();
        match loader::load_markdown(&path) {
            Ok(document) => {
                tab.preview.set_document(document);
                tab.watcher_error = None;
            }
            Err(error) => {
                tracing::warn!(
                    path = %label,
                    ?error,
                    "Cmd+O target failed to load",
                );
                tab.preview.set_error(label, error);
            }
        }
        tab.pending_reload = None;
        tab.pending_follow = None;
        tab.auto_follow_enabled = false;
        tab.sync_watch_target();
        true
    }

    /// `docs/ui.md` §6.2: `Cmd+\` / `Ctrl+\` resets the pane split
    /// to 50/50. This is app-wide (not per-tab) because the pane
    /// layout itself is shared across tabs.
    fn consume_pane_reset_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Backslash);
        let pressed = ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        if pressed {
            crate::ui::layout::reset(ctx);
        }
        pressed
    }

    /// Phase 9.5.3: `Cmd+T` / `Ctrl+T` opens a new tab (empty
    /// preview + fresh claude session). Mirrors the `+` button
    /// in the tab bar.
    fn consume_new_tab_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::T);
        let pressed = ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        if pressed {
            self.new_tab();
        }
        pressed
    }

    /// Phase 9.5.3: `Cmd+W` / `Ctrl+W` closes the active tab.
    /// Refuses to close the last remaining tab (mdpilot always
    /// has at least one workspace). The `close_tab` method
    /// already enforces this.
    fn consume_close_tab_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::W);
        let pressed = ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        if pressed {
            self.close_tab(self.active_tab);
        }
        pressed
    }

    /// Phase 9.5.3: `Cmd+1..9` / `Ctrl+1..9` switches to the N-th
    /// tab. Out-of-range indices are silently ignored (the user
    /// is asking for a tab that isn't there).
    fn consume_tab_switch_shortcuts(&mut self, ctx: &egui::Context) {
        // Digit keys 1..9 map to tab indices 0..8. The naming
        // mismatch (1-based UX, 0-based array) is the standard
        // editor convention (VS Code, browsers, etc.).
        const DIGITS: &[egui::Key] = &[
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        for (idx, key) in DIGITS.iter().enumerate() {
            let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, *key);
            if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                self.select_tab(idx);
            }
        }
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let new_title = compute_window_title(&self.active().preview.status);
        if new_title == self.last_window_title {
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(new_title.clone()));
        self.last_window_title = new_title;
    }

    fn file_picker_start_dir(&self) -> PathBuf {
        match &self.active().preview.status {
            PreviewStatus::Loaded { document, .. } => document
                .path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| self.project_root.clone()),
            _ => self.project_root.clone(),
        }
    }

    /// Phase 6.3: drain project-tree watch events and arm the active
    /// tab's auto-follow timer / image cache invalidation. With
    /// multi-tab (Phase 9.5) we route events to the *active* tab
    /// only — non-active tabs' auto-follow is effectively paused
    /// while they're in the background.
    fn drain_project_events(&mut self, ctx: &egui::Context) {
        let events = self.collect_project_events();
        if events.is_empty() {
            return;
        }
        let tab = self.active_mut();
        let current_path = match &tab.preview.status {
            PreviewStatus::Loaded { document, .. } => Some(document.path.clone()),
            PreviewStatus::Failed { path_label, .. } => Some(PathBuf::from(path_label)),
            PreviewStatus::Empty => None,
        };
        for event in events {
            match event {
                FileWatchEvent::Changed { path } => {
                    let is_current = current_path
                        .as_deref()
                        .map(|c| watcher::paths_match(&path, c))
                        .unwrap_or(false);
                    if is_current {
                        continue;
                    }
                    if !tab.auto_follow_enabled {
                        continue;
                    }
                    tab.pending_follow = Some((
                        path,
                        Instant::now() + crate::preview::watcher::FOLLOW_DEBOUNCE,
                    ));
                    ctx.request_repaint_after(crate::preview::watcher::FOLLOW_DEBOUNCE);
                }
                FileWatchEvent::Removed { path } => {
                    if let Some((pending_path, _)) = tab.pending_follow.as_ref() {
                        if watcher::paths_match(&path, pending_path) {
                            tab.pending_follow = None;
                        }
                    }
                }
                FileWatchEvent::Error(message) => {
                    tracing::warn!(
                        target: "mdpilot::project_watch",
                        message = %message,
                        "project watcher error",
                    );
                    tab.watcher_error = Some(format!("プロジェクト監視エラー: {message}"));
                }
            }
        }
    }

    /// Phase 9.X: state-machine advance for the
    /// `chat_quote_state`. Called once at the top of `logic()`,
    /// before any other input draining. Three states:
    ///
    /// - `Idle` → no action.
    /// - `PendingInject` → push an `Event::Copy` into this frame's
    ///   input events (label processing later in this frame will
    ///   see it and accumulate `text_to_copy`), then advance to
    ///   `AwaitingDrain`.
    /// - `AwaitingDrain` → drain `OutputCommand::CopyText` from
    ///   the *previous* frame's output (which is still sitting in
    ///   ctx.output_mut at this point because `logic()` runs
    ///   before eframe consumes the output for clipboard
    ///   delivery). Append the formatted quote to the active tab's
    ///   chat input and return to `Idle`.
    fn advance_chat_quote_state(&mut self, ctx: &egui::Context) {
        match self.chat_quote_state {
            ChatQuoteState::Idle => {}
            ChatQuoteState::PendingInject => {
                ctx.input_mut(|i| i.events.push(egui::Event::Copy));
                self.chat_quote_state = ChatQuoteState::AwaitingDrain;
                // Repaint so the on_end_pass that emits CopyText
                // runs even if there is no other input activity.
                ctx.request_repaint();
            }
            ChatQuoteState::AwaitingDrain => {
                let copied = ctx.output_mut(|o| {
                    let mut grabbed: Option<String> = None;
                    o.commands.retain(|cmd| match cmd {
                        egui::OutputCommand::CopyText(text) if grabbed.is_none() => {
                            grabbed = Some(text.clone());
                            // Keep the CopyText in the queue so the
                            // OS clipboard still gets it — the user
                            // expects a "→チャット" click to *also*
                            // leave the selection on the clipboard,
                            // matching the Cmd+C side-effect.
                            true
                        }
                        _ => true,
                    });
                    grabbed
                });
                if let Some(text) = copied {
                    self.append_quote_to_active_tab(&text);
                }
                self.chat_quote_state = ChatQuoteState::Idle;
            }
        }
    }

    fn append_quote_to_active_tab(&mut self, selection: &str) {
        if selection.is_empty() {
            return;
        }
        let (source, filename) = match &self.active().preview.status {
            PreviewStatus::Loaded { document, .. } => (
                Some(document.text.as_str()),
                document.path.file_name().and_then(|n| n.to_str()),
            ),
            PreviewStatus::Failed { path_label, .. } => (
                None,
                std::path::Path::new(path_label)
                    .file_name()
                    .and_then(|n| n.to_str()),
            ),
            PreviewStatus::Empty => (None, None),
        };
        let block = quote::format_quote_block(selection, source, filename);
        if block.is_empty() {
            return;
        }
        let input = &mut self.active_mut().chat.input;
        // Drop a leading blank line only when the input already has
        // content; otherwise the quote sits flush at the top.
        if !input.is_empty() && !input.ends_with('\n') {
            input.push('\n');
        }
        input.push_str(&block);
    }

    fn collect_project_events(&mut self) -> Vec<FileWatchEvent> {
        let mut out = Vec::new();
        let Some(rx) = self.project_events_rx.as_ref() else {
            return out;
        };
        loop {
            match rx.try_recv() {
                Ok(event) => out.push(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.project_events_rx = None;
                    break;
                }
            }
        }
        out
    }
}

fn initial_preview_state(project: &ProjectInit, resume_preview: Option<&Path>) -> PreviewState {
    // Priority: explicit positional arg → resume's last preview →
    // project README → empty. The resume-preview path slots in
    // *after* the explicit arg so `mdpilot foo.md` always wins over
    // whatever the previous session was looking at.
    let path =
        project::initial_preview(project).or_else(|| resume_preview.map(|p| p.to_path_buf()));
    let Some(path) = path else {
        return PreviewState::default();
    };
    match loader::load_markdown(&path) {
        Ok(document) => PreviewState::loaded(document),
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                ?error,
                "failed to load initial preview target",
            );
            let mut state = PreviewState::default();
            state.set_error(path.to_string_lossy().into_owned(), error);
            state
        }
    }
}

fn start_project_watcher(
    ctx: &egui::Context,
    root: PathBuf,
) -> notify::Result<(ProjectWatcher, Receiver<FileWatchEvent>)> {
    let (tx, rx) = mpsc::channel::<FileWatchEvent>();
    let wake_ctx = ctx.clone();
    let watcher = ProjectWatcher::start(root, tx, move || wake_ctx.request_repaint())?;
    Ok((watcher, rx))
}

/// Pure helper for `App::update_window_title`. Format:
/// `mdpilot - <basename>` while a file is loaded (or failed to load),
/// plain `mdpilot` for the empty placeholder.
fn compute_window_title(status: &PreviewStatus) -> String {
    const BASE: &str = "mdpilot";
    let basename = match status {
        PreviewStatus::Loaded { document, .. } => document
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string),
        PreviewStatus::Failed { path_label, .. } => std::path::Path::new(path_label)
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string),
        PreviewStatus::Empty => None,
    };
    match basename {
        Some(name) => format!("{BASE} - {name}"),
        None => BASE.to_string(),
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Advance the chat-quote state machine *first* so any
        // injected Event::Copy lands in this frame's input events
        // before label processing runs in ui().
        self.advance_chat_quote_state(ctx);
        // Per-tab event drain. Background tabs still drain their
        // chat / file-watcher events so history accumulates while
        // they're inactive — only the project-watcher dispatch is
        // routed exclusively to the active tab.
        for tab in &mut self.tabs {
            tab.drain_chat_events();
            tab.drain_watch_events(ctx);
        }
        self.drain_project_events(ctx);
        // Pending timers and shortcuts only act on the active tab.
        self.active_mut().poll_pending_reload(ctx);
        self.active_mut().poll_pending_follow(ctx);
        self.consume_reload_shortcut(ctx);
        self.consume_open_shortcut(ctx);
        self.consume_pane_reset_shortcut(ctx);
        self.consume_new_tab_shortcut(ctx);
        self.consume_close_tab_shortcut(ctx);
        self.consume_tab_switch_shortcuts(ctx);
        self.maybe_persist_active_session();
        self.maybe_persist_session_previews();
        self.update_window_title(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(start) = self.startup_started.take() {
            let elapsed = start.elapsed();
            tracing::info!(
                target: "mdpilot::perf",
                elapsed_ms = elapsed.as_millis() as u64,
                "first frame rendered (N-01)",
            );
        }

        // Phase 9.5.2 tab bar at the very top. The action drives
        // tab mutations *after* the borrow on `self.tabs` ends,
        // matching the send_text / follow_toggled pattern used
        // below.
        let tab_action = {
            let items: Vec<TabBarItem> = self
                .tabs
                .iter()
                .enumerate()
                .map(|(idx, tab)| TabBarItem {
                    label: &tab.label,
                    is_active: idx == self.active_tab,
                })
                .collect();
            egui::Panel::top("tab_bar")
                .show_inside(ui, |ui| tab_bar::show(ui, &items))
                .inner
        };
        match tab_action {
            TabBarAction::None => {}
            TabBarAction::Select(idx) => self.select_tab(idx),
            TabBarAction::Close(idx) => self.close_tab(idx),
            TabBarAction::NewTab => self.new_tab(),
            TabBarAction::OpenHistory => self.open_session_picker(),
        }

        let active_idx = self.active_tab;
        let tab = &mut self.tabs[active_idx];
        let session_alive = tab.session.is_some();
        let mut send_text: Option<String> = None;
        let mut follow_toggled = false;

        {
            let mut on_toggle_follow = || {
                follow_toggled = true;
            };
            egui::Panel::top("path_bar").show_inside(ui, |ui| {
                crate::ui::path_bar::show(
                    ui,
                    &tab.preview,
                    tab.auto_follow_enabled,
                    &mut on_toggle_follow,
                    tab.watcher_error.as_deref(),
                    session_alive,
                );
            });
        }
        if follow_toggled {
            tab.auto_follow_enabled = !tab.auto_follow_enabled;
            tracing::info!(
                enabled = tab.auto_follow_enabled,
                "auto-follow toggled via path bar",
            );
        }

        {
            let mut on_send = |text: String| {
                send_text = Some(text);
            };
            crate::ui::layout::show(
                ui,
                &mut tab.chat,
                &mut tab.preview,
                session_alive,
                &mut on_send,
            );
        }

        if let Some(text) = send_text {
            tab.handle_send(text);
        }

        self.show_chat_quote_bubble(ui.ctx());
        self.show_session_picker(ui.ctx());

        if let Some(cap) = self.debug_screenshot.as_mut() {
            cap.step(ui.ctx());
        }
    }
}

impl App {
    /// Phase 9.X.2: render the resume-picker modal when
    /// `session_picker` is `Some`. Closes on the modal's X
    /// button or after the user picks a session (which also
    /// opens a new tab via `open_tab_resuming`).
    fn show_session_picker(&mut self, ctx: &egui::Context) {
        let Some(data) = self.session_picker.as_ref() else {
            return;
        };
        let action = session_picker::show(ctx, &data.sessions, data.error.as_deref());
        match action {
            SessionPickerAction::None => {}
            SessionPickerAction::Close => {
                self.session_picker = None;
            }
            SessionPickerAction::Resume(session_id) => {
                self.session_picker = None;
                self.open_tab_resuming(session_id);
            }
        }
    }

    /// Phase 9.X: render the floating "→チャット" button when
    /// egui's `LabelSelectionState` reports a live selection and
    /// we're not already mid-flight on a previous request. The
    /// `Area` anchors to the latest pointer position; egui's
    /// `LabelSelectionState` keeps the selection bbox private, so
    /// this is the closest visible anchor we can reach without
    /// forking the plugin.
    fn show_chat_quote_bubble(&mut self, ctx: &egui::Context) {
        if self.chat_quote_state != ChatQuoteState::Idle {
            return;
        }
        let has_selection = ctx
            .plugin::<egui::text_selection::LabelSelectionState>()
            .lock()
            .has_selection();
        if !has_selection {
            self.chat_quote_anchor = None;
            self.chat_quote_dismissed = false;
            self.chat_quote_bubble_rect = None;
            return;
        }

        // Distinguish drag-release (range selection just finished)
        // from a plain click (no drag). egui's
        // `is_decidedly_dragging()` is true on the same frame the
        // drag ends; `any_click()` is true on the same frame for a
        // tap with no drag. They are mutually exclusive at release
        // time (input_state.rs:1511).
        let released_drag =
            ctx.input(|i| i.pointer.any_released() && i.pointer.is_decidedly_dragging());
        let clicked = ctx.input(|i| i.pointer.any_click());
        let interact_pos = ctx.input(|i| i.pointer.interact_pos());

        if released_drag {
            if let Some(pos) = interact_pos {
                self.chat_quote_anchor = Some(pos + egui::vec2(8.0, 12.0));
                self.chat_quote_dismissed = false;
            }
        } else if clicked {
            let inside_bubble = match (interact_pos, self.chat_quote_bubble_rect) {
                (Some(pos), Some(rect)) => rect.contains(pos),
                _ => false,
            };
            if !inside_bubble {
                self.chat_quote_dismissed = true;
                self.chat_quote_anchor = None;
                self.chat_quote_bubble_rect = None;
            }
        }

        if self.chat_quote_dismissed {
            return;
        }
        let Some(anchor) = self.chat_quote_anchor else {
            return;
        };
        let area_response = egui::Area::new(egui::Id::new("chat_quote_bubble"))
            .order(egui::Order::Foreground)
            .fixed_pos(anchor)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| {
                        ui.small_button("→ チャットへ")
                            .on_hover_text("選択範囲を出典付きでチャット入力欄に追記")
                            .clicked()
                    })
                    .inner
            });
        self.chat_quote_bubble_rect = Some(area_response.response.rect);
        if area_response.inner {
            self.chat_quote_state = ChatQuoteState::PendingInject;
        }
    }
}

/// One-shot screenshot helper, gated at runtime behind the
/// `--enable-dev-tools` CLI flag.
struct DebugScreenshot {
    path: String,
    frame_count: u32,
    requested: bool,
    closed: bool,
}

impl DebugScreenshot {
    fn from_env(cli: CliOptions) -> Option<Self> {
        if !cli.enable_dev_tools {
            return None;
        }
        std::env::var("MDPILOT_DEBUG_SCREENSHOT")
            .ok()
            .map(|path| Self {
                path,
                frame_count: 0,
                requested: false,
                closed: false,
            })
    }

    fn step(&mut self, ctx: &egui::Context) {
        if self.closed {
            return;
        }
        self.frame_count += 1;

        if !self.requested && self.frame_count >= 30 {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }

        if self.requested {
            let mut grabbed: Option<std::sync::Arc<egui::ColorImage>> = None;
            ctx.input(|i| {
                for event in &i.raw.events {
                    if let egui::Event::Screenshot { image, .. } = event {
                        grabbed = Some(image.clone());
                    }
                }
            });
            if let Some(image) = grabbed {
                let w = image.width() as u32;
                let h = image.height() as u32;
                let mut bytes = Vec::with_capacity(image.pixels.len() * 4);
                for c in image.pixels.iter() {
                    bytes.extend_from_slice(&c.to_array());
                }
                let buf = image::RgbaImage::from_raw(w, h, bytes)
                    .expect("debug screenshot: color image size mismatch");
                buf.save(&self.path)
                    .expect("debug screenshot: png save failed");
                eprintln!("debug screenshot saved to {}", &self.path);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                self.closed = true;
                return;
            }
        }

        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::loader::{LoadError, LoadedDocument, SizeClass};
    use std::path::PathBuf;

    fn loaded(path: &str) -> PreviewStatus {
        PreviewStatus::Loaded {
            document: LoadedDocument {
                path: PathBuf::from(path),
                text: String::new(),
                size_bytes: 0,
                size_class: SizeClass::Small,
            },
        }
    }

    #[test]
    fn empty_preview_uses_bare_title() {
        assert_eq!(compute_window_title(&PreviewStatus::Empty), "mdpilot");
    }

    #[test]
    fn loaded_preview_appends_basename() {
        assert_eq!(
            compute_window_title(&loaded("/Users/u/proj/README.md")),
            "mdpilot - README.md",
        );
    }

    #[test]
    fn failed_preview_uses_basename_from_label() {
        let status = PreviewStatus::Failed {
            path_label: "/Users/u/proj/missing.md".to_string(),
            error: LoadError::NotFound,
        };
        assert_eq!(compute_window_title(&status), "mdpilot - missing.md");
    }

    #[test]
    fn loaded_preview_with_no_filename_falls_back_to_bare() {
        let status = loaded("/");
        assert_eq!(compute_window_title(&status), "mdpilot");
    }
}
