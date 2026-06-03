// PreviewState is plumbed through App in Phase 4.2. The render module is
// reachable from the bin crate, but the size warning / error display paths
// only fire on real load failures so the `Failed` and `Large` arms can
// still look unreachable to dead-code analysis.
#![allow(dead_code)]

//! egui_commonmark renderer for the preview pane.
//!
//! Holds the `CommonMarkCache` (egui_commonmark's per-frame parse/layout
//! cache) and the current `PreviewStatus`. The cache lives on the UI
//! thread — egui_commonmark mutates it during `show`, so we hand it out
//! `&mut` and never share it across threads.

use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use crate::preview::image::rewrite_image_uris;
use crate::preview::loader::{LoadError, LoadedDocument, SizeClass, SOFT_LIMIT_BYTES};

/// syntect theme used in dark mode. Bundled with syntect's
/// `load_defaults` (dumps.rs:212), so available without any extra
/// theme files at runtime. Spec source: `docs/preview.md` §4.
const SYNTAX_THEME_DARK: &str = "base16-ocean.dark";

/// syntect theme used in light mode. Also bundled by default. Spec
/// source: `docs/preview.md` §4 ("InspiredGitHub 程度").
const SYNTAX_THEME_LIGHT: &str = "InspiredGitHub";

/// State held by `App` for the preview pane.
pub struct PreviewState {
    pub status: PreviewStatus,
    cache: CommonMarkCache,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            status: PreviewStatus::Empty,
            cache: CommonMarkCache::default(),
        }
    }
}

impl PreviewState {
    pub fn loaded(document: LoadedDocument) -> Self {
        let rendered_text_override = render_override_for(&document);
        Self {
            status: PreviewStatus::Loaded {
                document,
                rendered_text_override,
            },
            cache: CommonMarkCache::default(),
        }
    }

    /// Replace the displayed document. Refreshing the
    /// `CommonMarkCache` here is mainly to drop its per-heading scroll
    /// HashMap (`scroll: HashMap<egui::Id, ScrollableCache>` in
    /// egui_commonmark_backend's `misc.rs:418`) so anchors from the
    /// prior document don't bleed in. The syntect SyntaxSet/ThemeSet
    /// it also holds are static `load_defaults`, so re-loading them is
    /// wasted work — a future optimization could swap to `clear_scroll`
    /// if egui_commonmark ever exposes one.
    pub fn set_document(&mut self, document: LoadedDocument) {
        let rendered_text_override = render_override_for(&document);
        self.cache = CommonMarkCache::default();
        self.status = PreviewStatus::Loaded {
            document,
            rendered_text_override,
        };
    }

    pub fn set_error(&mut self, path_label: String, error: LoadError) {
        self.status = PreviewStatus::Failed { path_label, error };
    }

    pub fn clear(&mut self) {
        self.status = PreviewStatus::Empty;
    }
}

/// Build the post-processed text we hand to egui_commonmark, or `None`
/// when the raw `document.text` is fine to render as-is. Two transforms
/// stack here:
///
/// 1. **Image URI rewriting** (Phase 4.5, `docs/preview.md` §6) — every
///    inline image with a relative or absolute filesystem URL is
///    rewritten into `file://<absolute>` form, resolved against
///    `document.path.parent()`. External (`http(s)://`, `data:`, …)
///    URLs are untouched.
/// 2. **Large-doc code fence stripping** (`docs/preview.md` §4) — for
///    `SizeClass::Large` docs we strip every fenced code block's
///    info-string so each block falls through egui_commonmark's
///    `plain_highlighting` path instead of syntect.
///
/// The two transforms compose: rewrite first so the stripped output
/// preserves the resolved image URIs.
fn render_override_for(document: &LoadedDocument) -> Option<String> {
    let base_dir = document.path.parent();
    let rewritten = rewrite_image_uris(&document.text, base_dir);
    let processed = if document.size_class == SizeClass::Large {
        strip_code_block_info_strings(&rewritten)
    } else {
        rewritten
    };
    if processed == document.text {
        None
    } else {
        Some(processed)
    }
}

/// Three-way state of the preview pane.
#[derive(Debug)]
pub enum PreviewStatus {
    Empty,
    Loaded {
        document: LoadedDocument,
        /// `Some(text)` when we pre-processed the document body (Large
        /// docs get info-strings stripped to force plain rendering).
        /// `None` means hand `document.text` to egui_commonmark as-is.
        rendered_text_override: Option<String>,
    },
    Failed {
        /// Path string as the caller saw it (we don't keep the original
        /// `PathBuf` because the error may be `NotFound`).
        path_label: String,
        error: LoadError,
    },
}

