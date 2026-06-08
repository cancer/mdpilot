//! Read-only markdown source view for the preview pane.
//!
//! mdpilot used to render markdown via `egui_commonmark`. That whole
//! path was retired (2026-06-05 ユーザー判断): markdown プレビューは
//! omit、validation も実施せず、左ペインは syntect でハイライトされた
//! markdown ソース + 行番号の read-only 表示に置き換えた。
//!
//! Highlighting is done line-by-line with `syntect`'s bundled
//! `markdown.sublime-syntax` against `base16-ocean.dark` /
//! `InspiredGitHub`. The rendered output is a single `LayoutJob` so
//! egui's selection plugin can copy across lines naturally — the
//! trade-off is that copied text includes the line-number gutter; see
//! `format_gutter`.

#![allow(dead_code)]

use std::sync::OnceLock;

use eframe::egui;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

use crate::preview::loader::{LoadError, LoadedDocument, SizeClass, SOFT_LIMIT_BYTES};
use crate::vim::{Mode as VimMode, VimEngine};

/// Phase 10.2: editor state attached to every `Loaded` document.
/// Holds the vim engine that drives modal editing of `buffer()`.
#[derive(Debug)]
pub struct EditorState {
    pub vim: VimEngine,
}

impl EditorState {
    pub fn from_document(doc: &LoadedDocument) -> Self {
        Self {
            vim: VimEngine::new(doc.text.clone()),
        }
    }

    pub fn mode(&self) -> VimMode {
        self.vim.mode()
    }
}

/// Theme name used in dark mode. Bundled by `default-themes`.
const SYNTAX_THEME_DARK: &str = "base16-ocean.dark";

/// Theme name used in light mode. Bundled by `default-themes`.
const SYNTAX_THEME_LIGHT: &str = "InspiredGitHub";

/// Font size for the source view. Markdown source is monospace-only.
const SOURCE_FONT_SIZE: f32 = 13.0;

/// State held by `App` for the preview pane.
pub struct PreviewState {
    pub status: PreviewStatus,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            status: PreviewStatus::Empty,
        }
    }
}

impl PreviewState {
    pub fn loaded(document: LoadedDocument) -> Self {
        let editor = Box::new(EditorState::from_document(&document));
        Self {
            status: PreviewStatus::Loaded { document, editor },
        }
    }

    pub fn set_document(&mut self, document: LoadedDocument) {
        let editor = Box::new(EditorState::from_document(&document));
        self.status = PreviewStatus::Loaded { document, editor };
    }

    pub fn set_error(&mut self, path_label: String, error: LoadError) {
        self.status = PreviewStatus::Failed { path_label, error };
    }

    pub fn clear(&mut self) {
        self.status = PreviewStatus::Empty;
    }

    /// Convenience: current active vim mode (if a document is loaded).
    pub fn vim_mode(&self) -> Option<VimMode> {
        match &self.status {
            PreviewStatus::Loaded { editor, .. } => Some(editor.mode()),
            _ => None,
        }
    }
}

/// Three-way state of the preview pane.
#[derive(Debug)]
pub enum PreviewStatus {
    Empty,
    Loaded {
        document: LoadedDocument,
        /// Boxed because `EditorState` carries the vim engine's
        /// buffer + search state, which is much heavier than the
        /// other variants — boxing keeps the enum size symmetric.
        editor: Box<EditorState>,
    },
    Failed {
        path_label: String,
        error: LoadError,
    },
}

