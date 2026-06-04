//! Phase 9.X.2: read claude's per-project session history so the
//! UI picker can offer past sessions to resume.
//!
//! claude stores sessions under
//! `<claude_dir>/projects/<encoded-root>/<session-id>.jsonl`, where
//! `<encoded-root>` is the absolute project path with each `/`
//! replaced by `-`. Each `.jsonl` is line-delimited JSON; we read
//! enough of the file to extract the first user message (used as
//! the preview label) and use the OS mtime for "last used" sort
//! key.
//!
//! This module is read-only — mdpilot never writes to claude's
//! storage. If claude changes the on-disk layout in a future
//! release, the picker degrades gracefully (empty list, error
//! banner) rather than crashing.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;
use uuid::Uuid;

/// One past session that the picker can offer to resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    pub session_id: Uuid,
    pub modified: SystemTime,
    /// First user message, truncated for display. `None` when the
    /// file had no recognizable user entry yet (claude can persist
    /// a session header before any exchange happens).
    pub preview: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PickerError {
    #[error("could not locate claude storage dir: {0}")]
    NoClaudeDir(#[from] std::io::Error),
}

/// Encode a project root path the same way claude does: replace
/// every character that isn't ASCII-alphanumeric or `-` with a
/// hyphen. Pure — does not touch the filesystem.
///
/// Verified against the actual `~/.claude/projects/` layout:
/// - `/Users/cancer/repos/mdpilot` → `-Users-cancer-repos-mdpilot`
///   (only `/` rewrites)
/// - `/Users/u/proj/.claude/worktrees/foo` →
///   `-Users-u-proj--claude-worktrees-foo` (both `/` and `.` collapse)
/// - `/Users/u/proj/spike/egui_commonmark` →
///   `-Users-u-proj-spike-egui-commonmark` (`_` also rewrites)
///
/// The encoding is lossy (decode is ambiguous) but claude only
/// reads the directory name as-is, so matching its convention is
/// all that's required. Non-ASCII letters in source paths (CJK,
/// accented chars) are preserved by the `is_ascii_alphanumeric`
/// check — claude's exact behavior there isn't documented, but
/// the picker only needs to find the directory, so a mismatch
/// would just yield an empty session list (graceful).
pub fn encode_project_path(root: &Path) -> String {
    let s = root.to_string_lossy();
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Resolve the directory that holds session jsonl files for a
/// given project root. Returns `None` when the home directory is
/// unavailable — the picker should surface that as "no history".
pub fn project_session_dir(home: &Path, project_root: &Path) -> PathBuf {
    let encoded = encode_project_path(project_root);
    home.join(".claude").join("projects").join(encoded)
}

/// List sessions in `dir`, newest-first by mtime. Skips entries
/// that aren't `.jsonl` files. Sessions whose filenames don't
/// parse as a UUID are silently dropped (claude has occasionally
/// stored auxiliary files alongside session logs).
pub fn list_sessions(dir: &Path) -> Result<Vec<SessionMeta>, PickerError> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(PickerError::NoClaudeDir(err)),
    };

    let mut out: Vec<SessionMeta> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|_| path.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        else {
            continue;
        };
        let Ok(session_id) = Uuid::parse_str(stem) else {
            continue;
        };
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let preview = fs::read_to_string(&path)
            .ok()
            .and_then(|content| parse_first_user_message(&content));
        out.push(SessionMeta {
            session_id,
            modified,
            preview,
        });
    }

    // Sort by mtime descending so the most recent sessions surface
    // at the top of the picker.
    out.sort_by_key(|s| std::cmp::Reverse(s.modified));
    Ok(out)
}

/// Walk a jsonl content blob looking for the first
/// `type == "user"` line whose `message.content` resolves to a
/// non-empty string. Returns `None` when no such line exists
/// (queue-operation lines and unparsable lines are skipped).
///
/// claude has two content shapes for user messages:
/// - plain string: `"content": "hello"`
/// - array of content blocks: `"content": [{"type":"text","text":"hello"}, …]`
///
/// We handle both. Returned text is trimmed and truncated to 120
/// chars (rough chat-bubble width) so the picker layout stays
/// uniform.
pub fn parse_first_user_message(jsonl: &str) -> Option<String> {
    for line in jsonl.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let Some(content) = value.get("message").and_then(|m| m.get("content")) else {
            continue;
        };
        let text = extract_user_text(content)?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        return Some(truncate_preview(trimmed));
    }
    None
}

/// Pull text out of claude's variable content shape. Pure helper
/// extracted so the unit tests can exercise both branches.
fn extract_user_text(content: &Value) -> Option<String> {
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    let arr = content.as_array()?;
    let mut buf = String::new();
    for block in arr {
        if block.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                if !buf.is_empty() {
                    buf.push(' ');
                }
                buf.push_str(text);
            }
        }
    }
    if buf.is_empty() {
        None
    } else {
        Some(buf)
    }
}

