use eframe::egui;

use crate::preview::render::PreviewState;

pub fn show(ui: &mut egui::Ui, state: &mut PreviewState, watcher_error: Option<&str>) {
    if let Some(message) = watcher_error {
        show_watcher_error_banner(ui, message);
        ui.separator();
    }
    crate::preview::render::show(ui, state);
}

/// Phase 5.3 stop-gap: a single-line banner above the preview pane
/// that surfaces `App::watcher_error`. The Phase 7.7 status bar will
/// own this signal in the long run, at which point this helper can
/// be replaced with a status-bar entry. Color matches the
/// `size_warning` amber to read as "non-fatal warning, action
/// optional" rather than `LoadError`'s red.
fn show_watcher_error_banner(ui: &mut egui::Ui, message: &str) {
    ui.add(
        egui::Label::new(egui::RichText::new(message).color(egui::Color32::from_rgb(220, 180, 70)))
            .selectable(true),
    );
}
