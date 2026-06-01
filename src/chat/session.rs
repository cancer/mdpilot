// `ChatSession` and friends are wired into the UI in Phase 2.5. Until then
// the struct fields and accessors look dead from the bin crate.
#![allow(dead_code)]

use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::{self, JoinHandle};

use uuid::Uuid;

/// Options for spawning the `claude` child process. The contract is pinned
/// in `docs/chat.md` 2.1 and verified against an actual `claude` run in
/// `docs/spike-report.md` (Phase 2.0).
pub struct SpawnOptions {
    pub project_root: PathBuf,
    pub session_id: Uuid,
    /// If `true`, append `--continue` so claude resumes the session that the
    /// given `session_id` already references on disk. New sessions pass `false`.
    pub continue_session: bool,
    /// Override claude's default model. `None` means claude picks.
    pub model: Option<String>,
}

/// Owns the `claude` child process. Dropping the session sends `kill()` to
/// the child so the process tree never outlives mdpilot — see
/// `docs/chat.md` 2.4 and the Phase 1.4 advisor note for why this matters.
pub struct ChatSession {
    child: Child,
    session_id: Uuid,
    stderr_handle: Option<JoinHandle<()>>,
}

impl ChatSession {
    pub fn start(opts: SpawnOptions) -> std::io::Result<Self> {
        let args = build_args(&opts);
        let mut child = Command::new("claude")
            .args(&args)
            .current_dir(&opts.project_root)
            .env("MDPILOT_PROJECT_ROOT", &opts.project_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // claude's stderr is a low-volume, human-readable log stream; pipe
        // it into tracing so panics / API key errors / etc. surface in the
        // application log. The thread exits when the child closes stderr,
        // which happens after `kill()` in Drop.
        let stderr_handle = child.stderr.take().map(|stderr| {
            let reader = std::io::BufReader::new(stderr);
            thread::spawn(move || pipe_lines_to_tracing(reader))
        });

        Ok(Self {
            child,
            session_id: opts.session_id,
            stderr_handle,
        })
    }

    pub fn session_id(&self) -> Uuid {
        self.session_id
    }
}

impl Drop for ChatSession {
    fn drop(&mut self) {
        // Best-effort termination; the child may have already exited.
        let _ = self.child.kill();
        // Drain the stderr thread so its output reaches tracing before we
        // return from Drop. The thread exits on EOF, which kill() guarantees.
        if let Some(handle) = self.stderr_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Forward every line of `reader` to `tracing::warn` under the
/// `claude::stderr` target. Reading is best-effort: read errors are logged
/// and end the drain. Returns when the reader hits EOF.
pub(crate) fn pipe_lines_to_tracing<R: BufRead>(reader: R) {
    for line in reader.lines() {
        match line {
            Ok(line) => tracing::warn!(target: "claude::stderr", "{line}"),
            Err(err) => {
                tracing::warn!("error reading claude stderr: {err}");
                break;
            }
        }
    }
}

/// Assemble the CLI argument list for `claude`. Kept as a pure function so
/// we can unit test the contract without spawning a process.
pub(crate) fn build_args(opts: &SpawnOptions) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--print".into(),
        "--verbose".into(),
        "--input-format=stream-json".into(),
        "--output-format=stream-json".into(),
        "--include-partial-messages".into(),
        "--dangerously-skip-permissions".into(),
        "--session-id".into(),
        opts.session_id.to_string(),
    ];
    if opts.continue_session {
        args.push("--continue".into());
    }
    if let Some(model) = &opts.model {
        args.push("--model".into());
        args.push(model.clone());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_uuid() -> Uuid {
        Uuid::parse_str("12345678-1234-1234-1234-1234567890ab").unwrap()
    }

    fn opts(continue_session: bool) -> SpawnOptions {
        SpawnOptions {
            project_root: PathBuf::from("/tmp/mdpilot-test"),
            session_id: fixed_uuid(),
            continue_session,
            model: None,
        }
    }

    #[test]
    fn baseline_args_include_required_flags() {
        let args = build_args(&opts(false));
        for required in [
            "--print",
            "--verbose",
            "--input-format=stream-json",
            "--output-format=stream-json",
            "--include-partial-messages",
            "--dangerously-skip-permissions",
            "--session-id",
        ] {
            assert!(
                args.iter().any(|a| a == required),
                "missing {required}: {args:?}",
            );
        }
        assert!(args.contains(&fixed_uuid().to_string()));
    }

    #[test]
    fn new_session_omits_continue() {
        let args = build_args(&opts(false));
        assert!(
            !args.iter().any(|a| a == "--continue"),
            "new session should not pass --continue: {args:?}",
        );
    }

    #[test]
    fn resumed_session_includes_continue() {
        let args = build_args(&opts(true));
        assert!(
            args.iter().any(|a| a == "--continue"),
            "resumed session should pass --continue: {args:?}",
        );
    }

    #[test]
    fn model_override_appears_after_model_flag() {
        let mut o = opts(false);
        o.model = Some("claude-opus-4-7".into());
        let args = build_args(&o);
        let model_idx = args
            .iter()
            .position(|a| a == "--model")
            .expect("--model expected in args");
        assert_eq!(
            args.get(model_idx + 1).map(String::as_str),
            Some("claude-opus-4-7")
        );
    }

    #[test]
    fn omitting_model_omits_model_flag() {
        let args = build_args(&opts(false));
        assert!(
            !args.iter().any(|a| a == "--model"),
            "default invocation should not pass --model: {args:?}",
        );
    }

    #[test]
    fn pipe_lines_to_tracing_drains_until_eof() {
        let payload = b"first stderr line\nsecond line\n";
        let reader = std::io::Cursor::new(&payload[..]);
        pipe_lines_to_tracing(reader);
        // Just asserting that the function returns rather than blocking or
        // panicking; tracing output isn't observable without a subscriber.
    }

    #[test]
    fn pipe_lines_to_tracing_returns_on_empty_input() {
        let reader = std::io::Cursor::new(&b""[..]);
        pipe_lines_to_tracing(reader);
    }
}
