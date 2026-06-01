// Wired into ChatSession in Phase 2.5 / 2.7. Until then the API surface
// looks dead from the bin crate's perspective.
#![allow(dead_code)]

//! stream-json event extraction for `claude --output-format=stream-json`.
//!
//! Per `docs/chat.md` §3.2, mdpilot **does not** type the event payload
//! with `serde::Deserialize` because the actual schema (especially
//! `system/init`) carries 20+ fields that drift with claude versions.
//! Instead we walk `serde_json::Value` and pull out the 5–6 fields we
//! actually consume. Unknown event types are passed through as
//! `ChatEvent::Unknown` so the caller can log them and move on.

use std::io::BufRead;
use std::sync::mpsc::Sender;

use serde_json::Value;

/// Semantic events the UI consumes. The variant set covers everything the
/// Phase 2.0 / 2.2 probes observed, plus the partial-message stream_event
/// variants documented in agent-sdk/streaming-output.md.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    Init {
        session_id: String,
    },
    AssistantMessage {
        text: String,
        message_id: Option<String>,
    },
    TextDelta {
        uuid: Option<String>,
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ApiRetry {
        attempt: u64,
        max_retries: u64,
        error: String,
    },
    Result {
        subtype: String,
        total_cost_usd: Option<f64>,
        terminal_reason: Option<String>,
    },
    Unknown {
        event_type: String,
    },
}

/// Translate a single decoded JSON value into a `ChatEvent`. Returns
/// `ChatEvent::Unknown` for any event the parser does not yet handle so the
/// caller has a single, total mapping.
pub fn parse_event(value: &Value) -> ChatEvent {
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "system" => parse_system(value),
        "assistant" => parse_assistant(value),
        "stream_event" => parse_stream_event(value),
        "result" => ChatEvent::Result {
            subtype: string_field(value, "subtype"),
            total_cost_usd: value.get("total_cost_usd").and_then(Value::as_f64),
            terminal_reason: optional_string_field(value, "terminal_reason"),
        },
        other => ChatEvent::Unknown {
            event_type: other.to_string(),
        },
    }
}

fn parse_system(value: &Value) -> ChatEvent {
    let subtype = value.get("subtype").and_then(Value::as_str).unwrap_or("");
    match subtype {
        "init" => ChatEvent::Init {
            session_id: string_field(value, "session_id"),
        },
        "api_retry" => ChatEvent::ApiRetry {
            attempt: value.get("attempt").and_then(Value::as_u64).unwrap_or(0),
            max_retries: value
                .get("max_retries")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            error: string_field(value, "error"),
        },
        other => ChatEvent::Unknown {
            event_type: format!("system/{other}"),
        },
    }
}

fn parse_assistant(value: &Value) -> ChatEvent {
    let message = value.get("message");
    let text = message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .and_then(|arr| {
            arr.iter().find_map(|block| {
                let kind = block.get("type").and_then(Value::as_str)?;
                if kind == "text" {
                    block.get("text").and_then(Value::as_str).map(String::from)
                } else {
                    None
                }
            })
        })
        .unwrap_or_default();
    let message_id = message
        .and_then(|m| m.get("id"))
        .and_then(Value::as_str)
        .map(String::from);
    ChatEvent::AssistantMessage { text, message_id }
}

