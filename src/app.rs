use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use eframe::egui;
use uuid::Uuid;

use crate::chat::history::{ChatHistory, SystemMessage};
use crate::chat::session::{ChatSession, SpawnOptions};
use crate::chat::stream::ChatEvent;
use crate::cli::CliOptions;
use crate::preview::link::{self, LinkAction};
use crate::preview::loader::{self, LoadError};
use crate::preview::render::{PreviewState, PreviewStatus};
use crate::preview::watcher::{self, FileWatchEvent, FileWatcher, ReloadStep, RELOAD_DEBOUNCE};

pub struct App {
    chat: ChatHistory,
    preview: PreviewState,
    session: Option<ChatSession>,
    events_rx: Option<Receiver<ChatEvent>>,
    disconnect_announced: bool,
    /// Filesystem watcher for the current preview target. `None` only
    /// when `FileWatcher::start` failed at construction (extremely
    /// unusual — would mean the platform notify backend itself is
    /// unavailable). Reload via `notify` is a best-effort feature, so
    /// a missing watcher does not block other UI work.
    watcher: Option<FileWatcher>,
    watch_events_rx: Option<Receiver<FileWatchEvent>>,
    /// Deadline (Instant) at which the debounced reload should fire.
    /// `Some` while we're inside the 100 ms quiet window after the
    /// last `Changed` event for the current preview target. Cleared
    /// after the reload runs.
    pending_reload: Option<Instant>,
    /// Last watcher-side error to surface to the user. Phase 5.3 shows
    /// this as a banner above the preview pane until the next
    /// successful reload clears it. The real home for this signal is
    /// the Phase 7.7 status bar; this is a stop-gap so `docs/preview.md`
    /// §7 ("監視開始失敗時はステータスバーにエラー表示") is at least
    /// visually addressed in MVP.
    watcher_error: Option<String>,
    /// `--enable-dev-tools` runtime opt-in. The dev surface (currently
    /// only the `MDPILOT_DEBUG_SCREENSHOT` capture) only activates
    /// when this flag is set and the env var is present. Default
    /// runs (no flag) ignore the env var entirely.
    debug_screenshot: Option<DebugScreenshot>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cli: CliOptions) -> Self {
        crate::ui::fonts::install_japanese(&cc.egui_ctx);

        let mut chat = ChatHistory::default();
        let (session, events_rx) = match spawn_session(&cc.egui_ctx) {
            Ok((session, rx)) => (Some(session), Some(rx)),
            Err(err) => {
                tracing::warn!(error = %err, "failed to spawn claude session");
                chat.push_system(SystemMessage::SpawnFailed {
                    error: err.to_string(),
                });
                (None, None)
            }
        };

        let preview = preview_state_from_env();

        let (watcher, watch_events_rx, startup_watch_error) = match start_watcher(&cc.egui_ctx) {
            Ok((w, rx)) => (Some(w), Some(rx), None),
            Err(err) => {
                tracing::warn!(error = %err, "failed to start file watcher (auto-reload disabled)");
                (
                    None,
                    None,
                    Some(format!("ファイル監視を開始できません: {err}")),
                )
            }
        };

        let mut app = Self {
            chat,
            preview,
            session,
            events_rx,
            disconnect_announced: false,
            watcher,
            watch_events_rx,
            pending_reload: None,
            watcher_error: startup_watch_error,
            debug_screenshot: DebugScreenshot::from_env(cli),
        };
        app.sync_watch_target();
        app
    }

    /// Adjust the file-watcher subscription so it tracks exactly the
    /// path that `preview` currently displays. Called whenever the
    /// preview target changes (startup load, link-driven switch, or
    /// reload). On `Empty` / `Failed` we keep no watches so we don't
    /// spam tracing on a missing-file scenario.
    fn sync_watch_target(&mut self) {
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        watcher.unwatch_all();
        if let PreviewStatus::Loaded { document, .. } = &self.preview.status {
            if let Err(err) = watcher.watch(&document.path) {
                let label = document.path.display().to_string();
                tracing::warn!(
                    path = %label,
                    error = %err,
                    "failed to attach file watcher",
                );
                self.watcher_error = Some(format!("ファイル監視を開始できません ({label}): {err}"));
            }
        }
    }

    /// Drain any pending `FileWatchEvent`s and update reload bookkeeping.
    /// `Changed` events arm / re-arm the 100 ms debounce window so a
    /// burst of writes (atomic-save editors emit Create + Modify within
    /// a few ms) collapses into one reload. `Removed` is acted on
    /// immediately because the user benefit of the "見つかりません"
    /// banner is highest right at the deletion moment.
    ///
    /// Event drain runs in two passes (collect-then-apply) because
    /// `handle_removed` mutates `self` while `try_recv` is borrowing
    /// `self.watch_events_rx`. Collecting first sidesteps that.
    fn drain_watch_events(&mut self, ctx: &egui::Context) {
        let events = self.collect_watch_events();
        if events.is_empty() {
            return;
        }
        let current_path = match &self.preview.status {
            PreviewStatus::Loaded { document, .. } => Some(document.path.clone()),
            _ => None,
        };
        for event in events {
            match event {
                FileWatchEvent::Changed { path } => {
                    if let Some(current) = current_path.as_deref() {
                        if watcher::paths_match(&path, current) {
                            self.pending_reload = Some(Instant::now() + RELOAD_DEBOUNCE);
                            ctx.request_repaint_after(RELOAD_DEBOUNCE);
                        }
                    }
                }
                FileWatchEvent::Removed { path } => {
                    if let Some(current) = current_path.as_deref() {
                        if watcher::paths_match(&path, current) {
                            self.handle_removed(current.to_path_buf());
                        }
                    }
                }
                FileWatchEvent::Error(message) => {
                    tracing::warn!(message = %message, "file watcher error");
                    self.watcher_error = Some(format!("ファイル監視エラー: {message}"));
                }
            }
        }
    }

    /// First half of the drain: pull every available event off the
    /// receiver into a Vec without touching the rest of `self`.
    fn collect_watch_events(&mut self) -> Vec<FileWatchEvent> {
        let mut out = Vec::new();
        let Some(rx) = self.watch_events_rx.as_ref() else {
            return out;
        };
        loop {
            match rx.try_recv() {
                Ok(event) => out.push(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // Watcher dropped before App — should not happen
                    // in normal flow, but guard so we don't spin on a
                    // dead receiver every frame.
                    self.watch_events_rx = None;
                    break;
                }
            }
        }
        out
    }

    /// If a debounce window has elapsed, reload the preview target from
    /// disk and clear the deadline. Called after `drain_watch_events`
    /// so a brand-new `Changed` arriving this frame still gets the full
    /// quiet window.
    fn poll_pending_reload(&mut self, ctx: &egui::Context) {
        match watcher::reload_decision(self.pending_reload, Instant::now()) {
            ReloadStep::Idle => {}
            ReloadStep::Wait { remaining } => ctx.request_repaint_after(remaining),
            ReloadStep::Fire => {
                self.pending_reload = None;
                self.reload_current();
            }
        }
    }

    /// Re-read the current preview path from disk and update state.
    /// `set_document` keeps the document path stable, so the watcher
    /// subscription does not need to change for a reload. A success
    /// clears any stale watcher-error banner.
    ///
    /// Reload from `Failed` (e.g. user pressed Cmd+R after a missing
    /// file got recreated) walks through the same path: we still know
    /// the target via `Failed::path_label`, so the spec §9 manual
    /// recovery path works without needing a separate state machine.
    fn reload_current(&mut self) {
        let path = match &self.preview.status {
            PreviewStatus::Loaded { document, .. } => Some(document.path.clone()),
            PreviewStatus::Failed { path_label, .. } => Some(PathBuf::from(path_label)),
            PreviewStatus::Empty => None,
        };
        let Some(path) = path else {
            return;
        };
        match loader::load_markdown(&path) {
            Ok(document) => {
                self.preview.set_document(document);
                self.watcher_error = None;
                // Re-attach the watcher in case we recovered from a
                // missing-file state where the path may have been
                // unwatched / re-created.
                self.sync_watch_target();
            }
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    ?error,
                    "reload failed; surfacing as preview error",
                );
                self.preview
                    .set_error(path.to_string_lossy().into_owned(), error);
            }
        }
    }

    fn handle_removed(&mut self, path: PathBuf) {
        let label = path.to_string_lossy().into_owned();
        self.preview.set_error(label, LoadError::NotFound);
        // Cancel any pending Changed-triggered reload — the file is
        // gone, no point reloading.
        self.pending_reload = None;
        // Keep the watch attached. On macOS FSEvents the watch is on
        // the parent directory under the hood, so a future recreate
        // will arrive as Changed and we'll auto-restore per
        // docs/preview.md §7. On Linux inotify the watch may have
        // been invalidated by the unlink; the user can recover via
        // the manual reload added in Phase 5.3.
    }

    /// Write a user prompt to claude's stdin and record it in the local
    /// history. The matching assistant placeholder is only created on
    /// successful write so a `BrokenPipe` doesn't leave an empty assistant
    /// row sitting above the Disconnected banner.
    fn handle_send(&mut self, text: String) {
        self.chat.push_user(text.clone());
        let Some(session) = self.session.as_mut() else {
            // session is None only after a SpawnFailed at startup — that
            // banner is already in history, so don't double up on a second
            // Disconnected line. The Send button should already be disabled
            // in this state; reaching here means the input got past the
            // disabled gate, which is worth logging.
            tracing::warn!("send dispatched without an active claude session");
            return;
        };
        match session.send_user_message(&text) {
            Ok(()) => self.chat.start_assistant(None),
            Err(err) => {
                tracing::warn!(error = %err, "failed to write to claude stdin");
                if !self.disconnect_announced {
                    self.chat.push_system(SystemMessage::Disconnected);
                    self.disconnect_announced = true;
                }
            }
        }
    }

    /// Take ownership of every `OutputCommand::OpenUrl` egui_commonmark
    /// posted during the just-completed UI pass and dispatch it through
    /// our link policy (`docs/preview.md` §5). Other output commands
    /// (CopyText / CopyImage) are left in place for eframe to handle.
    ///
    /// Must run *after* `layout::show` so the link clicks have already
    /// posted their commands, and *before* eframe finalizes the frame
    /// (which would otherwise dispatch every URL via the default
    /// open-in-browser path).
    fn dispatch_link_clicks(&mut self, ctx: &egui::Context) {
        let clicked_urls: Vec<String> = ctx.output_mut(|o| {
            let mut clicked = Vec::new();
            o.commands.retain(|cmd| match cmd {
                egui::OutputCommand::OpenUrl(open_url) => {
                    clicked.push(open_url.url.clone());
                    false
                }
                _ => true,
            });
            clicked
        });

        for url in clicked_urls {
            self.handle_link_click(ctx, &url);
        }
    }

    fn handle_link_click(&mut self, ctx: &egui::Context, href: &str) {
        let current_dir = match &self.preview.status {
            PreviewStatus::Loaded { document, .. } => {
                document.path.parent().map(|p| p.to_path_buf())
            }
            _ => None,
        };
        let action = link::classify(href, current_dir.as_deref());
        match action {
            LinkAction::Empty => {}
            LinkAction::Anchor { fragment } => {
                // egui_commonmark doesn't expose per-heading anchors
                // yet; logging keeps the click traceable until Phase 9.
                tracing::info!(fragment = %fragment, "anchor link click (MVP no-op)");
            }
            LinkAction::External { url } => {
                // Re-post via egui so eframe's webbrowser path handles
                // it — that's what we drained from in the first place.
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
            LinkAction::SwitchMarkdown { path } => {
                let label = path.to_string_lossy().into_owned();
                match loader::load_markdown(&path) {
                    Ok(document) => self.preview.set_document(document),
                    Err(error) => {
                        tracing::warn!(
                            path = %label,
                            ?error,
                            "failed to switch preview target",
                        );
                        self.preview.set_error(label, error);
                    }
                }
                // Re-bind the watcher to the new target; the debounce
                // timer (if any) targeted the old path and is no
                // longer meaningful.
                self.pending_reload = None;
                self.sync_watch_target();
            }
            LinkAction::OpenWithOsApp { path } => {
                if let Err(err) = open::that(&path) {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        "OS open failed",
                    );
                }
            }
        }
    }

    /// Pull every pending event off the channel and fold it into the
    /// history. Called in `logic()` so the side effects are visible by the
    /// time `ui()` renders the same frame.
    fn drain_chat_events(&mut self) {
        let Some(rx) = self.events_rx.as_ref() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(event) => self.chat.apply(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !self.disconnect_announced {
                        self.chat.push_system(SystemMessage::Disconnected);
                        self.disconnect_announced = true;
                    }
                    self.events_rx = None;
                    break;
                }
            }
        }
    }
}

