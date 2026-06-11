// `ChatSession` and friends are wired into the UI in Phase 2.5. Until then
// the struct fields and accessors look dead from the bin crate.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::chat::stream::{pipe_stdout_to_channel, ChatEvent};

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
    stdout_handle: Option<JoinHandle<()>>,
    stderr_handle: Option<JoinHandle<()>>,
    stdin: Option<ChildStdin>,
}

impl ChatSession {
    /// Spawn `claude` and start the stdout/stderr drain threads.
    ///
    /// `events_tx` receives every parsed `ChatEvent` (the App folds them into
    /// `ChatHistory`). `wake_ui` is called once per forwarded event so the
    /// main UI thread re-renders without waiting for a mouse move — App passes
    /// a closure that calls `egui::Context::request_repaint`.
    pub fn start<F>(
        opts: SpawnOptions,
        events_tx: Sender<ChatEvent>,
        wake_ui: F,
    ) -> std::io::Result<Self>
    where
        F: Fn() + Send + 'static,
    {
        let args = build_args(&opts);
        // Phase 8.1 (bundle 起動 fix, 2026-06-11): Finder / Dock 経由
        // で `.app` を起動すると launchd のデフォルト PATH
        // (`/usr/bin:/bin:/usr/sbin:/sbin`) だけが渡され、login shell
        // の PATH を引き継がない。`claude` は fnm / nvm / homebrew
        // / volta などの user-local パスにいるのが大半なので、
        // `posix_spawnp` の名前解決が失敗する。
        //
        // 対処: `$SHELL -ilc 'echo $PATH'` で login shell の PATH を
        // 取り、その中から claude の絶対パスを探して `Command::new`
        // に渡す。env にも同じ PATH を流し込んで、claude 自身が起動
        // する node などのサブプロセスでも環境が揃うようにする。
        // 解決できなかった場合は "claude" のままにして、エラーメッ
        // セージで `which claude` を案内する。
        let resolved_path = resolve_login_path();
        let claude_path = find_claude_executable(resolved_path.as_deref());
        let mut command = match claude_path.as_deref() {
            Some(p) => Command::new(p),
            None => Command::new("claude"),
        };
        command
            .args(&args)
            .current_dir(&opts.project_root)
            .env("MDPILOT_PROJECT_ROOT", &opts.project_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(path) = resolved_path {
            command.env("PATH", path);
        }
        let mut child = command.spawn()?;

        // claude's stdout is the stream-json transport. Parsing happens on a
        // dedicated thread so the UI thread never blocks on read().
        let stdout_handle = child.stdout.take().map(|stdout| {
            let reader = BufReader::new(stdout);
            thread::spawn(move || pipe_stdout_to_channel(reader, events_tx, wake_ui))
        });

        // claude's stderr is a low-volume, human-readable log stream; pipe
        // it into tracing so panics / API key errors / etc. surface in the
        // application log. The thread exits when the child closes stderr,
        // which happens after `kill()` in Drop.
        let stderr_handle = child.stderr.take().map(|stderr| {
            let reader = BufReader::new(stderr);
            thread::spawn(move || pipe_lines_to_tracing(reader))
        });

        let stdin = child.stdin.take();

        Ok(Self {
            child,
            session_id: opts.session_id,
            stdout_handle,
            stderr_handle,
            stdin,
        })
    }

    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// Send a single user prompt down claude's stdin as one JSON line in the
    /// schema confirmed by Phase 2.2:
    /// `{"type":"user","message":{"role":"user","content":"<text>"}}`.
    pub fn send_user_message(&mut self, text: &str) -> std::io::Result<()> {
        let stdin = self.stdin.as_mut().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "claude stdin is not available (process exited?)",
            )
        })?;
        write_user_message(stdin, text)
    }

    /// Non-blocking liveness probe. `Running` if claude is still up;
    /// `Exited(status)` once it has terminated (either normally or via the
    /// graceful shutdown path in Drop).
    pub fn status(&mut self) -> std::io::Result<SessionStatus> {
        match self.child.try_wait()? {
            None => Ok(SessionStatus::Running),
            Some(status) => Ok(SessionStatus::Exited(status)),
        }
    }
}

/// Outcome of `ChatSession::status()`.
#[derive(Debug)]
pub enum SessionStatus {
    Running,
    Exited(ExitStatus),
}

