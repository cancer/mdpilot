use eframe::egui;

pub fn show(ui: &mut egui::Ui) {
    ui.centered_and_justified(|ui| {
        ui.label("プレビュー未指定");
    });
}
