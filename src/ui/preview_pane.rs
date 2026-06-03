use eframe::egui;

use crate::preview::render::PreviewState;

pub fn show(ui: &mut egui::Ui, state: &mut PreviewState) {
    crate::preview::render::show(ui, state);
}