pub fn show(ui: &mut egui::Ui, state: &mut PreviewState) {
    match &mut state.status {
        PreviewStatus::Empty => {
            ui.centered_and_justified(|ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new("プレビュー未指定").weak())
                        .selectable(false),
                );
            });
        }
        PreviewStatus::Failed { path_label, error } => {
            show_error(ui, path_label, error);
        }
        PreviewStatus::Loaded { document, editor } => {
            if document.size_class == SizeClass::Large {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(size_warning(document))
                            .color(egui::Color32::from_rgb(220, 180, 70)),
                    )
                    .selectable(false),
                );
                ui.separator();
            }
            let dark_mode = ui.style().visuals.dark_mode;
            let theme_name = if dark_mode {
                SYNTAX_THEME_DARK
            } else {
                SYNTAX_THEME_LIGHT
            };
            // Source-of-truth for the text is the vim engine buffer.
            // The LoadedDocument.text stays untouched as the
            // "originally loaded from disk" reference; keystroke save
            // (Phase 10.4) will sync them.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    show_source_grid(ui, editor.vim.buffer(), theme_name, Some(editor));
                });
            // Phase 10.6: search prompt strip lives at the bottom
            // of the pane while `/` input is active. We render it
            // *after* the source view so it visually anchors there.
            if let Some(query) = editor.vim.search_prompt() {
                ui.separator();
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("/{query}"))
                            .monospace()
                            .color(egui::Color32::from_rgb(220, 180, 70)),
                    )
                    .selectable(false),
                );
            } else if !editor.vim.search_matches().is_empty() {
                ui.separator();
                let total = editor.vim.search_matches().len();
                let current = editor.vim.current_match().map(|i| i + 1).unwrap_or(0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("検索: {current}/{total}"))
                            .monospace()
                            .weak(),
                    )
                    .selectable(false),
                );
            }
        }
    }
}

fn show_error(ui: &mut egui::Ui, path_label: &str, error: &LoadError) {
    ui.vertical(|ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(error_headline(error))
                    .color(egui::Color32::from_rgb(220, 90, 80))
                    .strong(),
            )
            .selectable(true),
        );
        ui.add(egui::Label::new(egui::RichText::new(path_label).weak()).selectable(true));
        if let Some(detail) = error_detail(error) {
            ui.add_space(4.0);
            ui.add(egui::Label::new(detail).selectable(true));
        }
    });
}

pub(crate) fn error_headline(error: &LoadError) -> &'static str {
    match error {
        LoadError::NotFound => "ファイルが見つかりません",
        LoadError::PermissionDenied => "ファイルを読み取れません（権限不足）",
        LoadError::NotUtf8 => "UTF-8 として読めません",
        LoadError::TooLarge { .. } => "ファイルが大きすぎるため表示できません",
        LoadError::Io(_) => "ファイルを開けませんでした",
    }
}

pub(crate) fn error_detail(error: &LoadError) -> Option<String> {
    match error {
        LoadError::TooLarge { size_bytes } => Some(format!(
            "{:.1} MiB > 10 MiB の上限（{} bytes）",
            *size_bytes as f64 / (1024.0 * 1024.0),
            size_bytes
        )),
        LoadError::Io(message) => Some(message.clone()),
        _ => None,
    }
}

pub(crate) fn size_warning(document: &LoadedDocument) -> String {
    format!(
        "ファイルサイズが {:.2} MiB（{} 以上）と大きいため、初回描画で\
         一瞬応答が遅れる可能性があります。",
        document.size_bytes as f64 / (1024.0 * 1024.0),
        format_mib(SOFT_LIMIT_BYTES),
    )
}

fn format_mib(bytes: u64) -> String {
    let mib = bytes as f64 / (1024.0 * 1024.0);
    if mib.fract() == 0.0 {
        format!("{:.0} MiB", mib)
    } else {
        format!("{:.2} MiB", mib)
    }
}

/// Cached `SyntaxSet`/`ThemeSet` so we don't reload them every frame.
/// syntect's defaults load from in-process binary blobs (no I/O), but
/// they still allocate; keep them behind `OnceLock`.
fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    static SET: OnceLock<ThemeSet> = OnceLock::new();
    SET.get_or_init(ThemeSet::load_defaults)
}

