use eframe::egui;

use crate::chat::history::ChatHistory;
use crate::preview::render::PreviewState;

const MIN_PANE_WIDTH: f32 = 240.0;
const PREVIEW_PANEL_ID: &str = "preview_pane";

pub fn show(
    ui: &mut egui::Ui,
    history: &mut ChatHistory,
    preview: &mut PreviewState,
    session_alive: bool,
    on_send: &mut dyn FnMut(String),
) {
    let avail = ui.available_width();
    let max_left = (avail - MIN_PANE_WIDTH).max(MIN_PANE_WIDTH);

    let preview_response = egui::Panel::left(PREVIEW_PANEL_ID)
        .resizable(true)
        .default_size(avail / 2.0)
        .size_range(MIN_PANE_WIDTH..=max_left)
        .show_inside(ui, |ui| {
            crate::ui::preview_pane::show(ui, preview);
        });

    // Hit-strip on the right edge of the preview pane: a thin column where the
    // resize handle lives. A double-click here resets to a 50/50 split.
    let edge_x = preview_response.response.rect.right();
    let edge_rect = egui::Rect::from_x_y_ranges(
        (edge_x - 4.0)..=(edge_x + 4.0),
        preview_response.response.rect.y_range(),
    );
    let edge = ui.interact(
        edge_rect,
        egui::Id::new("pane_boundary_dblclick"),
        egui::Sense::click(),
    );
    if edge.double_clicked() {
        reset(ui.ctx());
    }

    egui::CentralPanel::default().show_inside(ui, |ui| {
        crate::ui::chat_pane::show(ui, history, session_alive, on_send);
    });
}

/// Drop the persisted panel state so the next frame falls back to
/// `default_size` (a 50/50 split for our layout).
pub fn reset(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<egui::PanelState>(egui::Id::new(PREVIEW_PANEL_ID));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_clears_persisted_panel_state() {
        let ctx = egui::Context::default();
        let id = egui::Id::new(PREVIEW_PANEL_ID);
        let stored = egui::PanelState {
            rect: egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(123.0, 456.0)),
        };
        ctx.data_mut(|d| d.insert_persisted(id, stored));

        let before: Option<egui::PanelState> = ctx.data_mut(|d| d.get_persisted(id));
        assert!(before.is_some(), "precondition: state was stored");

        reset(&ctx);

        let after: Option<egui::PanelState> = ctx.data_mut(|d| d.get_persisted(id));
        assert!(after.is_none(), "reset should remove the persisted state");
    }

    #[test]
    fn reset_is_a_no_op_when_nothing_is_stored() {
        let ctx = egui::Context::default();
        // Should not panic and should leave the (absent) state absent.
        reset(&ctx);
        let after: Option<egui::PanelState> =
            ctx.data_mut(|d| d.get_persisted(egui::Id::new(PREVIEW_PANEL_ID)));
        assert!(after.is_none());
    }
}