impl Drop for ChatSession {
    fn drop(&mut self) {
        // Drop stdin first so claude observes EOF and gets a chance to exit
        // on its own. Without this, --print mode keeps reading from stdin.
        let _ = self.stdin.take();

        // Skip the kill path if claude has already exited; just join drain
        // threads so they don't outlive us.
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            self.join_drains();
            return;
        }

        // Polite shutdown on Unix: SIGTERM and wait up to GRACE_DURATION.
        // Windows has no SIGTERM analogue, so it falls through to kill().
        terminate_polite(&self.child);
        let deadline = Instant::now() + GRACE_DURATION;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Err(_) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        break;
                    }
                    thread::sleep(POLL_INTERVAL);
                }
            }
        }

        self.join_drains();
    }
}

impl ChatSession {
    /// Join the stdout drain first (it EOFs once claude closes stdout, which
    /// only happens after the child has exited), then the stderr drain.
    /// Joining either before the child exits would hang.
    fn join_drains(&mut self) {
        if let Some(handle) = self.stdout_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.stderr_handle.take() {
            let _ = handle.join();
        }
    }
}

const GRACE_DURATION: Duration = Duration::from_millis(500);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

#[cfg(unix)]
fn terminate_polite(child: &Child) {
    // Safety: child.id() is the pid of a process we own; sending SIGTERM is
    // safe even if it has already exited (returns ESRCH which we ignore).
    let pid = child.id() as libc::pid_t;
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
}

#[cfg(not(unix))]
fn terminate_polite(_child: &Child) {
    // No SIGTERM equivalent on Windows; the Drop loop falls through to
    // Child::kill() after the grace period.
}

/// Phase 8.1 helper: walk `path` (colon-separated) and return the
/// first directory that contains an executable named `claude`.
/// Falls back to `None` if no entry matches; the caller then leaves
/// `Command::new("claude")` alone so the spawn failure surfaces a
/// clean "claude not found" error to the user.
pub(crate) fn find_claude_executable(path: Option<&str>) -> Option<PathBuf> {
    let path = path?;
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let candidate = std::path::Path::new(dir).join("claude");
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && (m.permissions().mode() & 0o111 != 0))
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

/// Phase 8.1 (Finder/Dock launch fix, 2026-06-11): when the user
/// double-clicks the `.app` from Finder or launches via Dock, the
/// process inherits launchd's PATH (`/usr/bin:/bin:/usr/sbin:/sbin`)
/// instead of the shell's. `claude` is almost always installed under
/// something the user's login shell sets up (fnm / nvm / Homebrew /
/// volta), so spawn fails with "No such file or directory" until we
/// re-resolve PATH by asking the user's login shell.
///
/// Strategy: run `$SHELL -ilc 'echo -n $PATH'` once and cache the
/// result. The `-i` makes zsh / bash read interactive rc files
/// (where Homebrew + fnm shims usually live). If the call fails
/// (no SHELL, shell missing, timeout, …) we return `None` and the
/// caller falls back to the inherited PATH.
pub(crate) fn resolve_login_path() -> Option<String> {
    use std::sync::OnceLock;
    static CACHED: OnceLock<Option<String>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            let shell = std::env::var_os("SHELL")?;
            let output = std::process::Command::new(&shell)
                .arg("-ilc")
                .arg("printf %s \"$PATH\"")
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let raw = String::from_utf8(output.stdout).ok()?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .clone()
}