fn markdown_syntax() -> &'static SyntaxReference {
    let set = syntax_set();
    // Bundled syntax names live in `set.find_syntax_by_name`; the
    // markdown sublime-syntax registers as "Markdown".
    set.find_syntax_by_name("Markdown")
        .or_else(|| set.find_syntax_by_extension("md"))
        .unwrap_or_else(|| set.find_syntax_plain_text())
}

fn theme_for(name: &str) -> &'static Theme {
    let set = theme_set();
    set.themes
        .get(name)
        .or_else(|| set.themes.values().next())
        .expect("syntect default-themes ships with at least one theme")
}

/// Render the markdown source as a 2-column grid: a fixed-width
/// gutter (line numbers, non-selectable) on the left and a syntect-
/// highlighted body Label (selectable, wrap-enabled) on the right.
///
/// The split into two Labels per row is what keeps a long, wrapped
/// body line from crashing into the gutter column — a single
/// `LayoutJob` cannot express "hanging indent" because epaint's
/// `leading_space` only applies to the *first* row of a section.
/// Using `Grid` lets each row's height be set by the body's wrapped
/// height while the gutter cell stays at its natural single-row
/// height (top-aligned).
fn show_source_grid(
    ui: &mut egui::Ui,
    source: &str,
    theme_name: &str,
    editor: Option<&EditorState>,
) {
    let syntax = markdown_syntax();
    let theme = theme_for(theme_name);
    let mut highlighter = HighlightLines::new(syntax, theme);
    let set = syntax_set();

    let total_lines = source.lines().count().max(1);
    let gutter_width = total_lines.to_string().len();

    let gutter_color = egui::Color32::from_gray(120);
    let fallback_body_color = if theme_name == SYNTAX_THEME_DARK {
        egui::Color32::from_gray(220)
    } else {
        egui::Color32::from_gray(40)
    };

    let separator_color = egui::Color32::from_gray(60);
    let separator_stroke = egui::Stroke::new(1.0, separator_color);
    let row_spacing_y: f32 = 0.0;
    let separator_pad: f32 = 6.0;

    // Pre-compute the source line that holds the cursor (and where
    // within that line, byte-wise) so we can paint a cursor marker
    // on the matching row below.
    let cursor_pos = editor.map(|ed| ed.vim.cursor());
    let cursor_line = cursor_pos.map(|p| line_index_of(source, p));
    // Phase 10.6 (cursor): char-precise cursor. monospace font means
    // glyph width is uniform — multiply by the line-local column to
    // get x-offset from the body's left edge. CJK is wider in real
    // fonts but stays close enough for the bar to read as "the
    // cursor is around here".
    let mono_font = egui::FontId::monospace(SOURCE_FONT_SIZE);
    let char_width = ui.ctx().fonts_mut(|f| f.glyph_width(&mono_font, ' '));
    // Phase 10.6 (vis): match-highlight background. Mid-amber that
    // reads on both syntect themes.
    let match_bg = egui::Color32::from_rgba_unmultiplied(220, 180, 70, 90);
    let active_match_bg = egui::Color32::from_rgba_unmultiplied(255, 140, 60, 140);
    let empty_matches: &[std::ops::Range<usize>] = &[];
    let (matches, active_match_idx) = editor
        .map(|ed| (ed.vim.search_matches(), ed.vim.current_match()))
        .unwrap_or((empty_matches, None));

    egui::Grid::new("preview_source_grid")
        .num_columns(2)
        .spacing(egui::vec2(separator_pad * 2.0, row_spacing_y))
        .show(ui, |ui| {
            let mut line_start_in_buffer: usize = 0;
            for (idx, raw_line) in LinesWithEndings::from(source).enumerate() {
                let line_len = raw_line.len();
                let line_range = line_start_in_buffer..(line_start_in_buffer + line_len);
                let line_no = idx + 1;
                let gutter_text = format_gutter(line_no, gutter_width);
                let gutter_resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new(gutter_text)
                            .color(gutter_color)
                            .monospace()
                            .size(SOURCE_FONT_SIZE),
                    )
                    .selectable(false),
                );

                let sep_x = gutter_resp.rect.right() + separator_pad;
                let sep_top = gutter_resp.rect.top();
                let sep_bottom = gutter_resp.rect.bottom() + row_spacing_y;
                ui.painter().line_segment(
                    [egui::pos2(sep_x, sep_top), egui::pos2(sep_x, sep_bottom)],
                    separator_stroke,
                );

                let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
                // Phase 10.6 (vis): collect every search-match range
                // that overlaps this line so the segment splitter
                // below can inject a background-tinted section for
                // each one. `active` marks the currently-highlighted
                // match (n/N target) with a stronger color.
                let line_match_ranges: Vec<(std::ops::Range<usize>, bool)> = matches
                    .iter()
                    .enumerate()
                    .filter_map(|(i, m)| {
                        let lo = m.start.max(line_range.start);
                        let hi = m.end.min(line_range.end);
                        if lo >= hi {
                            return None;
                        }
                        let is_active = Some(i) == active_match_idx;
                        Some(((lo - line_range.start)..(hi - line_range.start), is_active))
                    })
                    .collect();

                let mut body_job = egui::text::LayoutJob::default();
                match highlighter.highlight_line(raw_line, set) {
                    Ok(ranges) => {
                        let mut piece_start = 0usize;
                        for (style, piece) in ranges {
                            let trimmed = piece.strip_suffix('\n').unwrap_or(piece);
                            if !trimmed.is_empty() {
                                let color = syntect_to_egui(style.foreground);
                                append_with_matches(
                                    &mut body_job,
                                    trimmed,
                                    piece_start,
                                    color,
                                    style.font_style,
                                    &line_match_ranges,
                                    match_bg,
                                    active_match_bg,
                                );
                            }
                            piece_start += piece.len();
                        }
                    }
                    Err(_) => {
                        append_with_matches(
                            &mut body_job,
                            line,
                            0,
                            fallback_body_color,
                            FontStyle::empty(),
                            &line_match_ranges,
                            match_bg,
                            active_match_bg,
                        );
                    }
                }
                if body_job.sections.is_empty() {
                    append(&mut body_job, " ", fallback_body_color, FontStyle::empty());
                }
                let body_resp = ui.add(egui::Label::new(body_job).selectable(true).wrap());

                // Cursor row indicator + char-precise cursor bar.
                // Char-precise is required to see horizontal motion
                // (h/l/0/$/w/b/e); monospace lets us compute column ×
                // glyph-width without poking at the Galley.
                if Some(idx) == cursor_line {
                    if let Some(ed) = editor {
                        let cursor_color = match ed.mode() {
                            VimMode::Normal => egui::Color32::from_rgb(80, 160, 220),
                            VimMode::Insert => egui::Color32::from_rgb(80, 200, 100),
                            VimMode::Visual => egui::Color32::from_rgb(220, 180, 70),
                        };
                        let bar_rect = egui::Rect::from_min_max(
                            egui::pos2(gutter_resp.rect.left() - 4.0, gutter_resp.rect.top()),
                            egui::pos2(gutter_resp.rect.left() - 1.0, gutter_resp.rect.bottom()),
                        );
                        ui.painter().rect_filled(bar_rect, 0.0, cursor_color);
                        let row_tint = cursor_color.linear_multiply(0.16);
                        ui.painter().rect_filled(body_resp.rect, 0.0, row_tint);
                        if let Some(cur) = cursor_pos {
                            let line_local_byte = cur.saturating_sub(line_range.start);
                            let col = line[..line_local_byte.min(line.len())].chars().count();
                            let cx = body_resp.rect.left() + col as f32 * char_width;
                            let cursor_bar = egui::Rect::from_min_max(
                                egui::pos2(cx, body_resp.rect.top()),
                                egui::pos2(cx + 2.0, body_resp.rect.bottom()),
                            );
                            ui.painter().rect_filled(cursor_bar, 0.0, cursor_color);
                        }
                    }
                }

                ui.end_row();
                line_start_in_buffer += line_len;
            }
        });
}

