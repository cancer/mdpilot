use eframe::egui;

use crate::preview::render::PreviewState;

/// Phase 7.7 simplified the preview pane: the watcher-error and
/// auto-follow stop-gap banners moved up into `ui::path_bar`, so
/// this is back to just delegating to the Markdown renderer.
pub fn show(ui: &mut egui::Ui, state: &mut PreviewState) {
    crate::preview::render::show(ui, state);
}
