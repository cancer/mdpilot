//! Phase 9.X.2: modal picker that lets the user resume a past
//! claude session.
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

pub fn show(
    ctx: &egui::Context,
    sessions: &[SessionMeta],
    error: Option<&str>,
) -> SessionPickerAction {
    let mut action = SessionPickerAction::None;
    let mut open = true;
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
                // Phase 9.X.3: 履歴ピッカーは mdpilot が preview を
                // 記録した session に限定しているため、claude
                // 自体には履歴があっても 0 件になる場合がある
                // (旧ビルドで作られた session など)。
                ui.label(
                    "復元可能なセッションは見つかりませんでした。\
                     一度起動して .md を開いたセッションが対象です。",
                );
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for session in sessions {
                    if draw_card(ui, session) {
                        action = SessionPickerAction::Resume(session.session_id);
                    }
                }
            });
        });
    if !open {
        // Window's built-in X button was clicked.
        action = SessionPickerAction::Close;
    }
    action
}

/// Render one session card. Returns `true` when the user clicked
/// on it (modal should resume that session).
fn draw_card(ui: &mut egui::Ui, session: &SessionMeta) -> bool {
    let mut clicked = false;
    egui::Frame::new()
        .fill(egui::Color32::from_rgb(40, 40, 40))
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