/// Phase 10.6 (vis): append `text` to `job` while splitting it into
/// segments that intersect with the search-match ranges. Match
/// segments get a `background` color (active match wins over normal
/// match if both apply to the same offset).
#[allow(clippy::too_many_arguments)]
fn append_with_matches(
    job: &mut egui::text::LayoutJob,
    text: &str,
    text_start_in_line: usize,
    color: egui::Color32,
    style: FontStyle,
    line_matches: &[(std::ops::Range<usize>, bool)],
    match_bg: egui::Color32,
    active_match_bg: egui::Color32,
) {
    if line_matches.is_empty() {
        append(job, text, color, style);
        return;
    }
    // Walk the text and emit alternating non-match / match runs.
    // We compare positions in line-local byte space; conversion to
    // text-local is straightforward via `text_start_in_line`.
    let text_end = text_start_in_line + text.len();
    let mut cursor = text_start_in_line;
    // Sort matches by start so we can sweep left-to-right; matches
    // shouldn't overlap, so simple iteration is enough.
    let mut sorted: Vec<&(std::ops::Range<usize>, bool)> = line_matches.iter().collect();
    sorted.sort_by_key(|(r, _)| r.start);
    for (range, is_active) in sorted {
        if range.end <= cursor || range.start >= text_end {
            continue;
        }
        if range.start > cursor {
            let plain = &text[(cursor - text_start_in_line)..(range.start - text_start_in_line)];
            append(job, plain, color, style);
        }
        let mat_start = range.start.max(cursor);
        let mat_end = range.end.min(text_end);
        let mat = &text[(mat_start - text_start_in_line)..(mat_end - text_start_in_line)];
        let bg = if *is_active {
            active_match_bg
        } else {
            match_bg
        };
        append_with_bg(job, mat, color, style, bg);
        cursor = mat_end;
    }
    if cursor < text_end {
        let tail = &text[(cursor - text_start_in_line)..];
        append(job, tail, color, style);
    }
}