/// Pre-loads a preview file from `MDPILOT_PREVIEW_FILE` if set. This is
/// a Phase 4.2 dev hook — the real entry path is `mdpilot <file>` in
/// Phase 6.1 / `Cmd+O` in Phase 7.1. The env-var path lets us drive the
/// renderer without those UI surfaces yet.
fn preview_state_from_env() -> PreviewState {
    let Ok(raw) = std::env::var("MDPILOT_PREVIEW_FILE") else {
        return PreviewState::default();
    };
    let path = PathBuf::from(&raw);
    match loader::load_markdown(&path) {
        Ok(document) => PreviewState::loaded(document),
        Err(error) => {
            tracing::warn!(path = %raw, ?error, "failed to load MDPILOT_PREVIEW_FILE");
            let mut state = PreviewState::default();
            state.set_error(raw, error);
            state
        }
    }
}

/// Build the filesystem watcher and the channel App will drain each
/// frame. The watcher itself owns its dispatcher thread; we only have
/// to hand it a wake-up closure so the UI thread re-runs `logic()`
/// when an event arrives.
fn start_watcher(ctx: &egui::Context) -> notify::Result<(FileWatcher, Receiver<FileWatchEvent>)> {
    let (tx, rx) = mpsc::channel::<FileWatchEvent>();
    let wake_ctx = ctx.clone();
    let watcher = FileWatcher::start(tx, move || wake_ctx.request_repaint())?;
    Ok((watcher, rx))
}

