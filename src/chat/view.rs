use eframe::egui;

use crate::chat::history::{ChatHistory, ChatMessage, SystemMessage, ToolBlock};

/// Render the chat pane: message history scroll area on top, prompt input
/// and Send/Cancel buttons at the bottom.
///
/// `session_alive` gates the Send button — false means we already pushed a
/// `SpawnFailed` banner at startup and there is no claude to talk to, so
/// the user shouldn't be able to enqueue messages that would just produce
/// a second error line.
///
/// `on_send` runs with the trimmed prompt text when the user submits — Send
/// button click or plain Enter. The view stays decoupled from the session
/// module: the callback owns the stdin write and history update.
///
/// The 中断 button is rendered but permanently disabled: claude CLI 2.1
/// does not expose a mid-turn interrupt over stdin (see `docs/chat.md` §10
/// and GitHub issue anthropics/claude-code#41665, closed as duplicate).
/// We show the button for visual continuity and explain via tooltip.
pub fn show(
    ui: &mut egui::Ui,
    history: &mut ChatHistory,
    session_alive: bool,
    on_send: &mut dyn FnMut(String),
) {
    let frame_id = ui.id().with("chat_pane_root");
    egui::Panel::bottom(frame_id.with("input"))
        .resizable(false)
        .min_size(72.0)
        .show_inside(ui, |ui| {
            input_row(ui, history, session_alive, on_send);
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

fn input_row(
    ui: &mut egui::Ui,
    history: &mut ChatHistory,
    session_alive: bool,
    on_send: &mut dyn FnMut(String),
) {
    ui.horizontal_top(|ui| {
        let buttons_width = 88.0;
        let input_width = (ui.available_width() - buttons_width).max(160.0);
        let editor = ui.add_sized(
            [input_width, 60.0],
            egui::TextEdit::multiline(&mut history.input)
                .desired_rows(2)
                .hint_text("プロンプトを入力… (Enter で送信、Shift+Enter で改行)"),
        );

        let can_send = session_alive && !history.input.trim().is_empty();

        // Plain Enter submits; Shift+Enter (or any modified Enter) falls
        // through to the TextEdit so the user gets a literal newline. We
        // consume the event before TextEdit sees it; otherwise we'd insert
        // the newline and then send a message ending with `\n`.
        //
        // Per egui's input contract (egui::Event::Key doc), key events that
        // were processed by an IME are *not* delivered. That means while
        // the IME is composing kana → kanji, the Enter that confirms
        // composition never appears in this event queue, so we don't need a
        // separate "composing" guard here.
        let mut submit = false;
        if editor.has_focus() && ui.input_mut(|i| extract_send_enter(&mut i.events)) && can_send {
            submit = true;
        }

        ui.vertical(|ui| {
            if ui
                .add_enabled(can_send, egui::Button::new("送信"))
                .clicked()
            {
                submit = true;
            }
            // 中断 is permanently disabled in MVP: claude CLI 2.1 has no
            // mid-turn interrupt over stdin (see docs/chat.md §10). We keep
            // the button so the layout is stable when upstream lands the
            // feature; the tooltip tells the user why it's greyed out.
            ui.add_enabled(false, egui::Button::new("中断"))
                .on_disabled_hover_text(
                    "Claude CLI 2.1 は応答中の中断に対応していません。\
                     応答完了までお待ちください。",
                );
        });

        if submit {
            let text = history.input.trim().to_string();
            history.input.clear();
            // Keep focus on the input so the user can keep typing without
            // clicking back into the field.
            editor.request_focus();
            on_send(text);
        }
    });
}

/// Extract a plain-Enter press from the event queue, consuming it so the
/// TextEdit beneath does not also insert a newline. Returns true iff at
/// least one such event was found.
///
/// Shift / Ctrl / Cmd / Alt + Enter are left in the queue: Shift+Enter falls
/// through to the TextEdit for a literal newline; the other combos are
/// reserved for future shortcuts (e.g. Cmd+Enter "force send" if we ever
/// want that).
pub(crate) fn extract_send_enter(events: &mut Vec<egui::Event>) -> bool {
    let mut send = false;
    events.retain(|event| match event {
        egui::Event::Key {
            key: egui::Key::Enter,
            pressed: true,
            modifiers,
            ..
        } if !modifiers.shift
            && !modifiers.ctrl
            && !modifiers.command
            && !modifiers.mac_cmd
            && !modifiers.alt =>
        {
            send = true;
            false
        }
        _ => true,
    });
    send
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
        SystemMessage::SpawnFailed { error } => {
            ui.colored_label(
                egui::Color32::from_rgb(220, 90, 80),
                format!("Claude を起動できませんでした: {error}"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn plain_enter_is_consumed_and_signals_send() {
        let mut events = vec![key(egui::Modifiers::NONE)];
        assert!(extract_send_enter(&mut events));
        assert!(events.is_empty(), "plain Enter must be consumed");
    }

    #[test]
    fn shift_enter_is_left_for_textedit() {
        let mut events = vec![key(egui::Modifiers::SHIFT)];
        assert!(!extract_send_enter(&mut events));
        assert_eq!(events.len(), 1, "Shift+Enter must fall through");
    }

    #[test]
    fn ctrl_enter_is_left_alone() {
        let mut events = vec![key(egui::Modifiers::CTRL)];
        assert!(!extract_send_enter(&mut events));
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn cmd_enter_is_left_alone() {
        let mut events = vec![key(egui::Modifiers::COMMAND)];
        assert!(!extract_send_enter(&mut events));
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn alt_enter_is_left_alone() {
        let mut events = vec![key(egui::Modifiers::ALT)];
        assert!(!extract_send_enter(&mut events));
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn enter_release_is_not_a_send() {
        let mut events = vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }];
        assert!(!extract_send_enter(&mut events));
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn non_enter_keys_are_untouched() {
        let mut events = vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }];
        assert!(!extract_send_enter(&mut events));
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn mixed_event_queue_only_strips_plain_enter() {
        let mut events = vec![
            egui::Event::Text("a".into()),
            key(egui::Modifiers::SHIFT),
            key(egui::Modifiers::NONE),
            egui::Event::Text("b".into()),
        ];
        assert!(extract_send_enter(&mut events));
        assert_eq!(events.len(), 3);
        // The remaining Key event must be the Shift+Enter one.
        let key_count = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(key_count, 1);
    }
}
