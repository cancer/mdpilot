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
use crate::preview::watcher::{
    self, FileWatchEvent, FileWatcher, ProjectWatcher, ReloadStep, FOLLOW_DEBOUNCE, RELOAD_DEBOUNCE,
};
use crate::project::{self, ProjectInit};

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
    /// Project-tree watcher (Phase 6.2). Holds the watch alive for
    /// the process lifetime; events flow through `project_events_rx`
    /// to `drain_project_events`, which arms `pending_follow` on
    /// changes to *other* `.md` files (auto-follow, F-09 案 A).
    _project_watcher: Option<ProjectWatcher>,
    project_events_rx: Option<Receiver<FileWatchEvent>>,
    /// Phase 6.3: deadline at which the auto-follow switch should
    /// fire. The path is what we'll load when the deadline elapses;
    /// a fresh project event before the deadline updates *both* the
    /// path and the deadline so we always follow the most-recent
    /// write.
    pending_follow: Option<(PathBuf, Instant)>,
    /// Canonical project root from Phase 6.1's `project::resolve`.
    /// Held for the `Cmd+O` file dialog's initial directory (Phase
    /// 7.1) and as the fallback when the current preview has no
    /// usable parent directory.
    project_root: PathBuf,
    /// `--enable-dev-tools` runtime opt-in. The dev surface (currently
    /// only the `MDPILOT_DEBUG_SCREENSHOT` capture) only activates
    /// when this flag is set and the env var is present. Default
    /// runs (no flag) ignore the env var entirely.
    debug_screenshot: Option<DebugScreenshot>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cli: CliOptions, project: ProjectInit) -> Self {
        crate::ui::fonts::install_japanese(&cc.egui_ctx);

        let mut chat = ChatHistory::default();
        let (session, events_rx) = match spawn_session(&cc.egui_ctx, project.root.clone()) {
            Ok((session, rx)) => (Some(session), Some(rx)),
            Err(err) => {
                tracing::warn!(error = %err, "failed to spawn claude session");
                chat.push_system(SystemMessage::SpawnFailed {
                    error: err.to_string(),
                });
                (None, None)
            }
        };

        let preview = initial_preview_state(&project);

        let (watcher, watch_events_rx, mut startup_watch_error) = match start_watcher(&cc.egui_ctx)
        {
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

        let (project_watcher, project_events_rx) =
            match start_project_watcher(&cc.egui_ctx, project.root.clone()) {
                Ok((w, rx)) => (Some(w), Some(rx)),
                Err(err) => {
                    tracing::warn!(
                        root = %project.root.display(),
                        error = %err,
                        "failed to start project watcher (auto-follow disabled)",
                    );
                    // Don't overwrite a previous watcher_error from
                    // the single-file watcher — both can fail
                    // independently, but the first one to fail
                    // gets the banner this frame.
                    if startup_watch_error.is_none() {
                        startup_watch_error =
                            Some(format!("プロジェクト監視を開始できません: {err}"));
                    }
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
            watcher_error: startup_watch_error,
            _project_watcher: project_watcher,
            project_events_rx,
            pending_follow: None,
            project_root: project.root.clone(),
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

    /// Phase 6.3: drain project-tree watch events and arm the
    /// auto-follow timer. Events for the *currently* displayed file
    /// are dropped (the single-file watcher in `drain_watch_events`
    /// will handle that reload via `RELOAD_DEBOUNCE`); events for
    /// *other* `.md` files schedule a follow switch after
    /// `FOLLOW_DEBOUNCE`.
    fn drain_project_events(&mut self, ctx: &egui::Context) {
        let events = self.collect_project_events();
        if events.is_empty() {
            return;
        }
        let current_path = match &self.preview.status {
            PreviewStatus::Loaded { document, .. } => Some(document.path.clone()),
            // `Failed::path_label` is the path the user *intended*
            // to view; for follow purposes we still treat it as
            // "current" so a write to the missing file doesn't trip
            // a follow switch to itself.
            PreviewStatus::Failed { path_label, .. } => Some(PathBuf::from(path_label)),
            PreviewStatus::Empty => None,
        };
        for event in events {
            match event {
                FileWatchEvent::Changed { path } => {
                    let is_current = current_path
                        .as_deref()
                        .map(|c| watcher::paths_match(&path, c))
                        .unwrap_or(false);
                    if is_current {
                        // Single-file watcher owns the reload path
                        // for the current file; we'd double-arm if
                        // we touched anything here.
                        continue;
                    }
                    self.pending_follow = Some((path, Instant::now() + FOLLOW_DEBOUNCE));
                    ctx.request_repaint_after(FOLLOW_DEBOUNCE);
                }
                FileWatchEvent::Removed { path } => {
                    // Don't follow into deleted files. If the deleted
                    // file *was* our pending follow target, drop the
                    // pending switch.
                    if let Some((pending_path, _)) = self.pending_follow.as_ref() {
                        if watcher::paths_match(&path, pending_path) {
                            self.pending_follow = None;
                        }
                    }
                }
                FileWatchEvent::Error(message) => {
                    tracing::warn!(
                        target: "mdpilot::project_watch",
                        message = %message,
                        "project watcher error",
                    );
                    self.watcher_error = Some(format!("プロジェクト監視エラー: {message}"));
                }
            }
        }
    }

    /// Two-pass drain mirror of [`Self::collect_watch_events`]: copy
    /// every pending project event out of the channel before we
    /// touch the rest of `self`. Keeps the borrow checker happy in
    /// `drain_project_events`.
    fn collect_project_events(&mut self) -> Vec<FileWatchEvent> {
        let mut out = Vec::new();
        let Some(rx) = self.project_events_rx.as_ref() else {
            return out;
        };
        loop {
            match rx.try_recv() {
                Ok(event) => out.push(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.project_events_rx = None;
                    break;
                }
            }
        }
        out
    }

    /// Phase 6.3: if the follow debounce has elapsed, switch the
    /// preview target to the queued path. Reuses
    /// `watcher::reload_decision` since the timing semantics are the
    /// same (deadline → Wait/Fire/Idle). Failure to load falls
    /// through to `set_error` so the user sees what happened
    /// (typically a race where the file was deleted between event
    /// and follow).
    fn poll_pending_follow(&mut self, ctx: &egui::Context) {
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
                        // Cancel any in-flight reload for the *old*
                        // file — it doesn't apply to the new target.
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
                // longer meaningful. Likewise drop any pending
                // auto-follow — user-driven navigation wins over
                // claude-driven follow.
                self.pending_reload = None;
                self.pending_follow = None;
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

/// Build the initial `PreviewState` from a resolved `ProjectInit`.
/// Phase 6.4 priority order (docs/preview.md §9.1):
///
/// 1. `init.initial_file` — the user spelled out a file on the
///    command line.
/// 2. A `README.md` (case-insensitive) directly under `init.root`.
/// 3. Empty pane. Auto-follow (Phase 6.3) can still populate it
///    once a `.md` lands anywhere under the root.
///
/// Load failures fall through to `set_error` so the user sees what
/// happened (e.g., the file existed at canonicalize time but became
/// unreadable seconds later).
fn initial_preview_state(project: &ProjectInit) -> PreviewState {
    let Some(path) = project::initial_preview(project) else {
        return PreviewState::default();
    };
    match loader::load_markdown(&path) {
        Ok(document) => PreviewState::loaded(document),
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                ?error,
                "failed to load initial preview target",
            );
            let mut state = PreviewState::default();
            state.set_error(path.to_string_lossy().into_owned(), error);
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

/// Build the project-tree watcher (Phase 6.2). Attaches recursively
/// to the resolved project root; the filter (`.md` only, excluded
/// dirs skipped) is baked into `ProjectWatcher` so the consumer
/// channel only sees relevant events.
fn start_project_watcher(
    ctx: &egui::Context,
    root: PathBuf,
) -> notify::Result<(ProjectWatcher, Receiver<FileWatchEvent>)> {
    let (tx, rx) = mpsc::channel::<FileWatchEvent>();
    let wake_ctx = ctx.clone();
    let watcher = ProjectWatcher::start(root, tx, move || wake_ctx.request_repaint())?;
    Ok((watcher, rx))
}

/// Spawn the claude child process with the supplied project root as
/// its cwd, plus a fresh session id. Phase 6.1 wires the root through
/// from `project::resolve`; Phase 6.5 will additionally set
/// `MDPILOT_PROJECT_ROOT` on the child env. Session-id persistence
/// via `SessionStore` is still pending (Phase 6 sub-task tied to
/// `--continue`).
fn spawn_session(
    ctx: &egui::Context,
    project_root: PathBuf,
) -> std::io::Result<(ChatSession, Receiver<ChatEvent>)> {
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

    /// `docs/ui.md` §6.2: `Cmd+O` (mac) / `Ctrl+O` (Win/Linux) opens
    /// a native file picker filtered to Markdown. The picker call
    /// `rfd::FileDialog::pick_file()` is synchronous — on macOS the
    /// egui frame loop pauses while the dialog is up, which matches
    /// the OS-wide convention that file dialogs block.
    ///
    /// On selection, the chosen path goes through the same load path
    /// as a link-driven `SwitchMarkdown`: load, set_document, drop
    /// any pending reload / follow, then re-bind the watcher.
    ///
    /// Phase 7.2 will add the auto-follow ON/OFF flag and turn it
    /// OFF on a successful pick (per `docs/preview.md` §9.1.1). For
    /// now we just clear the one-shot `pending_follow` so a
    /// project event captured *just before* the dialog dismissal
    /// doesn't snap the preview away from the file the user picked.
    fn consume_open_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
        let pressed = ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        if !pressed {
            return false;
        }

        let start_dir = self.file_picker_start_dir();
        let picked = rfd::FileDialog::new()
            .add_filter("Markdown", &["md", "markdown"])
            .set_directory(&start_dir)
            .pick_file();

        let Some(path) = picked else {
            // User dismissed without selecting — nothing to do.
            return true;
        };

        let label = path.to_string_lossy().into_owned();
        match loader::load_markdown(&path) {
            Ok(document) => {
                self.preview.set_document(document);
                self.watcher_error = None;
            }
            Err(error) => {
                tracing::warn!(
                    path = %label,
                    ?error,
                    "Cmd+O target failed to load",
                );
                self.preview.set_error(label, error);
            }
        }
        // Both pending timers were tied to the *previous* preview
        // target; neither applies after a user-driven switch.
        self.pending_reload = None;
        self.pending_follow = None;
        self.sync_watch_target();
        true
    }

    /// Pick the directory the file dialog should open in. Prefer the
    /// currently-shown file's parent, fall back to the project root.
    fn file_picker_start_dir(&self) -> PathBuf {
        match &self.preview.status {
            PreviewStatus::Loaded { document, .. } => document
                .path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| self.project_root.clone()),
            _ => self.project_root.clone(),
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_chat_events();
        self.drain_watch_events(ctx);
        self.drain_project_events(ctx);
        self.poll_pending_reload(ctx);
        self.poll_pending_follow(ctx);
        self.consume_reload_shortcut(ctx);
        self.consume_open_shortcut(ctx);
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
struct DebugScreenshot {
    path: String,
    frame_count: u32,
    requested: bool,
    closed: bool,
}

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
