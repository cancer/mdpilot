use std::path::{Path, PathBuf};

use eframe::egui;

use crate::preview::render::PreviewState;
use crate::ui::file_tree::{self, FileTreeAction};

/// Width of the file-tree sidebar when it's open. Resizing is not
/// supported yet — matches the `path_bar` decision to keep chrome
/// state-light until we have a settings file (Phase 9.3).
const FILE_TREE_WIDTH: f32 = 220.0;

/// Outcome of a single frame's preview-pane render. `Some(path)`
/// when the user clicked a file in the tree; the caller loads it.
pub struct PreviewPaneOutcome {
    pub open_file: Option<PathBuf>,
}

/// Phase 9.X.4: optional left sidebar with the project file tree.
/// When `tree_open` is true the pane is split horizontally into
/// `[tree | source view]`; otherwise it falls back to the source
/// view filling the full width.
pub fn show(
    ui: &mut egui::Ui,
    state: &mut PreviewState,
    project_root: &Path,
    tree_open: bool,
) -> PreviewPaneOutcome {
    let mut outcome = PreviewPaneOutcome { open_file: None };
    if tree_open {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(FILE_TREE_WIDTH, ui.available_height()),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    if let FileTreeAction::Open(path) = file_tree::show(ui, project_root) {
                        outcome.open_file = Some(path);
                    }
                },
            );
            ui.separator();
            ui.vertical(|ui| {
                crate::preview::render::show(ui, state);
            });
        });
    } else {
        crate::preview::render::show(ui, state);
    }
    outcome
}
