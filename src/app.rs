use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use eframe::egui;

use crate::cli::CliOptions;
use crate::preview::link::{self, LinkAction};
use crate::preview::loader::{self};
use crate::preview::render::{PreviewState, PreviewStatus};
use crate::preview::watcher::{self, FileWatchEvent, ProjectWatcher};
use crate::project::{self, ProjectInit};
use crate::tab::{Tab, TabIdGen};
use crate::ui::tab_bar::{self, TabBarAction, TabBarItem};

pub struct App {
    /// Phase 9.5: workspace tabs. Each tab owns a chat session and
    /// the document state it watches. The active tab is the one the
    /// UI draws and the project-event drain routes to.
    tabs: Vec<Tab>,
    active_tab: usize,
    /// Mints `TabId`s for new-tab actions (Cmd+T and the `+` button
    /// on the tab bar).
    tab_id_gen: TabIdGen,
    /// `App` reuses the egui context for spawning new tabs (each
    /// tab needs `wake_ui` to repaint on async events). Stashed on
    /// `App::new` and cloned when a new `Tab` is constructed.
    ctx: egui::Context,

    /// Project-tree watcher (Phase 6.2). Recursive `notify` watcher
    /// rooted at `project_root`; events flow through
    /// `project_events_rx` to `drain_project_events`, which forwards
    /// them to the active tab's auto-follow / image-reload paths.
    _project_watcher: Option<ProjectWatcher>,
    project_events_rx: Option<Receiver<FileWatchEvent>>,

    /// Canonical project root from Phase 6.1's `project::resolve`.
    /// Held for the `Cmd+O` file dialog's initial directory (Phase
    /// 7.1) and as the fallback when the current preview has no
    /// usable parent directory.
    project_root: PathBuf,

    /// Phase 7.4: most recently published window title. Stored so
    /// the per-frame `update_window_title` only sends a
    /// `ViewportCommand::Title` when the computed title has
    /// actually changed.
    last_window_title: String,

    /// `--enable-dev-tools` runtime opt-in. The dev surface (currently
    /// only the `MDPILOT_DEBUG_SCREENSHOT` capture) only activates
    /// when this flag is set and the env var is present.
    debug_screenshot: Option<DebugScreenshot>,

