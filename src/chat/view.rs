use eframe::egui;

use crate::chat::history::{ChatHistory, ChatMessage, SystemMessage, ToolBlock};

/// Render the chat pane: message history scroll area on top, prompt input
/// and Send/Cancel buttons at the bottom. The button callbacks become real
/// in Phase 3.2 (send) and Phase 3.6 (cancel).
pub fn show(ui: &mut egui::Ui, history: &mut ChatHistory) {
    let frame_id = ui.id().with("chat_pane_root");
    egui::Panel::bottom(frame_id.with("input"))
        .resizable(false)
        .min_size(72.0)
        .show_inside(ui, |ui| {
            input_row(ui, history);
        });

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            if history.messages.is_empty() {
                ui.weak("Claude にメッセージを送ると、ここに会話履歴が表示されます。");
                return;
            }
            for message in &history.messages {
                render_message(ui, message);
                ui.add_space(8.0);
            }
        });
}

fn input_row(ui: &mut egui::Ui, history: &mut ChatHistory) {
    ui.horizontal_top(|ui| {
        let buttons_width = 88.0;
        let input_width = (ui.available_width() - buttons_width).max(160.0);
        ui.add_sized(
            [input_width, 60.0],
            egui::TextEdit::multiline(&mut history.input)
                .desired_rows(2)
                .hint_text("プロンプトを入力… (Enter で送信、Shift+Enter で改行)"),
        );
        ui.vertical(|ui| {
            let can_send = !history.input.trim().is_empty();
            if ui
                .add_enabled(can_send, egui::Button::new("送信"))
                .clicked()
            {
                // Phase 3.2: forward history.input to ChatSession::send_user_message.
            }
            if ui.button("中断").clicked() {
                // Phase 3.6: abort an in-flight response.
            }
        });
    });
}

fn render_message(ui: &mut egui::Ui, message: &ChatMessage) {
    match message {
        ChatMessage::User { text } => {
            ui.label(egui::RichText::new("User").strong());
            ui.label(text);
        }
        ChatMessage::Assistant {
            text,
            tools,
            message_id: _,
        } => {
            ui.label(egui::RichText::new("Assistant").strong());
            if !text.is_empty() {
                ui.label(text);
            }
            for tool in tools {
                render_tool(ui, tool);
            }
        }
        ChatMessage::System(system) => render_system(ui, system),
    }
}

fn render_tool(ui: &mut egui::Ui, tool: &ToolBlock) {
    egui::CollapsingHeader::new(format!("⚙ {}", tool.name))
        .id_salt(format!("tool_{}", tool.id))
        .default_open(false)
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Input").weak());
            ui.code(format!("{:#}", tool.input));
            if let Some(output) = &tool.output {
                ui.label(egui::RichText::new("Output").weak());
                ui.code(output);
            } else {
                ui.weak("(出力待ち…)");
            }
        });
}

fn render_system(ui: &mut egui::Ui, system: &SystemMessage) {
    match system {
        SystemMessage::ApiRetry {
            attempt,
            max_retries,
            error,
        } => {
            ui.colored_label(
                egui::Color32::from_rgb(220, 180, 70),
                format!("API リトライ中: {error} ({attempt}/{max_retries})"),
            );
        }
        SystemMessage::ResultError { subtype } => {
            ui.colored_label(
                egui::Color32::from_rgb(220, 90, 80),
                format!("Claude のレスポンスがエラーで終了しました: {subtype}"),
            );
        }
        SystemMessage::Disconnected => {
            ui.colored_label(
                egui::Color32::from_rgb(220, 90, 80),
                "Claude セッションが切断されました。",
            );
        }
    }
}
