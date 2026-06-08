// Phase 9.5: a `Tab` packages everything that belongs to one
// "workspace": a claude chat session, the document being viewed, and
// the watchers tied to that document. The App holds a `Vec<Tab>` plus
// the *project* root and recursive watcher, which are shared across
// tabs.

// Phase 9.5.1 lands the struct without the tab bar UI. `id`,
// `label`, and `TabId::raw` are consumed by the tab bar coming in
// Phase 9.5.2; until then they look unused to the compiler.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use eframe::egui;
use uuid::Uuid;

use crate::chat::history::{ChatHistory, SystemMessage};
use crate::chat::session::{ChatSession, SpawnOptions};
use crate::chat::stream::ChatEvent;
use crate::preview::loader::{self, LoadError};
use crate::preview::render::{PreviewState, PreviewStatus};
use crate::preview::watcher::{self, FileWatchEvent, FileWatcher, ReloadStep, RELOAD_DEBOUNCE};

/// Stable identifier for a tab. Survives reordering / closing of
/// other tabs, unlike the `Vec<Tab>` index which shifts. UI
/// callbacks (close button, click-to-select) carry this so we
/// look up the tab by ID rather than by stale index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(u64);

impl TabId {
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// Monotonic counter used by `App` to mint fresh `TabId`s on every
/// new-tab action. Stored as a single u64 because tab counts in
/// practice never approach u64 limits.
#[derive(Debug, Default)]
pub struct TabIdGen(u64);

impl TabIdGen {
    pub fn next(&mut self) -> TabId {
        self.0 += 1;
        TabId(self.0)
    }
}

/// One workspace = one chat session + one document view.
///
/// Holds every piece of state that used to live directly on `App`
/// before the multi-tab refactor in Phase 9.5. Methods that mutate
/// per-tab state stay on `Tab`; methods that orchestrate across
/// tabs or the project watcher stay on `App`.
pub struct Tab {
    pub id: TabId,
    /// Display label in the tab bar. Default is `"タブ N"`; a future
    /// task can swap this for the preview filename or the first
    /// user message excerpt.
    pub label: String,

    // ----- chat side -----
    pub chat: ChatHistory,
    pub session: Option<ChatSession>,
    pub events_rx: Option<Receiver<ChatEvent>>,
    pub disconnect_announced: bool,
    /// Claude session ID — either generated fresh or restored from
    /// `SessionStore`. Exposed so App can persist it after a
    /// successful spawn (Phase 9.X.1).
    pub session_id: Uuid,
    /// Phase 9.X.1: set `true` once the first `system/init` event
    /// arrives, which is claude's confirmation that the session was
    /// successfully created (for new sessions) or resumed (for
    /// `--resume`) at the requested id. `App::logic` polls this to
    /// decide when to persist the session-id to disk — we only
    /// want to save ids that claude actually knows about, not ones
    /// that we generated but never used.
    pub session_confirmed: bool,

    // ----- document side -----
    pub preview: PreviewState,
    pub watcher: Option<FileWatcher>,
    pub watch_events_rx: Option<Receiver<FileWatchEvent>>,
    pub pending_reload: Option<Instant>,
    pub pending_follow: Option<(PathBuf, Instant)>,
    pub auto_follow_enabled: bool,
    pub watcher_error: Option<String>,
}

/// Optional handle for resuming an existing claude session.
/// When `Some`, the tab spawns with `--session-id <id> --continue`;
/// otherwise it spawns with a fresh UUID and no `--continue`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResumeSession {
    pub session_id: Uuid,
}

