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
    /// Phase 10.19: a line from claude's stderr that looks like an
    /// error message. Surfaces silent failures (missing API key,
    /// auth, etc.) that claude doesn't otherwise put on the JSON
    /// channel — without those, the chat just goes quiet and the
    /// user has no idea why.
    Stderr {
        line: String,
    },
    /// Phase 10.28: streaming tool argument fragment.
    /// `stream_event/content_block_delta/input_json_delta` carries
    /// pieces of the tool's input JSON; the concatenation across
    /// successive deltas forms a valid JSON object.
    ToolInputDelta {
        partial_json: String,
    },
    /// Phase 10.28: a `tool_result` block from a `"user"` event.
    /// claude CLI emits these once it finishes executing a tool;
    /// matched against the corresponding `ToolUse` by `id`.
    ToolResult {
        id: String,
        content: String,
    },
}

/// Translate a single decoded JSON value into a `ChatEvent`. Returns
/// `ChatEvent::Unknown` for any event the parser does not yet handle so the
/// caller has a single, total mapping.
///
/// For multi-event payloads (a `"user"` event containing several
/// `tool_result` blocks) this returns only the first; use
/// [`parse_events`] to get the full list.
pub fn parse_event(value: &Value) -> ChatEvent {
    parse_events(value)
        .into_iter()
        .next()
        .unwrap_or(ChatEvent::Unknown {
            event_type: "<empty>".into(),
        })
}

/// Phase 10.28: a `"user"` event can carry multiple `tool_result`
/// blocks (claude ran several tools in parallel), so the parser
/// needs to be able to emit more than one event per JSON line.
/// Most event types still produce a single-element vec.
pub fn parse_events(value: &Value) -> Vec<ChatEvent> {
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "system" => vec![parse_system(value)],
        "assistant" => vec![parse_assistant(value)],
        "stream_event" => vec![parse_stream_event(value)],
        "user" => parse_user_events(value),
        "result" => vec![ChatEvent::Result {
            subtype: string_field(value, "subtype"),
            total_cost_usd: value.get("total_cost_usd").and_then(Value::as_f64),
            terminal_reason: optional_string_field(value, "terminal_reason"),
        }],
        other => vec![ChatEvent::Unknown {
            event_type: other.to_string(),
        }],
    }
}

/// Walk the `message.content` array of a `"user"` event and emit
/// one `ToolResult` per `tool_result` block. Any non-tool_result
/// content (rare on the user side) is collapsed into a single
/// `Unknown` so the parser doesn't go silent on schema drift.
fn parse_user_events(value: &Value) -> Vec<ChatEvent> {
    let content = value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array);
    let Some(blocks) = content else {
        return vec![ChatEvent::Unknown {
            event_type: "user/no-content".into(),
        }];
    };
    let mut events = Vec::new();
    for block in blocks {
        let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "tool_result" {
            let id = block
                .get("tool_use_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let content = extract_tool_result_content(block.get("content"));
            events.push(ChatEvent::ToolResult { id, content });
        }
    }
    if events.is_empty() {
        events.push(ChatEvent::Unknown {
            event_type: "user/no-tool-result".into(),
        });
    }
    events
}