fn parse_stream_event(value: &Value) -> ChatEvent {
    let event = value.get("event");
    let inner_type = event
        .and_then(|e| e.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");
    match inner_type {
        "content_block_delta" => {
            let delta = event.and_then(|e| e.get("delta"));
            let delta_type = delta
                .and_then(|d| d.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if delta_type == "text_delta" {
                let text = delta
                    .and_then(|d| d.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let uuid = value.get("uuid").and_then(Value::as_str).map(String::from);
                ChatEvent::TextDelta { uuid, text }
            } else {
                ChatEvent::Unknown {
                    event_type: format!("stream_event/content_block_delta/{delta_type}"),
                }
            }
        }
        "content_block_start" => {
            let block = event.and_then(|e| e.get("content_block"));
            let block_type = block
                .and_then(|b| b.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if block_type == "tool_use" {
                ChatEvent::ToolUse {
                    id: block
                        .and_then(|b| b.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    name: block
                        .and_then(|b| b.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    input: block
                        .and_then(|b| b.get("input"))
                        .cloned()
                        .unwrap_or(Value::Null),
                }
            } else {
                ChatEvent::Unknown {
                    event_type: format!("stream_event/content_block_start/{block_type}"),
                }
            }
        }
        other => ChatEvent::Unknown {
            event_type: format!("stream_event/{other}"),
        },
    }
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn optional_string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(String::from)
}

/// Read JSON Lines off `reader` and forward each parsed event into `sender`.
/// Blank lines are skipped; invalid JSON is logged via tracing and skipped.
/// Returns when the reader hits EOF or the receiver is dropped.
///
/// `wake` runs after every successful `sender.send(...)`. App passes a
/// closure that calls `egui::Context::request_repaint` so the UI thread
/// notices the new event without waiting for the next OS-level input.
pub fn pipe_stdout_to_channel<R: BufRead, F: Fn()>(reader: R, sender: Sender<ChatEvent>, wake: F) {
    for line in reader.lines() {
        match line {
            Ok(line) if line.is_empty() => continue,
            Ok(line) => match serde_json::from_str::<Value>(&line) {
                Ok(value) => {
                    let event = parse_event(&value);
                    if let ChatEvent::Unknown { event_type } = &event {
                        tracing::warn!(target: "claude::stdout", "unknown event: {event_type}");
                    }
                    if sender.send(event).is_err() {
                        break;
                    }
                    wake();
                }
                Err(err) => {
                    tracing::warn!(
                        target: "claude::stdout",
                        "failed to parse line: {err}; line: {line:?}",
                    );
                }
            },
            Err(err) => {
                tracing::warn!(target: "claude::stdout", "read error: {err}");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_system_init_session_id_only() {
        let value = json!({
            "type": "system",
            "subtype": "init",
            "cwd": "/tmp",
            "session_id": "abc-123",
            "tools": ["Bash"],
            "model": "claude-opus",
            "unused_future_field": 42,
        });
        let event = parse_event(&value);
        assert_eq!(
            event,
            ChatEvent::Init {
                session_id: "abc-123".into()
            }
        );
    }

    #[test]
    fn parses_assistant_message_text_only() {
        let value = json!({
            "type": "assistant",
            "message": {
                "id": "msg_01",
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Hi there!"},
                ],
            },
        });
        let event = parse_event(&value);
        assert_eq!(
            event,
            ChatEvent::AssistantMessage {
                text: "Hi there!".into(),
                message_id: Some("msg_01".into()),
            }
        );
    }

    #[test]
    fn parses_text_delta() {
        let value = json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "text_delta", "text": "partial"},
            },
            "uuid": "u-1",
        });
        assert_eq!(
            parse_event(&value),
            ChatEvent::TextDelta {
                uuid: Some("u-1".into()),
                text: "partial".into()
            }
        );
    }

    #[test]
    fn parses_tool_use() {
        let value = json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_start",
                "content_block": {
                    "type": "tool_use",
                    "id": "tu_1",
                    "name": "Edit",
                    "input": {"file_path": "/tmp/x.md"},
                },
            },
        });
        let event = parse_event(&value);
        match event {
            ChatEvent::ToolUse { id, name, input } => {
                assert_eq!(id, "tu_1");
                assert_eq!(name, "Edit");
                assert_eq!(input["file_path"], "/tmp/x.md");
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn parses_api_retry() {
        let value = json!({
            "type": "system",
            "subtype": "api_retry",
            "attempt": 2,
            "max_retries": 3,
            "error": "rate_limit",
        });
        assert_eq!(
            parse_event(&value),
            ChatEvent::ApiRetry {
                attempt: 2,
                max_retries: 3,
                error: "rate_limit".into()
            }
        );
    }

    #[test]
    fn parses_result_success() {
        let value = json!({
            "type": "result",
            "subtype": "success",
            "total_cost_usd": 0.05,
            "terminal_reason": "completed",
        });
        assert_eq!(
            parse_event(&value),
            ChatEvent::Result {
                subtype: "success".into(),
                total_cost_usd: Some(0.05),
                terminal_reason: Some("completed".into()),
            }
        );
    }

    #[test]
    fn unknown_event_carries_qualified_type() {
        let value = json!({"type": "something_new"});
        assert_eq!(
            parse_event(&value),
            ChatEvent::Unknown {
                event_type: "something_new".into()
            }
        );
    }

    #[test]
    fn unknown_system_subtype_is_qualified() {
        let value = json!({"type": "system", "subtype": "hook_started"});
        assert_eq!(
            parse_event(&value),
            ChatEvent::Unknown {
                event_type: "system/hook_started".into()
            }
        );
    }

    #[test]
    fn pipe_forwards_events_until_eof() {
        let payload = b"{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s1\"}\n\
                        {\"type\":\"result\",\"subtype\":\"success\"}\n";
        let (tx, rx) = std::sync::mpsc::channel();
        pipe_stdout_to_channel(std::io::Cursor::new(&payload[..]), tx, || {});
        let received: Vec<ChatEvent> = rx.iter().collect();
        assert_eq!(received.len(), 2);
        assert!(matches!(received[0], ChatEvent::Init { .. }));
        assert!(matches!(received[1], ChatEvent::Result { .. }));
    }

    #[test]
    fn pipe_skips_blank_and_garbage_lines() {
        let payload = b"\n\
                        not json\n\
                        {\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s1\"}\n";
        let (tx, rx) = std::sync::mpsc::channel();
        pipe_stdout_to_channel(std::io::Cursor::new(&payload[..]), tx, || {});
        let received: Vec<ChatEvent> = rx.iter().collect();
        assert_eq!(received.len(), 1);
        assert!(matches!(received[0], ChatEvent::Init { .. }));
    }

    #[test]
    fn pipe_invokes_wake_once_per_forwarded_event() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let payload = b"\n\
                        garbage\n\
                        {\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s1\"}\n\
                        {\"type\":\"result\",\"subtype\":\"success\"}\n";
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        pipe_stdout_to_channel(std::io::Cursor::new(&payload[..]), tx, move || {
            calls_clone.fetch_add(1, Ordering::SeqCst);
        });
        let received: Vec<ChatEvent> = rx.iter().collect();
        assert_eq!(received.len(), 2);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "wake should fire once per successful send, not on blank/garbage lines",
        );
    }
}