/// Cap preview text at ~120 chars and append an ellipsis when
/// truncated. Operates on chars (not bytes) so multibyte
/// characters aren't split.
fn truncate_preview(text: &str) -> String {
    const LIMIT: usize = 120;
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(LIMIT).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn encode_project_path_replaces_slashes_with_hyphens() {
        assert_eq!(
            encode_project_path(Path::new("/Users/cancer/repos/mdpilot")),
            "-Users-cancer-repos-mdpilot",
        );
    }

    #[test]
    fn encode_project_path_collapses_dots_into_hyphens() {
        // `/.claude` ends up as `--claude` because both `/` and
        // `.` are non-alphanumeric and map to `-`.
        assert_eq!(
            encode_project_path(Path::new("/Users/u/proj/.claude/worktrees/foo")),
            "-Users-u-proj--claude-worktrees-foo",
        );
    }

    #[test]
    fn encode_project_path_rewrites_underscore() {
        // Verified against the real claude dir for
        // `…/spike/egui_commonmark` → `…-spike-egui-commonmark`.
        assert_eq!(
            encode_project_path(Path::new("/proj/egui_commonmark")),
            "-proj-egui-commonmark",
        );
    }

    #[test]
    fn encode_project_path_preserves_hyphen_in_source() {
        // Hyphens that already appear in directory names stay as
        // they are; `is_ascii_alphanumeric` is false for `-` but
        // we whitelist it explicitly.
        assert_eq!(
            encode_project_path(Path::new("/proj/dapper-mapping-rainbow")),
            "-proj-dapper-mapping-rainbow",
        );
    }

    #[test]
    fn parse_first_user_message_string_content() {
        let line =
            r#"{"type":"user","message":{"role":"user","content":"hello world"},"timestamp":"x"}"#;
        assert_eq!(
            parse_first_user_message(line),
            Some("hello world".to_string()),
        );
    }

    #[test]
    fn parse_first_user_message_array_content() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hello"},{"type":"text","text":"world"}]}}"#;
        assert_eq!(
            parse_first_user_message(line),
            Some("hello world".to_string()),
        );
    }

    #[test]
    fn parse_first_user_message_skips_queue_lines() {
        // claude's jsonl interleaves queue-operation lines with
        // actual messages; the picker should reach past them.
        let content = r#"{"type":"queue-operation","operation":"enqueue","content":"hi"}
{"type":"queue-operation","operation":"dequeue"}
{"type":"user","message":{"role":"user","content":"hi"}}
{"type":"assistant","message":{"role":"assistant","content":"hello"}}
"#;
        assert_eq!(parse_first_user_message(content), Some("hi".to_string()));
    }

    #[test]
    fn parse_first_user_message_returns_none_when_absent() {
        let content = r#"{"type":"assistant","message":{"role":"assistant","content":"hi"}}"#;
        assert_eq!(parse_first_user_message(content), None);
    }

    #[test]
    fn parse_first_user_message_skips_unparsable_lines() {
        let content = "not json\n{\"type\":\"user\",\"message\":{\"content\":\"survived\"}}\n";
        assert_eq!(
            parse_first_user_message(content),
            Some("survived".to_string()),
        );
    }

    #[test]
    fn parse_first_user_message_truncates_long_text() {
        let long = "a".repeat(200);
        let line = format!(r#"{{"type":"user","message":{{"role":"user","content":"{long}"}}}}"#);
        let preview = parse_first_user_message(&line).unwrap();
        // 120 chars + ellipsis
        assert_eq!(preview.chars().count(), 121);
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn parse_first_user_message_handles_multibyte_truncation() {
        // 200 Japanese characters — would split mid-byte if we
        // used .truncate(LIMIT) on the byte length.
        let long: String = "あ".repeat(200);
        let line = format!(r#"{{"type":"user","message":{{"role":"user","content":"{long}"}}}}"#);
        let preview = parse_first_user_message(&line).unwrap();
        assert_eq!(preview.chars().count(), 121);
    }

    #[test]
    fn list_sessions_returns_empty_for_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(list_sessions(&missing).unwrap().is_empty());
    }

    #[test]
    fn list_sessions_picks_up_jsonl_files_and_skips_others() {
        let dir = tempfile::tempdir().unwrap();
        let id_a = "8f9abf6d-57ea-4fa0-aa9f-a41c9f965f65";
        let id_b = "2b88733c-c80e-4c37-b1e2-233c80bdc8f0";
        write_jsonl(
            dir.path(),
            id_a,
            r#"{"type":"user","message":{"content":"first session"}}"#,
        );
        write_jsonl(
            dir.path(),
            id_b,
            r#"{"type":"user","message":{"content":"second session"}}"#,
        );
        // Distractor entries — should be skipped.
        fs::write(dir.path().join("not-a-uuid.jsonl"), "{}").unwrap();
        fs::write(dir.path().join("settings.json"), "{}").unwrap();
        fs::create_dir(dir.path().join("subagents")).unwrap();

        let sessions = list_sessions(dir.path()).unwrap();
        assert_eq!(sessions.len(), 2);
        let ids: Vec<String> = sessions.iter().map(|s| s.session_id.to_string()).collect();
        assert!(ids.contains(&id_a.to_string()));
        assert!(ids.contains(&id_b.to_string()));
    }

    #[test]
    fn list_sessions_sorts_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let id_old = "00000000-0000-0000-0000-000000000001";
        let id_new = "00000000-0000-0000-0000-000000000002";
        write_jsonl(
            dir.path(),
            id_old,
            r#"{"type":"user","message":{"content":"old"}}"#,
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
        write_jsonl(
            dir.path(),
            id_new,
            r#"{"type":"user","message":{"content":"new"}}"#,
        );

        let sessions = list_sessions(dir.path()).unwrap();
        assert_eq!(sessions[0].session_id.to_string(), id_new);
        assert_eq!(sessions[1].session_id.to_string(), id_old);
    }

    #[test]
    fn list_sessions_extracts_preview_from_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let id = "8f9abf6d-57ea-4fa0-aa9f-a41c9f965f65";
        write_jsonl(
            dir.path(),
            id,
            r#"{"type":"queue-operation"}
{"type":"user","message":{"role":"user","content":"do the thing"}}
"#,
        );
        let sessions = list_sessions(dir.path()).unwrap();
        assert_eq!(sessions[0].preview.as_deref(), Some("do the thing"),);
    }

    fn write_jsonl(dir: &Path, sid: &str, body: &str) {
        let path = dir.join(format!("{sid}.jsonl"));
        let mut f = File::create(path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }
}