fn append_with_bg(
    job: &mut egui::text::LayoutJob,
    text: &str,
    color: egui::Color32,
    style: FontStyle,
    background: egui::Color32,
) {
    if text.is_empty() {
        return;
    }
    let mut format = egui::TextFormat {
        font_id: egui::FontId::monospace(SOURCE_FONT_SIZE),
        color,
        background,
        ..Default::default()
    };
    if style.contains(FontStyle::ITALIC) {
        format.italics = true;
    }
    if style.contains(FontStyle::UNDERLINE) {
        format.underline = egui::Stroke::new(1.0, color);
    }
    job.append(text, 0.0, format);
}

/// Return the line index (0-based) containing byte offset `pos`.
/// `pos == source.len()` belongs to the last line.
fn line_index_of(source: &str, pos: usize) -> usize {
    source[..pos.min(source.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
}

/// Render the gutter cell for `line_no`. Right-aligned in a field
/// `width` wide. The `│` separator used to live here, but is now
/// painted as a continuous line by `show_source_grid` so it doesn't
/// break between rows.
pub(crate) fn format_gutter(line_no: usize, width: usize) -> String {
    format!("{:>width$}", line_no, width = width)
}

fn append(job: &mut egui::text::LayoutJob, text: &str, color: egui::Color32, style: FontStyle) {
    if text.is_empty() {
        return;
    }
    let mut format = egui::TextFormat {
        font_id: egui::FontId::monospace(SOURCE_FONT_SIZE),
        color,
        ..Default::default()
    };
    if style.contains(FontStyle::ITALIC) {
        format.italics = true;
    }
    if style.contains(FontStyle::UNDERLINE) {
        format.underline = egui::Stroke::new(1.0, color);
    }
    job.append(text, 0.0, format);
}

fn syntect_to_egui(c: syntect::highlighting::Color) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn error_headline_covers_every_variant() {
        for (err, expected) in [
            (LoadError::NotFound, "ファイルが見つかりません"),
            (
                LoadError::PermissionDenied,
                "ファイルを読み取れません（権限不足）",
            ),
            (LoadError::NotUtf8, "UTF-8 として読めません"),
            (
                LoadError::TooLarge { size_bytes: 0 },
                "ファイルが大きすぎるため表示できません",
            ),
            (
                LoadError::Io("disk full".into()),
                "ファイルを開けませんでした",
            ),
        ] {
            assert_eq!(error_headline(&err), expected, "for {err:?}");
        }
    }

    #[test]
    fn error_detail_shows_size_for_too_large() {
        let detail = error_detail(&LoadError::TooLarge {
            size_bytes: 12 * 1024 * 1024,
        })
        .unwrap();
        assert!(detail.contains("12.0 MiB"), "got: {detail}");
        assert!(detail.contains("12582912"), "got: {detail}");
    }

    #[test]
    fn error_detail_passes_through_io_message() {
        let detail = error_detail(&LoadError::Io("permission timeout".into())).unwrap();
        assert_eq!(detail, "permission timeout");
    }

    #[test]
    fn error_detail_is_none_for_self_explanatory_errors() {
        assert!(error_detail(&LoadError::NotFound).is_none());
        assert!(error_detail(&LoadError::PermissionDenied).is_none());
        assert!(error_detail(&LoadError::NotUtf8).is_none());
    }

    #[test]
    fn size_warning_quotes_the_actual_size() {
        let doc = LoadedDocument {
            path: PathBuf::from("/tmp/big.md"),
            text: String::new(),
            size_bytes: 3 * 1024 * 1024 + 512 * 1024,
            size_class: SizeClass::Large,
        };
        let warning = size_warning(&doc);
        assert!(warning.contains("3.50 MiB"), "got: {warning}");
        assert!(warning.contains("1 MiB"), "got: {warning}");
    }

    #[test]
    fn set_document_transitions_state() {
        let mut state = PreviewState::default();
        let doc = LoadedDocument {
            path: PathBuf::from("/tmp/a.md"),
            text: "# A".into(),
            size_bytes: 3,
            size_class: SizeClass::Small,
        };
        state.set_document(doc);
        assert!(matches!(state.status, PreviewStatus::Loaded { .. }));

        state.clear();
        assert!(matches!(state.status, PreviewStatus::Empty));

        state.set_error("missing.md".into(), LoadError::NotFound);
        assert!(matches!(
            state.status,
            PreviewStatus::Failed {
                error: LoadError::NotFound,
                ..
            }
        ));
    }

    #[test]
    fn gutter_pads_to_width() {
        assert_eq!(format_gutter(1, 3), "  1");
        assert_eq!(format_gutter(42, 3), " 42");
        assert_eq!(format_gutter(123, 3), "123");
    }

    #[test]
    fn show_source_grid_renders_without_panic() {
        // egui needs an offscreen context to allocate text widgets.
        // We don't assert on visuals here; the goal is to detect a
        // panic in the grid-construction code path (e.g. unbounded
        // Layout::Job, missing syntax). The matching screenshot smoke
        // is covered by --enable-dev-tools.
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                show_source_grid(ui, "# Title\n\nbody\n", SYNTAX_THEME_DARK, None);
            });
        });
    }

    #[test]
    fn show_source_grid_handles_empty_source() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                show_source_grid(ui, "", SYNTAX_THEME_DARK, None);
            });
        });
    }

    #[test]
    fn show_source_grid_works_with_light_theme() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                show_source_grid(ui, "hello\n", SYNTAX_THEME_LIGHT, None);
            });
        });
    }
}
