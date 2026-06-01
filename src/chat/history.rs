// History gets fed real ChatEvents in Phase 3.2; until then most of the
// constructors are unused from the bin crate.
#![allow(dead_code)]

//! In-memory chat history rendered by `crate::chat::view`.

use serde_json::Value;

#[derive(Debug, Default, Clone)]
pub struct ChatHistory {
    pub messages: Vec<ChatMessage>,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatMessage {
    User {
        text: String,
    },
    Assistant {
        /// `id` field from the stream-json `assistant` event (claude's own
        /// message id, e.g. `msg_01...`).
        message_id: Option<String>,
        text: String,
        tools: Vec<ToolBlock>,
    },
    System(SystemMessage),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolBlock {
    /// claude's tool_use id (e.g. `tu_01...`).
    pub id: String,
    pub name: String,
    pub input: Value,
    /// Filled in once a `tool_result` arrives for this id.
    pub output: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SystemMessage {
    ApiRetry {
        attempt: u64,
        max_retries: u64,
        error: String,
    },
    /// `result.subtype != "success"` — claude finished but reported an error.
    ResultError { subtype: String },
    /// claude child process exited while a response was in flight.
    Disconnected,
}

impl ChatHistory {
    pub fn push_user(&mut self, text: impl Into<String>) {
        self.messages.push(ChatMessage::User { text: text.into() });
    }

    /// Append a freshly-started assistant message that will be filled by
    /// subsequent `TextDelta` events.
    pub fn start_assistant(&mut self, message_id: Option<String>) {
        self.messages.push(ChatMessage::Assistant {
            message_id,
            text: String::new(),
            tools: Vec::new(),
        });
    }

    /// Append text to the most recent assistant message, creating a new one
    /// if there isn't one to extend.
    pub fn append_assistant_text(&mut self, text: &str) {
        if let Some(ChatMessage::Assistant { text: buf, .. }) = self.messages.last_mut() {
            buf.push_str(text);
        } else {
            self.messages.push(ChatMessage::Assistant {
                message_id: None,
                text: text.to_string(),
                tools: Vec::new(),
            });
        }
    }

    /// Replace the most recent assistant message body (used for the full
    /// `assistant` event when `--include-partial-messages` is off).
    pub fn replace_assistant_text(&mut self, message_id: Option<String>, text: String) {
        if let Some(ChatMessage::Assistant {
            text: buf,
            message_id: id,
            ..
        }) = self.messages.last_mut()
        {
            *buf = text;
            *id = message_id;
        } else {
            self.messages.push(ChatMessage::Assistant {
                message_id,
                text,
                tools: Vec::new(),
            });
        }
    }

    /// Attach a tool_use block to the most recent assistant message.
    pub fn push_tool_use(&mut self, block: ToolBlock) {
        if let Some(ChatMessage::Assistant { tools, .. }) = self.messages.last_mut() {
            tools.push(block);
        } else {
            // tool_use before any assistant message — start one to host it.
            self.messages.push(ChatMessage::Assistant {
                message_id: None,
                text: String::new(),
                tools: vec![block],
            });
        }
    }

    /// Populate `output` on the matching tool_use id, walking the most recent
    /// assistant message first.
    pub fn record_tool_result(&mut self, id: &str, output: String) {
        for message in self.messages.iter_mut().rev() {
            if let ChatMessage::Assistant { tools, .. } = message {
                if let Some(tool) = tools.iter_mut().rev().find(|t| t.id == id) {
                    tool.output = Some(output);
                    return;
                }
            }
        }
    }

    pub fn push_system(&mut self, system: SystemMessage) {
        self.messages.push(ChatMessage::System(system));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn append_to_empty_starts_a_new_assistant_message() {
        let mut h = ChatHistory::default();
        h.append_assistant_text("hello");
        assert_eq!(h.messages.len(), 1);
        match &h.messages[0] {
            ChatMessage::Assistant { text, tools, .. } => {
                assert_eq!(text, "hello");
                assert!(tools.is_empty());
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn append_continues_existing_assistant_message() {
        let mut h = ChatHistory::default();
        h.start_assistant(Some("msg_1".into()));
        h.append_assistant_text("partial ");
        h.append_assistant_text("end");
        let last = h.messages.last().unwrap();
        match last {
            ChatMessage::Assistant { text, .. } => assert_eq!(text, "partial end"),
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn replace_writes_into_existing_assistant_message() {
        let mut h = ChatHistory::default();
        h.start_assistant(None);
        h.replace_assistant_text(Some("msg_2".into()), "final text".into());
        match h.messages.last().unwrap() {
            ChatMessage::Assistant {
                text, message_id, ..
            } => {
                assert_eq!(text, "final text");
                assert_eq!(message_id.as_deref(), Some("msg_2"));
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn tool_use_attaches_to_current_assistant() {
        let mut h = ChatHistory::default();
        h.start_assistant(None);
        h.push_tool_use(ToolBlock {
            id: "tu_1".into(),
            name: "Edit".into(),
            input: json!({"file_path": "x.md"}),
            output: None,
        });
        match h.messages.last().unwrap() {
            ChatMessage::Assistant { tools, .. } => {
                assert_eq!(tools.len(), 1);
                assert_eq!(tools[0].id, "tu_1");
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn tool_result_fills_matching_tool_use() {
        let mut h = ChatHistory::default();
        h.start_assistant(None);
        h.push_tool_use(ToolBlock {
            id: "tu_1".into(),
            name: "Edit".into(),
            input: Value::Null,
            output: None,
        });
        h.push_tool_use(ToolBlock {
            id: "tu_2".into(),
            name: "Bash".into(),
            input: Value::Null,
            output: None,
        });
        h.record_tool_result("tu_2", "result-2".into());
        h.record_tool_result("tu_1", "result-1".into());
        match h.messages.last().unwrap() {
            ChatMessage::Assistant { tools, .. } => {
                assert_eq!(tools[0].output.as_deref(), Some("result-1"));
                assert_eq!(tools[1].output.as_deref(), Some("result-2"));
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn tool_result_with_no_match_is_silently_dropped() {
        let mut h = ChatHistory::default();
        h.start_assistant(None);
        h.record_tool_result("missing", "n/a".into());
        // No panic, no spurious tool block.
        match h.messages.last().unwrap() {
            ChatMessage::Assistant { tools, .. } => assert!(tools.is_empty()),
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn system_messages_appear_as_their_own_entry() {
        let mut h = ChatHistory::default();
        h.push_system(SystemMessage::ApiRetry {
            attempt: 1,
            max_retries: 3,
            error: "rate_limit".into(),
        });
        assert_eq!(h.messages.len(), 1);
        assert!(matches!(
            h.messages[0],
            ChatMessage::System(SystemMessage::ApiRetry { .. })
        ));
    }
}