/// Spawn the claude child process with cwd = current working directory and a
/// fresh session id. Returns the session and the receiver half of the event
/// channel. Phase 6 will replace `current_dir()` with the resolved project
/// root and start reading session ids from `SessionStore`.
fn spawn_session(ctx: &egui::Context) -> std::io::Result<(ChatSession, Receiver<ChatEvent>)> {
    let project_root = std::env::current_dir()?;
    let (tx, rx) = mpsc::channel::<ChatEvent>();
    let wake_ctx = ctx.clone();
    let session = ChatSession::start(
        SpawnOptions {
            project_root,
            session_id: Uuid::new_v4(),
            continue_session: false,
            model: None,
        },
        tx,
        move || wake_ctx.request_repaint(),
    )?;
    Ok((session, rx))
}

impl App {
    /// `docs/ui.md` §6.2: `Cmd+R` (mac) / `Ctrl+R` (Win/Linux) forces
    /// the preview to reload from disk. `consume_shortcut` removes the
    /// event from the queue so a focused `TextEdit` does not also
    /// receive it. Returns whether the reload fired (for tests).
    fn consume_reload_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::R);
        let pressed = ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        if pressed {
            self.reload_current();
        }
        pressed
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_chat_events();
        self.drain_watch_events(ctx);
        self.poll_pending_reload(ctx);
        self.consume_reload_shortcut(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The view records send/cancel intent into these locals; the actual
        // history mutation and stdin write happen after the UI pass so the
        // borrow on `self.chat` inside layout::show stays the only one in
        // play at a time.
        let session_alive = self.session.is_some();
        let mut send_text: Option<String> = None;
        {
            let mut on_send = |text: String| {
                send_text = Some(text);
            };
            crate::ui::layout::show(
                ui,
                &mut self.chat,
                &mut self.preview,
                self.watcher_error.as_deref(),
                session_alive,
                &mut on_send,
            );
        }

        if let Some(text) = send_text {
            self.handle_send(text);
        }

        // Intercept link clicks before eframe's end-of-frame URL
        // dispatch so we can apply the docs/preview.md §5 policy
        // (route .md to set_document, other paths to OS open, etc.).
        self.dispatch_link_clicks(ui.ctx());

        if let Some(cap) = self.debug_screenshot.as_mut() {
            cap.step(ui.ctx());
        }
    }
}

