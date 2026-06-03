use eframe::egui;

use crate::preview::render::PreviewState;

/// Optional preview-pane chrome above the rendered Markdown. The
/// caller passes (a) the current watcher-error message if any
/// (Phase 5.3) and (b) the auto-follow state plus a re-enable
/// callback (Phase 7.2). When the bool is `false`, an inline
/// banner with a "再開する" button appears above the preview;
/// clicking the button calls `on_reenable_follow` so App can
/// flip the flag back to true.
pub fn show(
    ui: &mut egui::Ui,
    state: &mut PreviewState,
    watcher_error: Option<&str>,
    auto_follow_enabled: bool,
    on_reenable_follow: &mut dyn FnMut(),
) {
    let mut chrome_drawn = false;
    if let Some(message) = watcher_error {
        show_watcher_error_banner(ui, message);
        chrome_drawn = true;
    }
    if !auto_follow_enabled {
        show_follow_disabled_banner(ui, on_reenable_follow);
        chrome_drawn = true;
    }
    if chrome_drawn {
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

/// Phase 7.2 stop-gap: a single-line indicator with a re-enable
/// button shown only while auto-follow is OFF (i.e., after the
/// user has explicitly opened a file via `Cmd+O`). Phase 7.7 will
/// fold this into the path bar so the status and the toggle live
/// next to the file path.
fn show_follow_disabled_banner(ui: &mut egui::Ui, on_reenable_follow: &mut dyn FnMut()) {
    ui.horizontal(|ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new("自動追従: OFF").color(egui::Color32::from_rgb(180, 180, 180)),
            )
            .selectable(false),
        );
        if ui.small_button("再開する").clicked() {
            on_reenable_follow();
        }
    });
}
