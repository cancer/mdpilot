use std::path::{Path, PathBuf};

use eframe::egui;

use crate::preview::render::PreviewState;
use crate::ui::file_tree::{self, FileTreeAction, FileTreeState};

/// Width of the file-tree sidebar when it's open. Resizing is not
/// supported yet — matches the `path_bar` decision to keep chrome
/// state-light until we have a settings file (Phase 9.3).
const FILE_TREE_WIDTH: f32 = 220.0;

/// Outcome of a single frame's preview-pane render.
pub struct PreviewPaneOutcome {
    /// User clicked a file in the tree; caller should load it.
    pub open_file: Option<PathBuf>,
    /// Phase 10.5: user picked one of the conflict-banner buttons.
    pub conflict_action: ConflictAction,
    /// Phase 10.7: user picked one of the follow-prompt buttons.
    pub follow_action: FollowAction,
    /// Phase 10.9: tree user pressed Esc → caller should move focus
    /// back to preview.
    pub tree_exit_to_preview: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAction {
    None,
    Reload,
    Keep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowAction {
    None,
    Accept,
    Dismiss,
}

/// Phase 9.X.4 + 10.5: optional left sidebar with the project file
/// tree, plus an in-pane conflict banner when the document was
/// edited both locally and on disk.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    state: &mut PreviewState,
    project_root: &Path,
    tree_open: bool,
    tree_state: &mut FileTreeState,
    conflict_detected: bool,
    follow_prompt: Option<&Path>,
) -> PreviewPaneOutcome {
    let mut outcome = PreviewPaneOutcome {
        open_file: None,
        conflict_action: ConflictAction::None,
        follow_action: FollowAction::None,
        tree_exit_to_preview: false,
    };
    // Phase 10.16 (2026-06-11): tree と preview を独立した scroll
    // region に分離するため、`Panel::left` ベースに切り替え。
    // 旧 `horizontal_top` レイアウトでは tree の ScrollArea が
    // preview pane 上の hover を奪ってしまい、preview がマウス
    // ホイールで動かない問題があった。
    if tree_open {
        egui::Panel::left("preview_file_tree")
            .resizable(false)
            .default_size(FILE_TREE_WIDTH)
            .show_inside(ui, |ui| {
                match file_tree::show(ui, project_root, tree_state) {
                    FileTreeAction::Open(path) => outcome.open_file = Some(path),
                    FileTreeAction::ExitToPreview => outcome.tree_exit_to_preview = true,
                    FileTreeAction::None => {}
                }
            });
    }
    if let Some(path) = follow_prompt {
        outcome.follow_action = show_follow_banner(ui, path);
    }
    if conflict_detected {
        outcome.conflict_action = show_conflict_banner(ui);
    }
    crate::preview::render::show(ui, state);
    outcome
}

fn show_follow_banner(ui: &mut egui::Ui, path: &Path) -> FollowAction {
    let mut action = FollowAction::None;
    let label = path.display().to_string();
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(80, 160, 220, 30))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!(
                            "Claude が {label} を編集しました。 切替えますか？",
                        ))
                        .color(egui::Color32::from_rgb(80, 160, 220)),
                    )
                    .selectable(false),
                );
                if ui
                    .small_button("開く")
                    .on_hover_text("preview を新ファイルに切替")
                    .clicked()
                {
                    action = FollowAction::Accept;
                }
                if ui
                    .small_button("留まる")
                    .on_hover_text("現在の編集を続ける (バナーを閉じる)")
                    .clicked()
                {
                    action = FollowAction::Dismiss;
                }
            });
        });
    ui.separator();
    action
}

fn show_conflict_banner(ui: &mut egui::Ui) -> ConflictAction {
    let mut action = ConflictAction::None;
    let amber = egui::Color32::from_rgb(220, 180, 70);
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(220, 180, 70, 30))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "⚠ ファイルが外部から変更されました (Claude 等)。 未保存の編集と競合しています。",
                        )
                        .color(amber),
                    )
                    .selectable(false),
                );
                if ui
                    .small_button("ディスクを読む")
                    .on_hover_text("buffer を破棄して disk から再読込")
                    .clicked()
                {
                    action = ConflictAction::Reload;
                }
                if ui
                    .small_button("buffer を保つ")
                    .on_hover_text("次の保存で disk を buffer で上書き")
                    .clicked()
                {
                    action = ConflictAction::Keep;
                }
                ui.add_enabled(
                    false,
                    egui::Button::new("diff (MVP 後)"),
                )
                .on_disabled_hover_text("差分表示は未実装");
            });
        });
    ui.separator();
    action
}
