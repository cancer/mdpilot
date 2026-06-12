use eframe::egui;

use crate::chat::history::{ChatHistory, ChatMessage, SystemMessage, ToolBlock};

/// Stable id for the chat prompt `TextEdit`. The `App` reads this id
/// in Phase 10.2 to suppress vim-engine event dispatch while the
/// user is typing in the chat input.
pub fn chat_input_id() -> egui::Id {
    egui::Id::new("chat_prompt_input")
}

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
/// The Send / Abort button swaps based on `history.in_flight`. When a
/// turn is in flight, the Abort button (and Esc while the chat owns
/// focus) kills the claude child via `Tab::abort_current_turn`, which
/// re-spawns with `--resume` so the same conversation can continue.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    history: &mut ChatHistory,
    session_alive: bool,
    on_send: &mut dyn FnMut(String),
    on_abort: &mut dyn FnMut(),
) {
    let frame_id = ui.id().with("chat_pane_root");
    egui::Panel::bottom(frame_id.with("input"))
        .resizable(false)
        .min_size(72.0)
        .show_inside(ui, |ui| {
            input_row(ui, history, session_alive, on_send, on_abort);
        });

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            if history.messages.is_empty() {
                // Empty-state hint isn't message content, so opt out of
                // selection — keeps the F-05 "no default dependence" claim
                // honest for every label this view renders.
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Claude にメッセージを送ると、ここに会話履歴が表示されます。",
                        )
                        .weak(),
                    )
                    .selectable(false),
                );
                return;
            }
            for message in &history.messages {
                render_message(ui, message);
                ui.add_space(8.0);
            }
            // Phase 10.26: one-shot scroll-to-bottom after the user
            // submits. `stick_to_bottom` only holds the bottom if
            // already there; if the user had scrolled up to read
            // backlog, sending a new prompt should still pull the
            // view back down so they see what they just said.
            if history.scroll_to_bottom_pending {
                ui.scroll_to_cursor(Some(egui::Align::Max));
                history.scroll_to_bottom_pending = false;
            }
        });
}

fn input_row(
    ui: &mut egui::Ui,
    history: &mut ChatHistory,
    session_alive: bool,
    on_send: &mut dyn FnMut(String),
    on_abort: &mut dyn FnMut(),
) {
    ui.horizontal_top(|ui| {
        let in_flight = history.in_flight;
        // Phase 10.25: 送信ボタンを撤廃。中断は in_flight のとき
        // だけ右側に出す。idle 中はボタン領域もゼロにして input が
        // 横幅をフル活用できるようにする。
        let buttons_width = if in_flight { 88.0 } else { 0.0 };
        let input_width = (ui.available_width() - buttons_width).max(160.0);

        // Phase 10.21: Consume Enter / Esc BEFORE the TextEdit
        // renders. egui's TextEdit reads from `i.events` during
        // `ui.add`, so removing the events *after* the add was a
        // no-op — the Text("\n") had already been written into
        // `history.input`. Pre-consume here, then render.
        // Focus check via `ctx.memory()` instead of `editor.has_focus()`
        // because we don't have the Response yet.
        let chat_focused = ui
            .ctx()
            .memory(|m| m.focused() == Some(chat_input_id()));
        let mut enter_pressed = false;
        let mut abort_via_key = false;
        if chat_focused {
            enter_pressed = ui.input_mut(|i| extract_send_enter(&mut i.events));
            if in_flight {
                abort_via_key = ui.input_mut(|i| extract_abort_escape(&mut i.events));
            }
        }

        let editor = ui.add_sized(
            [input_width, 60.0],
            egui::TextEdit::multiline(&mut history.input)
                .id(chat_input_id())
                .desired_rows(2)
                .hint_text("プロンプトを入力… (Enter で送信、Shift+Enter で改行)"),
        );

        // Phase 10.25: in_flight でも送信可能にする。claude CLI が
        // 同一セッションの stdin への追加 user message を受け付ける
        // 想定。デルタの混線リスクは認識しているがユーザーの
        // 「送信も中断もどちらも可能であるべき」指示を優先。
        let can_send = session_alive && !history.input.trim().is_empty();
        let submit = enter_pressed && can_send;

        if in_flight {
            ui.vertical(|ui| {
                if ui
                    .button("中断")
                    .on_hover_text("Esc でも中断できます")
                    .clicked()
                {
                    abort_via_key = true;
                }
            });
        }

        if submit {
            let text = history.input.trim().to_string();
            history.input.clear();
            history.in_flight = true;
            history.scroll_to_bottom_pending = true;
            editor.request_focus();
            on_send(text);
        } else if abort_via_key {
            history.in_flight = false;
            editor.request_focus();
            on_abort();
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
    // Bug fix (2026-06-11): winit / egui delivers Enter as BOTH an
    // `Event::Key(Enter)` and an `Event::Text("\n")`. We consumed
    // the Key above, but the Text leaked through to the TextEdit
    // and inserted a literal newline into the prompt. Drop the
    // companion Text("\n") on the same frame we decided to send.
    // Shift+Enter doesn't hit this path because the Key branch
    // above leaves `send = false`, so the Text("\n") stays in queue
    // and the TextEdit performs the expected newline insertion.
    if send {
        events.retain(|event| !matches!(event, egui::Event::Text(t) if t == "\n"));
    }
    send
}

/// Phase 10.14: consume Esc from the chat input event queue when a
/// turn is in flight. Returns true iff one such press was found.
/// Caller (input_row) only invokes us when the chat owns focus, so
/// this won't swallow Esc that belongs to other widgets (modal
/// dismiss, vim engine, …).
pub(crate) fn extract_abort_escape(events: &mut Vec<egui::Event>) -> bool {
    let mut abort = false;
    events.retain(|event| match event {
        egui::Event::Key {
            key: egui::Key::Escape,
            pressed: true,
            ..
        } => {
            abort = true;
            false
        }
        _ => true,
    });
    abort
}

fn render_message(ui: &mut egui::Ui, message: &ChatMessage) {
    let dark = ui.style().visuals.dark_mode;
    match message {
        ChatMessage::User { text } => {
            // Phase 10.18: User messages get a subtle tinted Frame
            // and a muted text color so Assistant's reply (which
            // keeps the high-contrast body_color and the explicit
            // "Assistant" header) is the visually dominant element.
            // No header label here — the tint itself signals "this
            // is User's turn", and dropping the text label keeps
            // User maximally lightweight per the contrast direction.
            egui::Frame::new()
                .fill(user_bubble_bg(dark))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(text).color(user_text_color(dark)),
                        )
                        .selectable(true),
                    );
                });
        }
        ChatMessage::Assistant {
            text,
            tools,
            message_id: _,
        } => {
            // Phase 10.24: drop the "Assistant" header too. The
            // User bubble's tint already establishes the speaker
            // contrast — Assistant is the default "plain text +
            // high contrast" branch, so no label is needed.
            if !text.is_empty() {
                ui.add(body_label(text, dark));
            }
            for tool in tools {
                render_tool(ui, tool);
            }
        }
        ChatMessage::System(system) => render_system(ui, system),
    }
}