    /// Phase 7.9: start of `App::new`, logged once at the first
    /// `ui()` call so a release-build run can be compared against
    /// the 3-second startup budget.
    startup_started: Option<Instant>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cli: CliOptions, project: ProjectInit) -> Self {
        let startup_started = Instant::now();
        crate::ui::fonts::install_japanese(&cc.egui_ctx);

        let (project_watcher, project_events_rx, startup_watch_error) =
            match start_project_watcher(&cc.egui_ctx, project.root.clone()) {
                Ok((w, rx)) => (Some(w), Some(rx), None),
                Err(err) => {
                    tracing::warn!(
                        root = %project.root.display(),
                        error = %err,
                        "failed to start project watcher (auto-follow disabled)",
                    );
                    (
                        None,
                        None,
                        Some(format!("プロジェクト監視を開始できません: {err}")),
                    )
                }
            };

        let mut tab_id_gen = TabIdGen::default();
        let initial_preview = initial_preview_state(&project);
        let mut initial_tab = Tab::new(
            &cc.egui_ctx,
            &project.root,
            initial_preview,
            tab_id_gen.next(),
            "タブ 1".to_string(),
        );
        // Surface the project-watcher startup error on the initial
        // tab's banner if it has no error of its own (the tab's own
        // FileWatcher::start may also have failed).
        if let Some(message) = startup_watch_error {
            if initial_tab.watcher_error.is_none() {
                initial_tab.watcher_error = Some(message);
            }
        }

        Self {
            tabs: vec![initial_tab],
            active_tab: 0,
            tab_id_gen,
            ctx: cc.egui_ctx.clone(),
            _project_watcher: project_watcher,
            project_events_rx,
            project_root: project.root,
            last_window_title: String::new(),
            debug_screenshot: DebugScreenshot::from_env(cli),
            startup_started: Some(startup_started),
        }
    }

    /// Create a fresh tab with an empty preview and append it to
    /// `tabs`. The new tab becomes active. Per `docs/preview.md`
    /// §9.1.4: starting empty matches the "no positional arg, no
    /// README" startup path.
    fn new_tab(&mut self) {
        let id = self.tab_id_gen.next();
        let label = format!("タブ {}", id.raw());
        let tab = Tab::new(
            &self.ctx,
            &self.project_root,
            PreviewState::default(),
            id,
            label,
        );
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        // Reset the cached window title; the next frame will
        // recompute it for the new active tab and emit a fresh
        // ViewportCommand::Title.
        self.last_window_title.clear();
    }

    /// Close the tab at `idx`. Idempotent at boundaries: refuses
    /// to close the last remaining tab (mdpilot always has at
    /// least one workspace). Adjusts `active_tab` so that:
    ///
    /// - closing the active tab moves focus to its left neighbor
    ///   (or the new tab at index 0 if we closed index 0)
    /// - closing a non-active tab to the *left* of active
    ///   decrements active_tab by 1 (otherwise the index would
    ///   skip)
    fn close_tab(&mut self, idx: usize) {
        if self.tabs.len() <= 1 || idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        if idx < self.active_tab {
            self.active_tab -= 1;
        } else if idx == self.active_tab {
            self.active_tab = idx.min(self.tabs.len() - 1);
        }
        self.last_window_title.clear();
    }

    /// Switch focus to `idx`. No-op if the index is invalid.
    fn select_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() || idx == self.active_tab {
            return;
        }
        self.active_tab = idx;
        self.last_window_title.clear();
    }

    fn active(&self) -> &Tab {
        &self.tabs[self.active_tab]
    }

    fn active_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active_tab]
    }

    /// `docs/ui.md` §6.2: `Cmd+R` (mac) / `Ctrl+R` (Win/Linux) forces
    /// the active tab's preview to reload from disk.
    fn consume_reload_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::R);
        let pressed = ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        if pressed {
            self.active_mut().reload_current();
        }
        pressed
    }

    /// `docs/ui.md` §6.2: `Cmd+O` / `Ctrl+O` opens a Markdown
    /// picker. Selection replaces the active tab's preview and
    /// disables that tab's auto-follow (`docs/preview.md` §9.1.1).
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
            return true;
        };

        let label = path.to_string_lossy().into_owned();
        let tab = self.active_mut();
        match loader::load_markdown(&path) {
            Ok(document) => {
                tab.preview.set_document(document);
                tab.watcher_error = None;
            }
            Err(error) => {
                tracing::warn!(
                    path = %label,
                    ?error,
                    "Cmd+O target failed to load",
                );
                tab.preview.set_error(label, error);
            }
        }
        tab.pending_reload = None;
        tab.pending_follow = None;
        tab.auto_follow_enabled = false;
        tab.sync_watch_target();
        true
    }

    /// `docs/ui.md` §6.2: `Cmd+\` / `Ctrl+\` resets the pane split
    /// to 50/50. This is app-wide (not per-tab) because the pane
    /// layout itself is shared across tabs.
    fn consume_pane_reset_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Backslash);
        let pressed = ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        if pressed {
            crate::ui::layout::reset(ctx);
        }
        pressed
    }

    /// Phase 9.5.3: `Cmd+T` / `Ctrl+T` opens a new tab (empty
    /// preview + fresh claude session). Mirrors the `+` button
    /// in the tab bar.
    fn consume_new_tab_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::T);
        let pressed = ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        if pressed {
            self.new_tab();
        }
        pressed
    }

    /// Phase 9.5.3: `Cmd+W` / `Ctrl+W` closes the active tab.
    /// Refuses to close the last remaining tab (mdpilot always
    /// has at least one workspace). The `close_tab` method
    /// already enforces this.
    fn consume_close_tab_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::W);
        let pressed = ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        if pressed {
            self.close_tab(self.active_tab);
        }
        pressed
    }

    /// Phase 9.5.3: `Cmd+1..9` / `Ctrl+1..9` switches to the N-th
    /// tab. Out-of-range indices are silently ignored (the user
    /// is asking for a tab that isn't there).
    fn consume_tab_switch_shortcuts(&mut self, ctx: &egui::Context) {
        // Digit keys 1..9 map to tab indices 0..8. The naming
        // mismatch (1-based UX, 0-based array) is the standard
        // editor convention (VS Code, browsers, etc.).
        const DIGITS: &[egui::Key] = &[
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        for (idx, key) in DIGITS.iter().enumerate() {
            let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, *key);
            if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                self.select_tab(idx);
            }
        }
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let new_title = compute_window_title(&self.active().preview.status);
        if new_title == self.last_window_title {
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(new_title.clone()));
        self.last_window_title = new_title;
    }

    fn file_picker_start_dir(&self) -> PathBuf {
        match &self.active().preview.status {
            PreviewStatus::Loaded { document, .. } => document
                .path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| self.project_root.clone()),
            _ => self.project_root.clone(),
        }
    }

    /// Phase 6.3: drain project-tree watch events and arm the active
    /// tab's auto-follow timer / image cache invalidation. With
    /// multi-tab (Phase 9.5) we route events to the *active* tab
    /// only — non-active tabs' auto-follow is effectively paused
    /// while they're in the background.
    fn drain_project_events(&mut self, ctx: &egui::Context) {
        let events = self.collect_project_events();
        if events.is_empty() {
            return;
        }
        let tab = self.active_mut();
        let current_path = match &tab.preview.status {
            PreviewStatus::Loaded { document, .. } => Some(document.path.clone()),
            PreviewStatus::Failed { path_label, .. } => Some(PathBuf::from(path_label)),
            PreviewStatus::Empty => None,
        };
        for event in events {
            match event {
                FileWatchEvent::Changed { path } => {
                    if watcher::is_image_path(&path) {
                        let uri = crate::preview::image::to_file_uri(&path);
                        ctx.forget_image(&uri);
                        ctx.request_repaint();
                        continue;
                    }
                    let is_current = current_path
                        .as_deref()
                        .map(|c| watcher::paths_match(&path, c))
                        .unwrap_or(false);
                    if is_current {
                        continue;
                    }
                    if !tab.auto_follow_enabled {
                        continue;
                    }
                    tab.pending_follow = Some((
                        path,
                        Instant::now() + crate::preview::watcher::FOLLOW_DEBOUNCE,
                    ));
                    ctx.request_repaint_after(crate::preview::watcher::FOLLOW_DEBOUNCE);
                }
                FileWatchEvent::Removed { path } => {
                    if watcher::is_image_path(&path) {
                        let uri = crate::preview::image::to_file_uri(&path);
                        ctx.forget_image(&uri);
                        ctx.request_repaint();
                        continue;
                    }
                    if let Some((pending_path, _)) = tab.pending_follow.as_ref() {
                        if watcher::paths_match(&path, pending_path) {
                            tab.pending_follow = None;
                        }
                    }
                }
                FileWatchEvent::Error(message) => {
                    tracing::warn!(
                        target: "mdpilot::project_watch",
                        message = %message,
                        "project watcher error",
                    );
                    tab.watcher_error = Some(format!("プロジェクト監視エラー: {message}"));
                }
            }
        }
    }

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

    /// Take ownership of every `OutputCommand::OpenUrl` egui_commonmark
    /// posted during the just-completed UI pass and dispatch it through
    /// our link policy (`docs/preview.md` §5).
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
        let current_dir = match &self.active().preview.status {
            PreviewStatus::Loaded { document, .. } => {
                document.path.parent().map(|p| p.to_path_buf())
            }
            _ => None,
        };
        let action = link::classify(href, current_dir.as_deref());
        match action {
            LinkAction::Empty => {}
            LinkAction::Anchor { fragment } => {
                tracing::info!(fragment = %fragment, "anchor link click (MVP no-op)");
            }
            LinkAction::External { url } => {
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
            LinkAction::SwitchMarkdown { path } => {
                let label = path.to_string_lossy().into_owned();
                let tab = self.active_mut();
                match loader::load_markdown(&path) {
                    Ok(document) => tab.preview.set_document(document),
                    Err(error) => {
                        tracing::warn!(
                            path = %label,
                            ?error,
                            "failed to switch preview target",
                        );
                        tab.preview.set_error(label, error);
                    }
                }
                tab.pending_reload = None;
                tab.pending_follow = None;
                tab.sync_watch_target();
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
}

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

fn start_project_watcher(
    ctx: &egui::Context,
    root: PathBuf,
) -> notify::Result<(ProjectWatcher, Receiver<FileWatchEvent>)> {
    let (tx, rx) = mpsc::channel::<FileWatchEvent>();
    let wake_ctx = ctx.clone();
    let watcher = ProjectWatcher::start(root, tx, move || wake_ctx.request_repaint())?;
    Ok((watcher, rx))
}

/// Pure helper for `App::update_window_title`. Format:
/// `mdpilot - <basename>` while a file is loaded (or failed to load),
/// plain `mdpilot` for the empty placeholder.
fn compute_window_title(status: &PreviewStatus) -> String {
    const BASE: &str = "mdpilot";
    let basename = match status {
        PreviewStatus::Loaded { document, .. } => document
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string),
        PreviewStatus::Failed { path_label, .. } => std::path::Path::new(path_label)
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string),
        PreviewStatus::Empty => None,
    };
    match basename {
        Some(name) => format!("{BASE} - {name}"),
        None => BASE.to_string(),
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Per-tab event drain. Background tabs still drain their
        // chat / file-watcher events so history accumulates while
        // they're inactive — only the project-watcher dispatch is
        // routed exclusively to the active tab.
        for tab in &mut self.tabs {
            tab.drain_chat_events();
            tab.drain_watch_events(ctx);
        }
        self.drain_project_events(ctx);
        // Pending timers and shortcuts only act on the active tab.
        self.active_mut().poll_pending_reload(ctx);
        self.active_mut().poll_pending_follow(ctx);
        self.consume_reload_shortcut(ctx);
        self.consume_open_shortcut(ctx);
        self.consume_pane_reset_shortcut(ctx);
        self.consume_new_tab_shortcut(ctx);
        self.consume_close_tab_shortcut(ctx);
        self.consume_tab_switch_shortcuts(ctx);
        self.update_window_title(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(start) = self.startup_started.take() {
            let elapsed = start.elapsed();
            tracing::info!(
                target: "mdpilot::perf",
                elapsed_ms = elapsed.as_millis() as u64,
                "first frame rendered (N-01)",
            );
        }

        // Phase 9.5.2 tab bar at the very top. The action drives
        // tab mutations *after* the borrow on `self.tabs` ends,
        // matching the send_text / follow_toggled pattern used
        // below.
        let tab_action = {
            let items: Vec<TabBarItem> = self
                .tabs
                .iter()
                .enumerate()
                .map(|(idx, tab)| TabBarItem {
                    label: &tab.label,
                    is_active: idx == self.active_tab,
                })
                .collect();
            egui::Panel::top("tab_bar")
                .show_inside(ui, |ui| tab_bar::show(ui, &items))
                .inner
        };
        match tab_action {
            TabBarAction::None => {}
            TabBarAction::Select(idx) => self.select_tab(idx),
            TabBarAction::Close(idx) => self.close_tab(idx),
            TabBarAction::NewTab => self.new_tab(),
        }

        let active_idx = self.active_tab;
        let tab = &mut self.tabs[active_idx];
        let session_alive = tab.session.is_some();
        let mut send_text: Option<String> = None;
        let mut follow_toggled = false;

        {
            let mut on_toggle_follow = || {
                follow_toggled = true;
            };
            egui::Panel::top("path_bar").show_inside(ui, |ui| {
                crate::ui::path_bar::show(
                    ui,
                    &tab.preview,
                    tab.auto_follow_enabled,
                    &mut on_toggle_follow,
                    tab.watcher_error.as_deref(),
                    session_alive,
                );
            });
        }
        if follow_toggled {
            tab.auto_follow_enabled = !tab.auto_follow_enabled;
            tracing::info!(
                enabled = tab.auto_follow_enabled,
                "auto-follow toggled via path bar",
            );
        }

        {
            let mut on_send = |text: String| {
                send_text = Some(text);
            };
            crate::ui::layout::show(
                ui,
                &mut tab.chat,
                &mut tab.preview,
                session_alive,
                &mut on_send,
            );
        }

        if let Some(text) = send_text {
            tab.handle_send(text);
        }

        self.dispatch_link_clicks(ui.ctx());

        if let Some(cap) = self.debug_screenshot.as_mut() {
            cap.step(ui.ctx());
        }
    }
}

/// One-shot screenshot helper, gated at runtime behind the
/// `--enable-dev-tools` CLI flag.
struct DebugScreenshot {
    path: String,
    frame_count: u32,
    requested: bool,
    closed: bool,
}

impl DebugScreenshot {
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
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                self.closed = true;
                return;
            }
        }

        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::loader::{LoadError, LoadedDocument, SizeClass};
    use std::path::PathBuf;

    fn loaded(path: &str) -> PreviewStatus {
        PreviewStatus::Loaded {
            document: LoadedDocument {
                path: PathBuf::from(path),
                text: String::new(),
                size_bytes: 0,
                size_class: SizeClass::Small,
            },
            rendered_text_override: None,
        }
    }

    #[test]
    fn empty_preview_uses_bare_title() {
        assert_eq!(compute_window_title(&PreviewStatus::Empty), "mdpilot");
    }

    #[test]
    fn loaded_preview_appends_basename() {
        assert_eq!(
            compute_window_title(&loaded("/Users/u/proj/README.md")),
            "mdpilot - README.md",
        );
    }

    #[test]
    fn failed_preview_uses_basename_from_label() {
        let status = PreviewStatus::Failed {
            path_label: "/Users/u/proj/missing.md".to_string(),
            error: LoadError::NotFound,
        };
        assert_eq!(compute_window_title(&status), "mdpilot - missing.md");
    }

    #[test]
    fn loaded_preview_with_no_filename_falls_back_to_bare() {
        let status = loaded("/");
        assert_eq!(compute_window_title(&status), "mdpilot");
    }
}