/// `tool_result.content` is either a plain string (Bash, Read, etc.)
/// or an array of content blocks (text / image). Flatten the array
/// case to a newline-joined string of just the text blocks; image
/// blocks are ignored for now (the chat pane has no inline-image
/// rendering yet).
fn extract_tool_result_content(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| {
                let kind = b.get("type").and_then(Value::as_str).unwrap_or("");
                if kind == "text" {
                    b.get("text").and_then(Value::as_str).map(String::from)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
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
            match delta_type {
                "text_delta" => {
                    let text = delta
                        .and_then(|d| d.get("text"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let uuid = value.get("uuid").and_then(Value::as_str).map(String::from);
                    ChatEvent::TextDelta { uuid, text }
                }
                "input_json_delta" => {
                    let partial_json = delta
                        .and_then(|d| d.get("partial_json"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    ChatEvent::ToolInputDelta { partial_json }
                }
                _ => ChatEvent::Unknown {
                    event_type: format!("stream_event/content_block_delta/{delta_type}"),
                },
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
                    // Phase 10.28: a single JSON line may expand to
                    // multiple events (e.g., `"user"` carrying N
                    // tool_results), so parse via parse_events and
                    // ship each one separately.
                    for event in parse_events(&value) {
                        if let ChatEvent::Unknown { event_type } = &event {
                            tracing::warn!(target: "claude::stdout", "unknown event: {event_type}");
                        }
                        if sender.send(event).is_err() {
                            return;
                        }
                        wake();
                    }
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

/// Phase 10.19: drain claude's stderr. Every line goes to tracing
/// (preserving the pre-10.19 debugging behavior), and lines that
/// look like errors are *also* shipped to `sender` as
/// `ChatEvent::Stderr` so the UI can surface them. Without this,
/// fatal stderr messages (missing API key, auth failure) silently
/// kill the turn and the user only sees an empty assistant bubble.
pub fn pipe_stderr_to_channel<R: BufRead, F: Fn()>(
    reader: R,
    sender: Sender<ChatEvent>,
    wake: F,
) {
    for line in reader.lines() {
        match line {
            Ok(line) => {
                tracing::warn!(target: "claude::stderr", "{line}");
                if looks_like_error(&line) {
                    if sender.send(ChatEvent::Stderr { line }).is_err() {
                        break;
                    }
                    wake();
                }
            }
            Err(err) => {
                tracing::warn!("error reading claude stderr: {err}");
                break;
            }
        }
    }
}

/// Heuristic — true if the stderr line looks like something the
/// user should know about. Case-insensitive substring match on the
/// usual suspects; we err on the side of forwarding to the UI
/// because silent failures are the bug we're fixing here.
pub(crate) fn looks_like_error(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("error")
        || lower.contains("fatal")
        || lower.contains("panic")
        || lower.contains("unauthor")
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

    #[test]
    fn looks_like_error_catches_the_usual_suspects() {
        assert!(looks_like_error("Error: ANTHROPIC_API_KEY is not set"));
        assert!(looks_like_error("error connecting to api.anthropic.com"));
        assert!(looks_like_error("FATAL: out of memory"));
        assert!(looks_like_error("thread 'main' panicked at..."));
        assert!(looks_like_error("HTTP 401 Unauthorized"));
        assert!(looks_like_error("Unauthorised request"));
    }

    #[test]
    fn looks_like_error_skips_innocuous_lines() {
        assert!(!looks_like_error(""));
        assert!(!looks_like_error("starting up..."));
        assert!(!looks_like_error("[info] using model claude-opus"));
        assert!(!looks_like_error("checking auth"));
    }

    #[test]
    fn parses_input_json_delta_as_tool_input_delta() {
        let value = json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "index": 1,
                "delta": {
                    "type": "input_json_delta",
                    "partial_json": "{\"command\":"
                }
            }
        });
        assert_eq!(
            parse_event(&value),
            ChatEvent::ToolInputDelta {
                partial_json: "{\"command\":".into(),
            }
        );
    }

    #[test]
    fn parses_user_tool_result_string_content() {
        let value = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "tu_01abc",
                        "content": "files: a.md b.md\n",
                    }
                ]
            }
        });
        let events = parse_events(&value);
        assert_eq!(
            events,
            vec![ChatEvent::ToolResult {
                id: "tu_01abc".into(),
                content: "files: a.md b.md\n".into(),
            }],
        );
    }

    #[test]
    fn parses_user_tool_result_array_content_joins_text_blocks() {
        let value = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "tu_2",
                        "content": [
                            {"type": "text", "text": "line 1"},
                            {"type": "image", "source": {}},
                            {"type": "text", "text": "line 2"}
                        ]
                    }
                ]
            }
        });
        let events = parse_events(&value);
        assert_eq!(
            events,
            vec![ChatEvent::ToolResult {
                id: "tu_2".into(),
                content: "line 1\nline 2".into(),
            }],
        );
    }

    #[test]
    fn parses_multiple_tool_results_from_one_user_event() {
        let value = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "tu_a", "content": "A"},
                    {"type": "tool_result", "tool_use_id": "tu_b", "content": "B"}
                ]
            }
        });
        let events = parse_events(&value);
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ChatEvent::ToolResult { id, content } if id == "tu_a" && content == "A"
        ));
        assert!(matches!(
            &events[1],
            ChatEvent::ToolResult { id, content } if id == "tu_b" && content == "B"
        ));
    }
}