pub fn show(ui: &mut egui::Ui, state: &mut PreviewState) {
    // Destructure once so the two `&mut` borrows below — one to
    // `status` for reading the document, the other to `cache` for
    // egui_commonmark — are disjoint and don't need any cloning of the
    // (potentially several-MiB) document body each frame.
    let PreviewState { status, cache } = state;
    match status {
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
        PreviewStatus::Loaded {
            document,
            rendered_text_override,
        } => {
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
            let body = rendered_text_override
                .as_deref()
                .unwrap_or(document.text.as_str());
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Themes match docs/preview.md §4. egui_commonmark
                    // picks light vs dark by ui.style().visuals.dark_mode.
                    CommonMarkViewer::new()
                        .syntax_theme_dark(SYNTAX_THEME_DARK)
                        .syntax_theme_light(SYNTAX_THEME_LIGHT)
                        .show(ui, cache, body);
                });
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

/// Short human-readable headline for the error variant. Pure so we can
/// unit-test the mapping without spinning up egui.
pub(crate) fn error_headline(error: &LoadError) -> &'static str {
    match error {
        LoadError::NotFound => "ファイルが見つかりません",
        LoadError::PermissionDenied => "ファイルを読み取れません（権限不足）",
        LoadError::NotUtf8 => "UTF-8 として読めません",
        LoadError::TooLarge { .. } => "ファイルが大きすぎるため表示できません",
        LoadError::Io(_) => "ファイルを開けませんでした",
    }
}

/// Secondary line with the size limit or OS error. Optional because
/// `NotFound` / `PermissionDenied` / `NotUtf8` are self-explanatory.
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

/// Warning banner shown above a `SizeClass::Large` document.
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

/// Rewrite every fenced code block's opening fence to drop the
/// info-string (the part after the backticks/tildes). egui_commonmark
/// routes language-less code blocks through `plain_highlighting`
/// instead of syntect — so this is how we honor `docs/preview.md` §4's
/// "Large docs disable syntax highlighting" rule without forking
/// egui_commonmark's render path.
///
/// Tracks fence state line-by-line so a stray ``` inside a paragraph or
/// inside another fence doesn't get mangled. Matches both backtick
/// (` ``` `) and tilde (`~~~`) fences and honors variable-length runs
/// per CommonMark §4.5.
pub(crate) fn strip_code_block_info_strings(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut open_fence: Option<(char, usize)> = None;
    for line in markdown.split_inclusive('\n') {
        // `split_inclusive` keeps the trailing '\n' in each chunk; the
        // body matchers below want it stripped, but we still write it
        // back at the end.
        let (body, newline) = match line.strip_suffix('\n') {
            Some(rest) => (rest, "\n"),
            None => (line, ""),
        };

        if let Some((fc, fcount)) = open_fence {
            // Inside a fenced block: passthrough until we find a
            // matching closing fence.
            if is_closing_fence(body, fc, fcount) {
                open_fence = None;
            }
            out.push_str(body);
            out.push_str(newline);
            continue;
        }

        if let Some((fc, fcount, indent)) = match_opening_fence(body) {
            // Rewrite the line to: <indent><fcount * fc><newline>
            out.push_str(&body[..indent]);
            for _ in 0..fcount {
                out.push(fc);
            }
            out.push_str(newline);
            open_fence = Some((fc, fcount));
            continue;
        }

        out.push_str(body);
        out.push_str(newline);
    }
    out
}

/// Returns `Some((fence_char, run_length, indent_byte_offset))` if
/// `body` opens a fenced code block per CommonMark §4.5. `indent` is
/// 0..=3 spaces (the only valid indents for a fence).
fn match_opening_fence(body: &str) -> Option<(char, usize, usize)> {
    // CommonMark allows up to 3 spaces of indentation before the fence.
    let indent = body.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return None;
    }
    let rest = &body[indent..];
    let first = rest.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = rest.chars().take_while(|&c| c == first).count();
    if count < 3 {
        return None;
    }
    // Backtick fences additionally forbid backticks anywhere on the
    // info-string line (CommonMark §4.5 last paragraph). Tilde fences
    // are looser; rather than encode the full rule, just verify the
    // info-string after the run doesn't contain the fence char for
    // backticks — that's enough to avoid mis-classifying inline-code
    // lines like "``not a fence``".
    if first == '`' {
        let after = &rest[count..];
        if after.contains('`') {
            return None;
        }
    }
    Some((first, count, indent))
}