/// One-shot screenshot helper, gated at runtime behind the
/// `--enable-dev-tools` CLI flag.
///
/// Activated by passing `--enable-dev-tools` *and* setting
/// `MDPILOT_DEBUG_SCREENSHOT=/path/to/out.png`. Waits a handful of
/// frames so layout settles, then requests one viewport screenshot,
/// saves it as PNG, and exits the process. Default runs (no flag)
/// short-circuit in `from_env` and never construct the helper, so
/// the env var has no observable effect on production-style runs.
///
/// A hang watchdog (`WATCHDOG_FRAMES_AFTER_REQUEST`) bounds how long
/// we'll wait for `Event::Screenshot` after sending the viewport
/// command. Phase 5.2/5.3 testing observed an intermittent hang on
/// background runs that couldn't be reproduced reliably; we don't
/// know the root cause, but bounding the wait at least keeps stuck
/// dev runs from accumulating as orphan processes.
struct DebugScreenshot {
    path: String,
    frame_count: u32,
    requested: bool,
    closed: bool,
    /// Frame at which the screenshot was first requested. Used by the
    /// watchdog: if `Event::Screenshot` hasn't arrived after
    /// `WATCHDOG_FRAMES_AFTER_REQUEST` frames, abort with an error
    /// rather than spinning forever.
    requested_at_frame: u32,
}