/// Serialize a single user message in the stream-json input contract (see
/// `docs/chat.md` §3.1) and write it as one JSON line followed by `\n`,
/// flushing the writer so the child observes the request.
pub(crate) fn write_user_message<W: Write>(writer: &mut W, text: &str) -> std::io::Result<()> {
    let payload = serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": text},
    });
    serde_json::to_writer(&mut *writer, &payload)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
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
///
/// Resume semantics (Phase 9.X.1): `claude` rejects the combination
/// `--session-id <X> --continue` with the error
///
/// > --session-id can only be used with --continue or --resume if
/// > --fork-session is also specified.
///
/// So we pick exactly one of two argument shapes:
///
/// - **New session** (`continue_session == false`): pass
///   `--session-id <uuid>` so the next claude process uses *our*
///   generated UUID. No `--continue` / `--resume`.
/// - **Resume by id** (`continue_session == true`): pass
///   `--resume <uuid>` so claude reopens that specific session.
///   `--session-id` is omitted; with `--resume` claude takes the
///   id from the positional value.
pub(crate) fn build_args(opts: &SpawnOptions) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--print".into(),
        "--verbose".into(),
        "--input-format=stream-json".into(),
        "--output-format=stream-json".into(),
        "--include-partial-messages".into(),
        "--dangerously-skip-permissions".into(),
    ];
    if opts.continue_session {
        args.push("--resume".into());
        args.push(opts.session_id.to_string());
    } else {
        args.push("--session-id".into());
        args.push(opts.session_id.to_string());
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
        // New-session shape: --session-id is included, --resume is
        // not, claude generates the session under our UUID.
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
    fn new_session_uses_session_id_not_resume() {
        // claude rejects --session-id + --continue / --resume
        // without --fork-session, so the new-session path must NOT
        // include the resume flag.
        let args = build_args(&opts(false));
        assert!(
            !args.iter().any(|a| a == "--resume" || a == "--continue"),
            "new session must not pass --resume or --continue: {args:?}",
        );
    }

    #[test]
    fn resumed_session_uses_resume_not_session_id() {
        // The resume path must use `--resume <uuid>` exclusively;
        // including `--session-id` alongside would trip claude's
        // own validation (see build_args doc).
        let args = build_args(&opts(true));
        assert!(
            args.iter().any(|a| a == "--resume"),
            "resumed session must pass --resume: {args:?}",
        );
        assert!(
            !args.iter().any(|a| a == "--session-id"),
            "resumed session must not also pass --session-id: {args:?}",
        );
        assert!(args.contains(&fixed_uuid().to_string()));
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

    #[test]
    fn write_user_message_emits_one_jsonl() {
        let mut buf: Vec<u8> = Vec::new();
        write_user_message(&mut buf, "hello world").unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.ends_with('\n'), "must terminate with newline: {s:?}");
        let trimmed = s.trim_end();
        let parsed: serde_json::Value = serde_json::from_str(trimmed).unwrap();
        assert_eq!(parsed["type"], "user");
        assert_eq!(parsed["message"]["role"], "user");
        assert_eq!(parsed["message"]["content"], "hello world");
    }

    #[test]
    fn write_user_message_escapes_quotes_and_newlines() {
        let mut buf: Vec<u8> = Vec::new();
        write_user_message(&mut buf, "say \"hi\"\nthen \"bye\"").unwrap();
        let s = String::from_utf8(buf).unwrap();
        // Exactly one terminating newline; the embedded newline must be
        // JSON-escaped, not emitted as a literal LF.
        let line_count = s.matches('\n').count();
        assert_eq!(line_count, 1, "expected one \\n, got {line_count}: {s:?}");
        let parsed: serde_json::Value = serde_json::from_str(s.trim_end()).unwrap();
        assert_eq!(parsed["message"]["content"], "say \"hi\"\nthen \"bye\"");
    }

    #[test]
    fn write_user_message_handles_japanese_utf8() {
        let mut buf: Vec<u8> = Vec::new();
        write_user_message(&mut buf, "こんにちは").unwrap();
        let s = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(s.trim_end()).unwrap();
        assert_eq!(parsed["message"]["content"], "こんにちは");
    }

    // ---- Phase 8.1: find_claude_executable ----

    #[test]
    fn find_claude_executable_returns_none_for_no_path() {
        assert_eq!(find_claude_executable(None), None);
    }

    #[test]
    fn find_claude_executable_returns_none_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let path = format!("{}:{}", dir.path().display(), other.path().display());
        assert_eq!(find_claude_executable(Some(&path)), None);
    }

    #[cfg(unix)]
    #[test]
    fn find_claude_executable_finds_first_match() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("claude");
        std::fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
        let mut perms = bin.metadata().unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin, perms).unwrap();
        let path = format!("/nonexistent:{}", dir.path().display());
        assert_eq!(find_claude_executable(Some(&path)), Some(bin));
    }

    #[cfg(unix)]
    #[test]
    fn find_claude_executable_skips_non_executable_files() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("claude");
        // 0o644: readable but not executable — should not match.
        std::fs::write(&bin, "not a binary").unwrap();
        let path = format!("{}", dir.path().display());
        assert_eq!(find_claude_executable(Some(&path)), None);
    }

    #[test]
    fn find_claude_executable_handles_empty_segments() {
        // PATHs from real shells sometimes have empty segments
        // (`:/usr/bin:`); we must not panic or treat them as cwd.
        let dir = tempfile::tempdir().unwrap();
        let path = format!("::{}", dir.path().display());
        assert_eq!(find_claude_executable(Some(&path)), None);
    }
}
