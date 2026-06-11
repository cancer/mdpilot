//! Phase 9.X.2 + 10.10: modal picker that lets the user resume a
//! past claude session, with keyboard navigation.
//!
//! Rendered as a centered `egui::Window` over the main UI. The
//! caller supplies the `SessionMeta` list (already sorted newest
//! first by `chat::history_picker::list_sessions`); the picker
//! returns a `SessionPickerAction` that App matches on to open a
//! resumed tab or close the modal.

use std::time::SystemTime;

use chrono::{DateTime, Local};
use eframe::egui;
use uuid::Uuid;

use crate::chat::history_picker::SessionMeta;

/// User intent emitted by the picker for the current frame.
#[derive(Debug, PartialEq, Eq)]
pub enum SessionPickerAction {
    None,
    Close,
    Resume(Uuid),
}

/// Phase 10.10: persisted picker state — currently just the selected
/// card. Kept across frames so j/k movement sticks.
#[derive(Debug, Default)]
pub struct SessionPickerState {
    pub selected: usize,
}

pub fn show(
    ctx: &egui::Context,
    sessions: &[SessionMeta],
    error: Option<&str>,
    state: &mut SessionPickerState,
) -> SessionPickerAction {
    let mut action = SessionPickerAction::None;
    let mut open = true;
    // Clamp selection to the visible list each frame.
    if !sessions.is_empty() && state.selected >= sessions.len() {
        state.selected = sessions.len() - 1;
    }

    // Phase 10.10 keynav: handle j/k/Enter/Esc before drawing so the
    // selection / close decision is reflected immediately.
    if let Some(nav_action) = handle_keynav(ctx, sessions, state) {
        action = nav_action;
    }

    egui::Window::new("過去のチャット履歴から再開")
        .open(&mut open)
        .resizable(true)
        .default_width(560.0)
        .default_height(440.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            if let Some(err) = error {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 90, 80),
                    format!("履歴の読込に失敗: {err}"),
                );
                ui.separator();
            }
            if sessions.is_empty() {
                ui.label("このプロジェクトでの過去セッションは見つかりませんでした。");
                return;
            }
            ui.weak("j/k: 移動  Enter: 再開  Esc: 閉じる");
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (idx, session) in sessions.iter().enumerate() {
                    let selected = idx == state.selected;
                    if draw_card(ui, session, selected) {
                        state.selected = idx;
                        action = SessionPickerAction::Resume(session.session_id);
                    }
                }
            });
        });
    if !open && matches!(action, SessionPickerAction::None) {
        action = SessionPickerAction::Close;
    }
    action
}

fn handle_keynav(
    ctx: &egui::Context,
    sessions: &[SessionMeta],
    state: &mut SessionPickerState,
) -> Option<SessionPickerAction> {
    let mut action: Option<SessionPickerAction> = None;
    let events = ctx.input(|i| i.events.clone());
    for event in events {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            continue;
        };
        if modifiers.any() {
            continue;
        }
        match key {
            egui::Key::J | egui::Key::ArrowDown => {
                if state.selected + 1 < sessions.len() {
                    state.selected += 1;
                }
            }
            egui::Key::K | egui::Key::ArrowUp => {
                if state.selected > 0 {
                    state.selected -= 1;
                }
            }
            egui::Key::Enter => {
                if let Some(session) = sessions.get(state.selected) {
                    action = Some(SessionPickerAction::Resume(session.session_id));
                }
            }
            egui::Key::Escape => {
                action = Some(SessionPickerAction::Close);
            }
            _ => {}
        }
    }
    action
}

/// Render one session card. `selected` is true for the row currently
/// highlighted by keynav; both pointer click and `Enter` keystroke
/// commit through the same `SessionPickerAction::Resume` path.
/// Returns `true` when the user clicked this card.
fn draw_card(ui: &mut egui::Ui, session: &SessionMeta, selected: bool) -> bool {
    let mut clicked = false;
    let bg = if selected {
        egui::Color32::from_rgb(70, 90, 110)
    } else {
        egui::Color32::from_rgb(40, 40, 40)
    };
    let frame_resp = egui::Frame::new()
        .fill(bg)
        .inner_margin(egui::Margin::same(8))
        .outer_margin(egui::Margin::symmetric(0, 4))
        .show(ui, |ui| {
            let response = ui.interact(
                ui.available_rect_before_wrap(),
                egui::Id::new(("session_card", session.session_id)),
                egui::Sense::click(),
            );
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(short_id(session.session_id)).strong(),
                        )
                        .selectable(false),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format_modified(session.modified)).weak(),
                            )
                            .selectable(false),
                        );
                    });
                });
                if let Some(preview) = &session.preview {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(preview).color(egui::Color32::LIGHT_GRAY),
                        )
                        .selectable(false)
                        .wrap(),
                    );
                } else {
                    ui.add(
                        egui::Label::new(egui::RichText::new("(プレビューなし)").weak().italics())
                            .selectable(false),
                    );
                }
            });
            if response.clicked() {
                clicked = true;
            }
        });
    if selected {
        frame_resp.response.scroll_to_me(Some(egui::Align::Center));
    }
    clicked
}

fn short_id(id: Uuid) -> String {
    let s = id.to_string();
    s.chars().take(8).collect()
}

/// Format the file mtime as a local-time short stamp like
/// "2026-06-04 23:12". Pure-ish (uses Local timezone via chrono).
fn format_modified(t: SystemTime) -> String {
    let datetime: DateTime<Local> = DateTime::<Local>::from(t);
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_id_takes_first_eight_chars() {
        let id = Uuid::parse_str("8f9abf6d-57ea-4fa0-aa9f-a41c9f965f65").unwrap();
        assert_eq!(short_id(id), "8f9abf6d");
    }
}
