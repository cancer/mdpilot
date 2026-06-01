use eframe::egui;

use crate::chat::history::ChatHistory;

pub fn show(ui: &mut egui::Ui, history: &mut ChatHistory) {
    crate::chat::view::show(ui, history);
}
