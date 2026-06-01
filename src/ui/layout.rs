use eframe::egui;

const MIN_PANE_WIDTH: f32 = 240.0;

pub fn show(ui: &mut egui::Ui) {
    let avail = ui.available_width();
    let max_left = (avail - MIN_PANE_WIDTH).max(MIN_PANE_WIDTH);

    egui::Panel::left("preview_pane")
        .resizable(true)
        .default_size(avail / 2.0)
        .size_range(MIN_PANE_WIDTH..=max_left)
        .show_inside(ui, |ui| {
            crate::ui::preview_pane::show(ui);
        });

    egui::CentralPanel::default().show_inside(ui, |ui| {
        crate::ui::chat_pane::show(ui);
    });
}
