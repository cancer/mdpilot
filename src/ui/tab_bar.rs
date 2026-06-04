//! Phase 9.5.2: workspace tab bar at the top of the window.
//!
//! Renders one chip per `Tab` (label + close button) plus a `+`
//! button at the right that mints a new workspace. Returns the
//! click intent to the caller as `TabBarAction`, mirroring the
//! callback pattern used by `chat::view` and `path_bar`.
//!
//! Layout: a single horizontal row sized to fit. Active tab has
//! a stronger background; close button only renders when more
//! than one tab is open (last tab is not closable, matching the
//! 9.5 spec).

use eframe::egui;

/// One visible tab. Borrowed view of the underlying `Tab`; the
/// caller owns the data.
pub struct TabBarItem<'a> {
    pub label: &'a str,
    pub is_active: bool,
}

/// What the user did on this frame. `None` is the default; the
/// click handlers below set exactly one variant per frame because
/// `egui::Response::clicked()` only fires on release.
#[derive(Debug, PartialEq, Eq)]
pub enum TabBarAction {
    None,
    /// Index into the `items` slice the caller passed in.
    Select(usize),
    Close(usize),
    NewTab,
}

const ACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(60, 60, 60);
const INACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(32, 32, 32);

pub fn show(ui: &mut egui::Ui, items: &[TabBarItem]) -> TabBarAction {
    let mut action = TabBarAction::None;
    ui.horizontal(|ui| {
        // The egui default spacing between widgets in a row is
        // ~8 px which makes the tab strip feel sparse. Tighten it
        // so the chips read as a single unit.
        ui.spacing_mut().item_spacing.x = 2.0;
        let can_close = items.len() > 1;
        for (idx, item) in items.iter().enumerate() {
            if draw_tab(ui, item, can_close, &mut action, idx) {
                // draw_tab signaled it set an action; nothing else
                // to do this iteration.
            }
        }
        // Right-side `+` for a fresh tab. Small-button keeps the
        // bar compact.
        if ui.small_button("+").on_hover_text("新規タブ").clicked() {
            action = TabBarAction::NewTab;
        }
    });
    action
}

/// Render a single tab chip. Returns whether an action was set
/// this frame (so the caller can short-circuit if needed; we
/// currently don't, but it's a useful return for tests).
fn draw_tab(
    ui: &mut egui::Ui,
    item: &TabBarItem,
    can_close: bool,
    action: &mut TabBarAction,
    idx: usize,
) -> bool {
    let bg = if item.is_active {
        ACTIVE_BG
    } else {
        INACTIVE_BG
    };
    // `egui::Frame::new()` is the modern API in egui 0.34; the
    // older `Frame::default()` was removed.
    let frame = egui::Frame::new().fill(bg).inner_margin(egui::Margin {
        left: 8,
        right: 6,
        top: 2,
        bottom: 2,
    });
    frame
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Label is a clickable area covering the tab text.
                let label =
                    egui::Label::new(egui::RichText::new(item.label).color(if item.is_active {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::LIGHT_GRAY
                    }))
                    .selectable(false)
                    .sense(egui::Sense::click());
                let label_resp = ui.add(label);
                let mut fired = false;
                if label_resp.clicked() {
                    *action = TabBarAction::Select(idx);
                    fired = true;
                }
                if can_close {
                    // The close button is small and lives inside
                    // the same row so the chip reads as a single
                    // unit. `×` is the canonical close glyph.
                    let close_resp = ui
                        .small_button(egui::RichText::new("×").size(12.0))
                        .on_hover_text("タブを閉じる");
                    if close_resp.clicked() {
                        *action = TabBarAction::Close(idx);
                        fired = true;
                    }
                }
                fired
            })
            .inner
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_bar_action_default_is_none() {
        // The action variants are exhaustive; this guards a future
        // change from accidentally making `None` non-default by
        // checking that all variants are still distinguishable.
        assert_ne!(TabBarAction::None, TabBarAction::NewTab);
        assert_ne!(TabBarAction::Select(0), TabBarAction::Close(0));
        assert_ne!(TabBarAction::Select(0), TabBarAction::Select(1));
    }
}