impl Tab {
    /// Construct a fully-wired tab. Spawns the claude child process
    /// (with `project_root` as its cwd) and the per-tab
    /// `FileWatcher`. `initial_preview` is the document state the
    /// tab starts with — startup uses `project::initial_preview`,
    /// `Cmd+T` passes `PreviewState::default()` for an empty pane.
    ///
    /// `resume` is `Some(_)` for the F-11 startup path where we
    /// load the previous run's session-id from `SessionStore`;
    /// `None` for a new tab where we mint a fresh UUID.
    pub fn new(
        ctx: &egui::Context,
        project_root: &Path,
        initial_preview: PreviewState,
        id: TabId,
        label: String,
        resume: Option<ResumeSession>,
    ) -> Self {
        let mut chat = ChatHistory::default();
        let session_id = resume.map(|r| r.session_id).unwrap_or_else(Uuid::new_v4);
        let continue_session = resume.is_some();
        let (session, events_rx) = match spawn_session(
            ctx,
            project_root.to_path_buf(),
            session_id,
            continue_session,
        ) {
            Ok((session, rx)) => (Some(session), Some(rx)),
            Err(err) => {
                tracing::warn!(error = %err, "failed to spawn claude session");
                chat.push_system(SystemMessage::SpawnFailed {
                    error: err.to_string(),
                });
                (None, None)
            }
        };

        let (watcher, watch_events_rx, watcher_error) = match start_watcher(ctx) {
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

        let mut tab = Self {
            id,
            label,
            chat,
            session,
            events_rx,
            disconnect_announced: false,
            session_id,
            session_confirmed: false,
            preview: initial_preview,
            watcher,
            watch_events_rx,
            pending_reload: None,
            pending_follow: None,
            auto_follow_enabled: true,
            watcher_error,
        };
        tab.sync_watch_target();
        tab
    }

    /// Path the preview is currently looking at (`Loaded`), or
    /// `None` when the preview is `Empty` or `Failed`. Used by
    /// the App to persist "session → last preview" across runs
    /// (Phase 9.X.3).
    pub fn current_preview_path(&self) -> Option<&Path> {
        if let PreviewStatus::Loaded { document, .. } = &self.preview.status {
            Some(document.path.as_path())
        } else {
            None
        }
    }

    /// Adjust the file-watcher subscription so it tracks exactly the
    /// path that `preview` currently displays. Called whenever the
    /// preview target changes within this tab.
    pub fn sync_watch_target(&mut self) {
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

    /// Pull every pending chat event off the receiver and fold it
    /// into history. Mirrors the pre-refactor App::drain_chat_events.
    /// Side-effect: flips `session_confirmed = true` on the first
    /// `system/init` event so App knows the session-id is real and
    /// can be persisted to disk (Phase 9.X.1 F-11).
    pub fn drain_chat_events(&mut self) {
        let Some(rx) = self.events_rx.as_ref() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(event) => {
                    if matches!(&event, ChatEvent::Init { .. }) {
                        self.session_confirmed = true;
                    }
                    self.chat.apply(event);
                }
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

    /// File-watcher event handling (single file = this tab's
    /// current preview target). 100 ms debounce on Changed,
    /// immediate Removed handling. Pure two-pass drain so we can
    /// mutate `self` between collect and apply.
    pub fn drain_watch_events(&mut self, ctx: &egui::Context) {
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
                    self.watch_events_rx = None;
                    break;
                }
            }
        }
        out
    }

    /// If the reload debounce has elapsed, reload the preview
    /// target from disk and clear the deadline.
    pub fn poll_pending_reload(&mut self, ctx: &egui::Context) {
        match watcher::reload_decision(self.pending_reload, Instant::now()) {
            ReloadStep::Idle => {}
            ReloadStep::Wait { remaining } => ctx.request_repaint_after(remaining),
            ReloadStep::Fire => {
                self.pending_reload = None;
                self.reload_current();
            }
        }
    }

    /// If the auto-follow debounce has elapsed, switch the preview
    /// target to the queued path and rebind watchers.
    pub fn poll_pending_follow(&mut self, ctx: &egui::Context) {
        let deadline = self.pending_follow.as_ref().map(|(_, d)| *d);
        match watcher::reload_decision(deadline, Instant::now()) {
            ReloadStep::Idle => {}
            ReloadStep::Wait { remaining } => ctx.request_repaint_after(remaining),
            ReloadStep::Fire => {
                let Some((path, _)) = self.pending_follow.take() else {
                    return;
                };
                tracing::info!(
                    path = %path.display(),
                    "auto-follow switching preview target",
                );
                let label = path.to_string_lossy().into_owned();
                match loader::load_markdown(&path) {
                    Ok(document) => {
                        self.preview.set_document(document);
                        self.watcher_error = None;
                        self.pending_reload = None;
                        self.sync_watch_target();
                    }
                    Err(error) => {
                        tracing::warn!(
                            path = %label,
                            ?error,
                            "auto-follow target failed to load",
                        );
                        self.preview.set_error(label, error);
                    }
                }
            }
        }
    }

    /// Reload (Cmd+R or watcher-triggered) the current preview
    /// path. Also walks `Failed` state so a manual reload after a
    /// missing file got recreated works.
    pub fn reload_current(&mut self) {
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

    pub fn handle_removed(&mut self, path: PathBuf) {
        let label = path.to_string_lossy().into_owned();
        self.preview.set_error(label, LoadError::NotFound);
        // Cancel any pending Changed-triggered reload — the file
        // is gone, no point reloading.
        self.pending_reload = None;
    }

    /// Submit a user prompt to claude's stdin. The matching
    /// assistant placeholder is only created on successful write so
    /// a BrokenPipe doesn't leave an empty assistant row above the
    /// Disconnected banner.
    pub fn handle_send(&mut self, text: String) {
        self.chat.push_user(text.clone());
        let Some(session) = self.session.as_mut() else {
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
}

fn spawn_session(
    ctx: &egui::Context,
    project_root: PathBuf,
    session_id: Uuid,
    continue_session: bool,
) -> std::io::Result<(ChatSession, Receiver<ChatEvent>)> {
    let (tx, rx) = mpsc::channel::<ChatEvent>();
    let wake_ctx = ctx.clone();
    let session = ChatSession::start(
        SpawnOptions {
            project_root,
            session_id,
            continue_session,
            model: None,
        },
        tx,
        move || wake_ctx.request_repaint(),
    )?;
    Ok((session, rx))
}

fn start_watcher(ctx: &egui::Context) -> notify::Result<(FileWatcher, Receiver<FileWatchEvent>)> {
    let (tx, rx) = mpsc::channel::<FileWatchEvent>();
    let wake_ctx = ctx.clone();
    let watcher = FileWatcher::start(tx, move || wake_ctx.request_repaint())?;
    Ok((watcher, rx))
}
