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

use crate::preview::loader::{LoadError, LoadedDocument, SizeClass, SOFT_LIMIT_BYTES};

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
        Self {
            status: PreviewStatus::Loaded(document),
            cache: CommonMarkCache::default(),
        }
    }

    /// Replace the displayed document. Clears the egui_commonmark cache
    /// so prior-document state (scroll positions keyed by heading id,
    /// link_hooks, and the syntect SyntaxSet/ThemeSet once Phase 4.3
    /// enables `better_syntax_highlighting`) doesn't carry over.
    pub fn set_document(&mut self, document: LoadedDocument) {
        self.cache = CommonMarkCache::default();
        self.status = PreviewStatus::Loaded(document);
    }

    pub fn set_error(&mut self, path_label: String, error: LoadError) {
        self.status = PreviewStatus::Failed { path_label, error };
    }

    pub fn clear(&mut self) {
        self.status = PreviewStatus::Empty;
    }
}

/// Three-way state of the preview pane.
pub enum PreviewStatus {
    Empty,
    Loaded(LoadedDocument),
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
        PreviewStatus::Loaded(document) => {
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
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    CommonMarkViewer::new().show(ui, cache, &document.text);
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
    fn set_document_resets_cache() {
        // CommonMarkCache doesn't expose its size, so we just exercise the
        // public state machine and trust that allocating a fresh
        // CommonMarkCache happens (we'd see a test compile failure if the
        // method signature drifted).
        let mut state = PreviewState::default();
        let doc = LoadedDocument {
            path: PathBuf::from("a.md"),
            text: "# A".into(),
            size_bytes: 3,
            size_class: SizeClass::Small,
        };
        state.set_document(doc);
        assert!(matches!(state.status, PreviewStatus::Loaded(_)));

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
}
