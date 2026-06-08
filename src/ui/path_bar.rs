//! Phase 7.7: a one-line status strip at the top of the window.
//!
//! Absorbs the stop-gap banners introduced in Phase 5.3
//! (watcher_error) and Phase 7.2 (auto-follow toggle), and adds
//! the spec'd preview path display + claude connection indicator.
//!
//! Layout: left-aligned preview path, right-aligned status chips
//! (`auto-follow` toggle, `claude` connection state, optional
//! watcher error). The watcher-error chip is amber so it reads
//! as "non-fatal warning", consistent with the prior banner.

use eframe::egui;

use crate::preview::render::{PreviewState, PreviewStatus};

const CLAUDE_OK: egui::Color32 = egui::Color32::from_rgb(80, 200, 100);
const CLAUDE_DOWN: egui::Color32 = egui::Color32::from_rgb(220, 90, 80);
const WARN_AMBER: egui::Color32 = egui::Color32::from_rgb(220, 180, 70);

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    preview: &PreviewState,
    auto_follow_enabled: bool,
    on_toggle_follow: &mut dyn FnMut(),
    watcher_error: Option<&str>,
    session_alive: bool,
    file_tree_open: bool,
    on_toggle_tree: &mut dyn FnMut(),
    is_unbound: bool,
) {
    ui.horizontal(|ui| {
        if is_unbound {
            ui.add(
                egui::Label::new(
                    egui::RichText::new("（プロジェクト未選択 / Cmd+O で選択）").weak(),
                )
                .selectable(false),
            );
        } else {
            let path_text = preview_path_label(preview);
            ui.add(
                egui::Label::new(egui::RichText::new(path_text))
                    .selectable(true)
                    .truncate(),
            );
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(err) = watcher_error {
                ui.add(
                    egui::Label::new(egui::RichText::new(err).color(WARN_AMBER)).selectable(true),
                );
                ui.separator();
            }

            let (claude_color, claude_text) = if session_alive {
                (CLAUDE_OK, "● Claude 接続中")
            } else {
                (CLAUDE_DOWN, "● Claude 切断")
            };
            ui.add(
                egui::Label::new(egui::RichText::new(claude_text).color(claude_color))
                    .selectable(false),
            );
            ui.separator();

            let follow_text = if auto_follow_enabled {
                "自動追従: ON"
            } else {
                "自動追従: OFF"
            };
            if ui.small_button(follow_text).clicked() {
                on_toggle_follow();
            }
            ui.separator();

            let tree_text = if file_tree_open {
                "ツリー: ON"
            } else {
                "ツリー: OFF"
            };
            if ui
                .small_button(tree_text)
                .on_hover_text("ファイルツリー サイドバー (Cmd+B)")
                .clicked()
            {
                on_toggle_tree();
            }
        });
    });
}

/// Build the left-aligned label text for the current preview state.
/// `Loaded` shows the absolute path, `Failed` prefixes a warning
/// glyph, `Empty` shows the localized placeholder. Pure so we can
/// unit-test the label-derivation rules without involving egui.
pub(crate) fn preview_path_label(preview: &PreviewState) -> String {
    match &preview.status {
        PreviewStatus::Loaded { document, .. } => document.path.display().to_string(),
        PreviewStatus::Failed { path_label, .. } => format!("⚠ {path_label}"),
        PreviewStatus::Empty => "（プレビュー未指定）".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::loader::{LoadError, LoadedDocument, SizeClass};
    use std::path::PathBuf;

    fn state_loaded(path: &str) -> PreviewState {
        PreviewState::loaded(LoadedDocument {
            path: PathBuf::from(path),
            text: String::new(),
            size_bytes: 0,
            size_class: SizeClass::Small,
        })
    }

    fn state_failed(path: &str) -> PreviewState {
        let mut s = PreviewState::default();
        s.set_error(path.to_string(), LoadError::NotFound);
        s
    }

    #[test]
    fn empty_label_uses_placeholder() {
        let s = PreviewState::default();
        assert_eq!(preview_path_label(&s), "（プレビュー未指定）");
    }

    #[test]
    fn loaded_label_shows_absolute_path() {
        let s = state_loaded("/Users/u/proj/README.md");
        assert_eq!(preview_path_label(&s), "/Users/u/proj/README.md");
    }

    #[test]
    fn failed_label_prefixes_warning_glyph() {
        let s = state_failed("/missing.md");
        assert_eq!(preview_path_label(&s), "⚠ /missing.md");
    }
}
