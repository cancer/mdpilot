use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use eframe::egui;
use uuid::Uuid;

use crate::chat::history::{ChatHistory, SystemMessage};
use crate::chat::session::{ChatSession, SpawnOptions};
use crate::chat::stream::ChatEvent;
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
    #[cfg(debug_assertions)]
    debug_screenshot: Option<DebugScreenshot>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
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

        let (watcher, watch_events_rx) = match start_watcher(&cc.egui_ctx) {
            Ok((w, rx)) => (Some(w), Some(rx)),
            Err(err) => {
                tracing::warn!(error = %err, "failed to start file watcher (auto-reload disabled)");
                (None, None)
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
            #[cfg(debug_assertions)]
            debug_screenshot: DebugScreenshot::from_env(),
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
                tracing::warn!(
                    path = %document.path.display(),
                    error = %err,
                    "failed to attach file watcher",
                );
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
    /// subscription does not need to change for a reload.
    fn reload_current(&mut self) {
        let Some(path) = (match &self.preview.status {
            PreviewStatus::Loaded { document, .. } => Some(document.path.clone()),
            _ => None,
        }) else {
            return;
        };
        match loader::load_markdown(&path) {
            Ok(document) => self.preview.set_document(document),
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    ?error,
                    "auto-reload failed; surfacing as preview error",
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

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_chat_events();
        self.drain_watch_events(ctx);
        self.poll_pending_reload(ctx);
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

        #[cfg(debug_assertions)]
        if let Some(cap) = self.debug_screenshot.as_mut() {
            cap.step(ui.ctx());
        }
    }
}

/// One-shot screenshot helper compiled only in debug builds.
///
/// Activated by setting `MDPILOT_DEBUG_SCREENSHOT=/path/to/out.png`. Waits a
/// handful of frames so layout settles, then requests one viewport screenshot,
/// saves it as PNG, and exits the process. Release builds skip the entire
/// module so this leaves no production footprint.
#[cfg(debug_assertions)]
struct DebugScreenshot {
    path: String,
    frame_count: u32,
    requested: bool,
    closed: bool,
}

#[cfg(debug_assertions)]
impl DebugScreenshot {
    fn from_env() -> Option<Self> {
        std::env::var("MDPILOT_DEBUG_SCREENSHOT")
            .ok()
            .map(|path| Self {
                path,
                frame_count: 0,
                requested: false,
                closed: false,
            })
    }

    fn step(&mut self, ctx: &egui::Context) {
        if self.closed {
            return;
        }
        self.frame_count += 1;

        if !self.requested && self.frame_count >= 30 {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }

        if self.requested {
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