/// True when `body` is a valid closing fence for an open fence of
/// `(fence_char, open_count)`. CommonMark §4.5 requires the closing
/// run to be at least as long as the opening, the same char, and no
/// trailing non-whitespace.
fn is_closing_fence(body: &str, fence_char: char, open_count: usize) -> bool {
    let indent = body.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return false;
    }
    let rest = &body[indent..];
    let count = rest.chars().take_while(|&c| c == fence_char).count();
    if count < open_count {
        return false;
    }
    let after = &rest[count..];
    after.chars().all(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn error_headline_covers_every_variant() {
        // Use a representative value for each variant so a future variant
        // addition forces an update via exhaustive match in the production
        // code.
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
        assert!(
            detail.contains("12582912"),
            "should include byte count: {detail}"
        );
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
        // Use an absolute path so a future maintainer adding an image
        // to this fixture sees a well-formed `file:///abs/...` URI,
        // not the partially-resolved `file://img.png` that `a.md`'s
        // empty parent would produce.
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
    fn small_documents_skip_the_render_override() {
        let state = PreviewState::loaded(LoadedDocument {
            path: PathBuf::from("small.md"),
            text: "```rust\nfn main() {}\n```\n".into(),
            size_bytes: 30,
            size_class: SizeClass::Small,
        });
        match state.status {
            PreviewStatus::Loaded {
                rendered_text_override,
                ..
            } => assert!(
                rendered_text_override.is_none(),
                "Small docs should render raw text",
            ),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[test]
    fn small_documents_with_relative_images_get_rewritten() {
        // Phase 4.5: relative image URIs must be rewritten even for
        // Small docs, because that's the only path egui_extras can
        // resolve against an absolute filesystem location.
        let state = PreviewState::loaded(LoadedDocument {
            path: PathBuf::from("/proj/docs/intro.md"),
            text: "see ![diagram](images/x.png)".into(),
            size_bytes: 32,
            size_class: SizeClass::Small,
        });
        match state.status {
            PreviewStatus::Loaded {
                rendered_text_override,
                ..
            } => {
                let rendered =
                    rendered_text_override.expect("relative image must trigger the override");
                assert!(
                    rendered.contains("file:///proj/docs/images/x.png"),
                    "image URI must be absolute file:// URI: {rendered}",
                );
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[test]
    fn large_documents_compose_image_rewrite_and_fence_strip() {
        // Phase 4.5: both transforms should stack — relative image URI
        // resolved to file:// AND the code-block info-string stripped.
        let state = PreviewState::loaded(LoadedDocument {
            path: PathBuf::from("/proj/big.md"),
            text: "![](logo.png)\n\n```rust\nfn x() {}\n```\n".into(),
            size_bytes: 2 * 1024 * 1024,
            size_class: SizeClass::Large,
        });
        match state.status {
            PreviewStatus::Loaded {
                rendered_text_override,
                ..
            } => {
                let rendered = rendered_text_override.expect("Large doc must override");
                assert!(
                    rendered.contains("file:///proj/logo.png"),
                    "image must be rewritten: {rendered}",
                );
                assert!(
                    rendered.contains("```\nfn x"),
                    "info-string must be stripped: {rendered}",
                );
                assert!(
                    !rendered.contains("```rust"),
                    "info-string `rust` must be gone: {rendered}",
                );
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[test]
    fn large_documents_get_info_strings_stripped() {
        let state = PreviewState::loaded(LoadedDocument {
            path: PathBuf::from("large.md"),
            text: "before\n\n```rust\nfn main() {}\n```\n\nafter\n".into(),
            size_bytes: 2 * 1024 * 1024,
            size_class: SizeClass::Large,
        });
        match state.status {
            PreviewStatus::Loaded {
                rendered_text_override,
                ..
            } => {
                let rendered = rendered_text_override.expect("Large doc must override");
                assert!(
                    rendered.contains("```\nfn main"),
                    "info string should be gone: {rendered:?}",
                );
                assert!(
                    !rendered.contains("```rust"),
                    "info string `rust` should be removed: {rendered:?}",
                );
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[test]
    fn strip_leaves_non_code_text_alone() {
        let input = "# Title\n\nA paragraph with a `code span`.\n\n- item\n";
        let output = strip_code_block_info_strings(input);
        assert_eq!(output, input);
    }

    #[test]
    fn strip_handles_tilde_fences() {
        let input = "~~~python\nprint(1)\n~~~\n";
        let output = strip_code_block_info_strings(input);
        assert_eq!(output, "~~~\nprint(1)\n~~~\n");
    }

    #[test]
    fn strip_handles_multiple_blocks() {
        let input = "```rust\nfn a() {}\n```\ntext\n```ts\nlet b = 0;\n```\n";
        let output = strip_code_block_info_strings(input);
        assert_eq!(output, "```\nfn a() {}\n```\ntext\n```\nlet b = 0;\n```\n",);
    }

    #[test]
    fn strip_respects_variable_fence_length() {
        // 4-backtick opener can only be closed by 4+ backticks; a
        // 3-backtick line inside is content, not a closer.
        let input = "````rust\n```\nstill inside\n````\n";
        let output = strip_code_block_info_strings(input);
        assert_eq!(output, "````\n```\nstill inside\n````\n");
    }

    #[test]
    fn strip_leaves_inline_code_alone() {
        // An info-string line with extra backticks is not a valid
        // opening fence per CommonMark §4.5; verify we don't misfire on
        // text like "``inline``".
        let input = "This is ``inline`` code, not a fence.\n";
        let output = strip_code_block_info_strings(input);
        assert_eq!(output, input);
    }

    #[test]
    fn strip_preserves_indented_fences() {
        // CommonMark allows up to 3 spaces of indent before a fence;
        // beyond that, it would be indented-code.
        let input = "   ```rust\nfn x() {}\n   ```\n";
        let output = strip_code_block_info_strings(input);
        assert_eq!(output, "   ```\nfn x() {}\n   ```\n");
    }

    #[test]
    fn strip_preserves_trailing_newline_presence() {
        // No trailing \n in input → no trailing \n in output.
        let input = "```rs\nfn x() {}\n```";
        let output = strip_code_block_info_strings(input);
        assert_eq!(output, "```\nfn x() {}\n```");
    }
}
