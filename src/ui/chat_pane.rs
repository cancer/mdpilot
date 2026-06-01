use eframe::egui;

pub fn show(ui: &mut egui::Ui) {
    ui.centered_and_justified(|ui| {
        ui.label("Claude 接続準備中…");
    });
}
