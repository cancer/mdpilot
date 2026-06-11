use eframe::egui;

use crate::chat::history::ChatHistory;

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    history: &mut ChatHistory,
    session_alive: bool,
    on_send: &mut dyn FnMut(String),
    on_abort: &mut dyn FnMut(),
) {
    crate::chat::view::show(ui, history, session_alive, on_send, on_abort);
}