/// `300` frames ≈ 5 seconds at 60 FPS. Generous enough that a busy
/// system has time to render and respond to the Screenshot viewport
/// command, but short enough that an actually-stuck loop fails fast.
const WATCHDOG_FRAMES_AFTER_REQUEST: u32 = 300;

impl DebugScreenshot {
    /// Runtime gate. Returns `None` unless **both** the
    /// `--enable-dev-tools` CLI flag is set *and* the
    /// `MDPILOT_DEBUG_SCREENSHOT` env var carries an output path.
    /// Either condition missing → screenshot is a no-op for this run.
    fn from_env(cli: CliOptions) -> Option<Self> {
        if !cli.enable_dev_tools {
            return None;
        }
        std::env::var("MDPILOT_DEBUG_SCREENSHOT")
            .ok()
            .map(|path| Self {
                path,
                frame_count: 0,
                requested: false,
                closed: false,
                requested_at_frame: 0,
            })
    }

    fn step(&mut self, ctx: &egui::Context) {
        if self.closed {
            return;
        }
        self.frame_count += 1;

        if self.frame_count == 1 || self.frame_count.is_multiple_of(30) {
            tracing::info!(
                target: "mdpilot::devtools",
                frame = self.frame_count,
                requested = self.requested,
                "screenshot helper tick",
            );
        }

        if !self.requested && self.frame_count >= 30 {
            tracing::info!(
                target: "mdpilot::devtools",
                "requesting viewport screenshot",
            );
            self.requested = true;
            self.requested_at_frame = self.frame_count;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }

        if self.requested {
            // Watchdog: if Event::Screenshot hasn't arrived after a
            // few hundred frames, give up rather than spin forever.
            // Phase 5.2/5.3 saw intermittent hangs in background
            // runs that couldn't be reproduced after the
            // --enable-dev-tools refactor; root cause unidentified,
            // but a bounded wait keeps a stuck dev run from leaving
            // an orphan mdpilot process.
            if self.frame_count.saturating_sub(self.requested_at_frame)
                >= WATCHDOG_FRAMES_AFTER_REQUEST
            {
                tracing::error!(
                    target: "mdpilot::devtools",
                    frames_waited = self.frame_count - self.requested_at_frame,
                    path = %self.path,
                    "screenshot watchdog tripped — closing without saving",
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                self.closed = true;
                return;
            }
            let mut grabbed: Option<std::sync::Arc<egui::ColorImage>> = None;
            ctx.input(|i| {
                for event in &i.raw.events {
                    if let egui::Event::Screenshot { image, .. } = event {
                        grabbed = Some(image.clone());
                    }
                }
            });
            if let Some(image) = grabbed {
                let w = image.width() as u32;
                let h = image.height() as u32;
                let mut bytes = Vec::with_capacity(image.pixels.len() * 4);
                for c in image.pixels.iter() {
                    bytes.extend_from_slice(&c.to_array());
                }
                let buf = image::RgbaImage::from_raw(w, h, bytes)
                    .expect("debug screenshot: color image size mismatch");
                buf.save(&self.path)
                    .expect("debug screenshot: png save failed");
                eprintln!("debug screenshot saved to {}", &self.path);
                // Walk through eframe's shutdown so future child processes
                // (claude in Phase 2+) get a chance to Drop cleanly. Bypassing
                // this with std::process::exit would orphan them.
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                self.closed = true;
                return;
            }
        }

        ctx.request_repaint();
    }
}