/// Phase 10.18: subtle background tint for the User bubble. Alpha is
/// kept low so the Frame reads as "different region" rather than
/// "callout box" — Assistant should stay the focal point.
fn user_bubble_bg(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 14)
    } else {
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 12)
    }
}

/// Phase 10.18: deliberately lower-contrast than `body_color` so the
/// User block recedes relative to Assistant. Sits roughly midway
/// between the panel background and Assistant's body text.
fn user_text_color(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_gray(170)
    } else {
        egui::Color32::from_gray(90)
    }
}

/// Message body text — explicitly selectable so the F-05 contract doesn't
/// depend on egui's `style.interaction.selectable_labels = true` default
/// staying true in a future version (or a user theme).
///
/// 2026-06-11: body text was hard to read because egui's default
/// `widget_text_color` blended into the dark background. We pick a
/// near-white color in dark mode (and near-black in light) so the
/// transcript reads clearly without overpowering tool / status
/// blocks (which keep their own colors).
fn body_label(text: &str, dark: bool) -> egui::Label {
    egui::Label::new(egui::RichText::new(text).color(body_color(dark))).selectable(true)
}

/// High-contrast body text color for the current theme.
fn body_color(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_gray(240)
    } else {
        egui::Color32::from_gray(20)
    }
}

fn render_tool(ui: &mut egui::Ui, tool: &ToolBlock) {
    egui::CollapsingHeader::new(format!("⚙ {}", tool.name))
        .id_salt(format!("tool_{}", tool.id))
        .default_open(false)
        .show(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new("Input").weak()).selectable(false));
            ui.add(
                egui::Label::new(egui::RichText::new(format!("{:#}", tool.input)).code())
                    .selectable(true),
            );
            if let Some(output) = &tool.output {
                ui.add(egui::Label::new(egui::RichText::new("Output").weak()).selectable(false));
                ui.add(egui::Label::new(egui::RichText::new(output).code()).selectable(true));
            } else {
                ui.add(
                    egui::Label::new(egui::RichText::new("(出力待ち…)").weak()).selectable(false),
                );
            }
        });
}

fn render_system(ui: &mut egui::Ui, system: &SystemMessage) {
    // System messages (errors, retries) stay selectable so the user can
    // copy error text into a bug report or search.
    match system {
        SystemMessage::ApiRetry {
            attempt,
            max_retries,
            error,
        } => {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "API リトライ中: {error} ({attempt}/{max_retries})"
                    ))
                    .color(egui::Color32::from_rgb(220, 180, 70)),
                )
                .selectable(true),
            );
        }
        SystemMessage::ResultError { subtype } => {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "Claude のレスポンスがエラーで終了しました: {subtype}"
                    ))
                    .color(egui::Color32::from_rgb(220, 90, 80)),
                )
                .selectable(true),
            );
        }
        SystemMessage::Disconnected => {
            ui.add(
                egui::Label::new(
                    egui::RichText::new("Claude セッションが切断されました。")
                        .color(egui::Color32::from_rgb(220, 90, 80)),
                )
                .selectable(true),
            );
        }
        SystemMessage::StderrError { line } => {
            // Phase 10.19: claude wrote something error-looking to
            // stderr. Render in the same red as ResultError but
            // keep a `claude stderr:` prefix so the user can tell
            // it's not a JSON-channel error.
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("claude stderr: {line}"))
                        .color(egui::Color32::from_rgb(220, 90, 80)),
                )
                .selectable(true),
            );
        }
        SystemMessage::SpawnFailed { error } => {
            // Phase 8.1 (2026-06-11): claude CLI が見つからないケース
            // (Finder 起動 + PATH 不足 / 未インストール) はユーザー側で
            // 解決が必要なので、行動可能なメッセージに改善する。
            let not_found = error.contains("No such file or directory")
                || error.contains("not found")
                || error.contains("os error 2");
            let body = if not_found {
                format!(
                    "Claude CLI が見つかりませんでした ({error})。\n\
                     mdpilot は別途インストールした `claude` コマンドを呼び出します。\n\
                     インストール手順: https://docs.claude.com/claude-code\n\
                     ターミナルで `which claude` が成功することを確認し、mdpilot を再起動してください。"
                )
            } else {
                format!("Claude を起動できませんでした: {error}")
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(body).color(egui::Color32::from_rgb(220, 90, 80)),
                )
                .selectable(true),
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
