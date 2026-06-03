// Phase 5.1 puts the infrastructure in place; the wiring into App and
// the 100 ms debounce land in Phase 5.2 (single-file reload, F-08), so
// the public surface is intentionally unused from the bin crate for
// one phase. Silence the dead-code lint for that interval.
#![allow(dead_code)]

//! Background filesystem watcher built on `notify`.
//!
//! `notify::recommended_watcher` owns its own dispatcher thread (on
//! macOS it sits on top of FSEvents), so this module does *not* spawn
//! an additional `std::thread`. The callback registered with the
//! watcher runs on notify's thread; it translates raw `notify::Event`
//! values into our coarser `FileWatchEvent` and pushes them through
//! an `mpsc::Sender` so the UI thread can drain them every frame —
//! the same pattern `ChatSession::start` uses for claude stdout.
//!
//! Phase 5.1 only provides the watcher *infrastructure*; the actual
//! preview-target binding (start/stop on `set_document`, 100 ms
//! debounce, re-render on `Changed`, "ファイルが見つかりません" on
//! `Removed`) is Phase 5.2 per `docs/preview.md` §7 and `docs/plan.md`.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// `docs/preview.md` §7 — collapse rapid-fire writes (atomic-save
/// editors often emit Create + Modify(Content) + Modify(Metadata)
/// within a few ms) into a single reload after a 100 ms quiet
/// period. Held here so App and tests share one number.
pub const RELOAD_DEBOUNCE: Duration = Duration::from_millis(100);

/// `docs/plan.md` Phase 6.3 / `docs/claude-integration.md` §6.2 —
/// quiet window between a project-tree write and the auto-follow
/// switch. Larger than `RELOAD_DEBOUNCE` because we want to give
/// claude / external editors a moment to *also* hit the current
/// file before we yank the preview away from it.
pub const FOLLOW_DEBOUNCE: Duration = Duration::from_millis(200);

/// Coarse-grained file-change notification that App cares about.
/// One `notify::Event` may name multiple paths and one of several
/// `EventKind`s; `classify_event` collapses those into a flat list
/// of `FileWatchEvent` values so the consumer doesn't have to know
/// about the notify enum hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileWatchEvent {
    /// File content / metadata changed, or the file was just created.
    /// Atomic saves from editors typically arrive as Create+Modify;
    /// we conflate both into `Changed` so the App can treat them
    /// identically (reload-from-disk).
    Changed { path: PathBuf },
    /// File was removed or renamed-away from its watched location.
    /// Preview should switch to the "見つかりません" state per
    /// `docs/preview.md` §7. The next `Changed` for the same path
    /// (recreation) should automatically bring it back.
    Removed { path: PathBuf },
    /// Backend reported a non-fatal error (e.g., overflow of the
    /// kernel event queue, watch failure on one of several paths).
    /// The string is `notify::Error::to_string()` so consumers don't
    /// depend on `notify::Error` directly.
    Error(String),
}

/// Owns the platform-recommended watcher. The watcher itself manages
/// its dispatcher thread; this struct just remembers which paths we
/// asked it to watch so callers can `unwatch_all()` cheaply on file
/// switch.
pub struct FileWatcher {
    inner: RecommendedWatcher,
    watched: Vec<PathBuf>,
}

impl FileWatcher {
    /// Start the watcher. `events_tx` is the sender half of the
    /// channel App drains on each frame; `wake_ui` should call
    /// `egui::Context::request_repaint` so the UI sees the event
    /// without waiting for the next user input. Both run on notify's
    /// dispatcher thread.
    ///
    /// Returns the constructed `FileWatcher` with no paths attached —
    /// call [`watch`](Self::watch) to register a target.
    pub fn start<F>(events_tx: mpsc::Sender<FileWatchEvent>, wake_ui: F) -> notify::Result<Self>
    where
        F: Fn() + Send + 'static,
    {
        let handler = move |res: notify::Result<Event>| {
            let mut delivered = false;
            match res {
                Ok(event) => {
                    for our_event in classify_event(&event) {
                        if events_tx.send(our_event).is_err() {
                            // Receiver gone (App dropped) — silently
                            // give up; subsequent events will fail the
                            // same way and the watcher will be Dropped
                            // soon.
                            return;
                        }
                        delivered = true;
                    }
                }
                Err(err) => {
                    // Error path is best-effort: surface once, then
                    // notify will keep delivering future events.
                    let _ = events_tx.send(FileWatchEvent::Error(err.to_string()));
                    delivered = true;
                }
            }
            if delivered {
                wake_ui();
            }
        };
        let inner = notify::recommended_watcher(handler)?;
        Ok(Self {
            inner,
            watched: Vec::new(),
        })
    }

    /// Start watching `path` for changes. We always use
    /// `RecursiveMode::NonRecursive` — preview targets a single file,
    /// and recursive watching is reserved for Phase 6.2 (project-root
    /// auto-follow).
    pub fn watch(&mut self, path: &Path) -> notify::Result<()> {
        self.inner.watch(path, RecursiveMode::NonRecursive)?;
        // Keep the canonical-ish form by storing what was passed in;
        // callers do their own resolution before calling us.
        if !self.watched.iter().any(|p| p == path) {
            self.watched.push(path.to_path_buf());
        }
        Ok(())
    }

    /// Stop watching `path`. Idempotent at the bookkeeping level
    /// (a no-op if we never added it), but propagates notify's error
    /// when the backend complains.
    pub fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        self.inner.unwatch(path)?;
        self.watched.retain(|p| p != path);
        Ok(())
    }

    /// Stop watching every registered path. Used by App when the
    /// preview target switches: drop the old watch before attaching
    /// the new one so we don't briefly double-report.
    pub fn unwatch_all(&mut self) {
        // Use mem::take so we can iterate-and-mutate without
        // borrow-checker gymnastics. If a single unwatch fails we log
        // and continue — losing the bookkeeping is worse than a noisy
        // backend complaint.
        let paths = std::mem::take(&mut self.watched);
        for path in paths {
            if let Err(err) = self.inner.unwatch(&path) {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "failed to unwatch path during unwatch_all",
                );
            }
        }
    }

    /// Snapshot of currently-registered watch paths. Exists for
    /// debugging and tests; App should not depend on the order or
    /// uniqueness guarantees beyond what `watch`/`unwatch` provide.
    pub fn watched_paths(&self) -> &[PathBuf] {
        &self.watched
    }
}

/// Outcome of one polling step against the reload deadline.
/// `Idle` when no Changed event has armed the timer, `Wait` while we
/// are still inside the debounce window (caller should
/// `ctx.request_repaint_after(remaining)`), `Fire` once the deadline
/// has elapsed (caller should reload and clear the deadline).
#[derive(Debug, PartialEq)]
pub enum ReloadStep {
    Idle,
    Wait { remaining: Duration },
    Fire,
}

/// Pure: decide what the App should do this frame given a possibly
/// pending reload deadline. The App owns the `Option<Instant>` and
/// passes it in; this function does not mutate state, so deciding
/// whether to clear the deadline on `Fire` stays at the call site.
pub fn reload_decision(deadline: Option<Instant>, now: Instant) -> ReloadStep {
    match deadline {
        None => ReloadStep::Idle,
        Some(d) if now >= d => ReloadStep::Fire,
        Some(d) => ReloadStep::Wait { remaining: d - now },
    }
}

/// True iff `a` and `b` denote the same filesystem entity.
///
/// macOS FSEvents canonicalizes paths before delivery (e.g.
/// `/private/var/...` instead of the symlink-redirected
/// `/var/...`), so direct `==` comparison between the path we
/// asked to watch and the path carried by an incoming event will
/// often fail. We try exact equality first (cheap, no syscall),
/// then fall back to `fs::canonicalize` on both sides. If both
/// canonicalize calls fail (e.g. the file was just deleted, which
/// is exactly when path comparison still matters), report
/// inequality — the App will get a separate `Removed` event for
/// the original path anyway.
pub fn paths_match(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    let canonical_a = std::fs::canonicalize(a).ok();
    let canonical_b = std::fs::canonicalize(b).ok();
    match (canonical_a, canonical_b) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Pure mapping from raw notify event to our flat event list. One
/// `notify::Event` carries `paths: Vec<PathBuf>`, so we fan out one
/// `FileWatchEvent` per path. Returning `Vec` keeps the function
/// trivially unit-testable — the call site immediately iterates.
///
/// `Access` events (open/close/exec) are dropped because they're
/// noisy on macOS FSEvents and don't help the preview decide whether
/// to reload. `Any` / `Other` are also dropped — when the backend
/// can't classify, we'd rather miss than spuriously reload.
pub fn classify_event(event: &Event) -> Vec<FileWatchEvent> {
    let mut out = Vec::with_capacity(event.paths.len());
    for path in &event.paths {
        match event.kind {
            EventKind::Modify(_) | EventKind::Create(_) => {
                out.push(FileWatchEvent::Changed { path: path.clone() });
            }
            EventKind::Remove(_) => {
                out.push(FileWatchEvent::Removed { path: path.clone() });
            }
            EventKind::Access(_) | EventKind::Any | EventKind::Other => {}
        }
    }
    out
}

// =====================================================================
// Phase 6.2: project-tree watcher (recursive, .md-only, with
// excluded directories per docs/claude-integration.md §6.1).
// Auto-follow logic that consumes these events lives in Phase 6.3.
// =====================================================================

/// Directories we skip when fanning out events for the project tree.
/// `docs/claude-integration.md` §6.1 spells out this list as the
/// default; override (e.g. via `.mdpilotignore`) is MVP-后. Matching is
/// case-sensitive because all the names are conventional.
pub const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".svelte-kit",
    ".venv",
    "__pycache__",
];

/// Markdown extensions that count as "preview targets" for auto-follow.
/// Case-insensitive (`README.MD`, `Notes.Markdown` should match).
const MARKDOWN_EXTENSIONS: &[&str] = &["md", "markdown"];

/// Image extensions that the project watcher tracks for the
/// auto-reload pipeline (Phase 9.1B). The set matches the formats
/// `docs/preview.md` §6 lists; we forward the file watch events for
/// each, and the App turns them into `ctx.forget_image` calls so
/// `egui_extras::FileLoader` re-fetches on the next render. HTTPS
/// images are not in scope — they have no local file to watch.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg"];

/// True when `name` is one of the dirs in [`EXCLUDED_DIRS`]. Pure,
/// case-sensitive — the names are conventional and lowercasing would
/// false-positive `.GIT/`-style dirs (rare but technically valid).
pub fn is_excluded_dir(name: &str) -> bool {
    EXCLUDED_DIRS.contains(&name)
}

/// True when `path`'s file extension is one of [`MARKDOWN_EXTENSIONS`]
/// (case-insensitive). Files without an extension never match.
pub fn is_markdown_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(ext) if MARKDOWN_EXTENSIONS.iter().any(|m| ext.eq_ignore_ascii_case(m))
    )
}

/// True when `path`'s file extension is one of [`IMAGE_EXTENSIONS`]
/// (case-insensitive). Used by `classify_project_event` to forward
/// image-file changes alongside `.md` changes.
pub fn is_image_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(ext) if IMAGE_EXTENSIONS.iter().any(|m| ext.eq_ignore_ascii_case(m))
    )
}

/// True when `path` lives somewhere inside an [`EXCLUDED_DIRS`]
/// subtree under `root`. The match is on the path components strictly
/// between `root` and `path` — `root/.git/HEAD` is excluded,
/// `root/.git` (the dir itself) is *not* excluded by this predicate
/// because it has no intermediate component (we filter the directory
/// itself via the same predicate at higher levels, but this function
/// answers "is the leaf inside an excluded subtree").
///
/// Returns `false` when `path` is not under `root`; callers using the
/// project watcher should treat out-of-root paths as out-of-scope.
pub fn is_in_excluded_subtree(path: &Path, root: &Path) -> bool {
    let relative = match path.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let mut components = relative.components();
    // The last component is `path`'s file name (or the leaf dir);
    // we only care about the components above it. `next_back` peels
    // off the leaf so we don't false-positive on a file *named*
    // ".git" sitting at the project root.
    components.next_back();
    components.any(|c| match c {
        std::path::Component::Normal(name) => name.to_str().map(is_excluded_dir).unwrap_or(false),
        _ => false,
    })
}

/// Pure project-tree classifier: returns `Changed` / `Removed`
/// events for `.md` (Phase 6.3 auto-follow) **and** image files
/// (Phase 9.1B auto-reload) outside excluded subtrees. Everything
/// else is dropped. The App distinguishes the two cases by
/// checking the extension at drain time.
pub fn classify_project_event(event: &Event, root: &Path) -> Vec<FileWatchEvent> {
    let mut out = Vec::new();
    for path in &event.paths {
        if !is_markdown_path(path) && !is_image_path(path) {
            continue;
        }
        if is_in_excluded_subtree(path, root) {
            continue;
        }
        match event.kind {
            EventKind::Modify(_) | EventKind::Create(_) => {
                out.push(FileWatchEvent::Changed { path: path.clone() });
            }
            EventKind::Remove(_) => {
                out.push(FileWatchEvent::Removed { path: path.clone() });
            }
            EventKind::Access(_) | EventKind::Any | EventKind::Other => {}
        }
    }
    out
}

/// Recursive watcher rooted at the project directory. Emits filtered
/// `FileWatchEvent`s (`.md` / `.markdown` only, excluded dirs
/// skipped) onto the channel handed to [`Self::start`].
///
/// Wraps the same `notify::recommended_watcher` as [`FileWatcher`];
/// kept as a distinct type so its filter contract (project-tree,
/// markdown-only) is visible at the type level rather than encoded
/// in a runtime predicate.
///
/// Per `docs/claude-integration.md` §2 the project root does not
/// change during a single mdpilot run (switching projects means a
/// new window). So `start` attaches immediately and stays attached
/// for the lifetime of the watcher — no `watch` / `unwatch_all`
/// surface like [`FileWatcher`] has.
pub struct ProjectWatcher {
    _inner: RecommendedWatcher,
    root: PathBuf,
}

impl ProjectWatcher {
    /// Start the project watcher attached to `root`. The notify
    /// callback closes over `root` so it can classify incoming
    /// events without further synchronization; we keep a copy on
    /// `Self` purely for the [`Self::watched_root`] accessor.
    pub fn start<F>(
        root: PathBuf,
        events_tx: mpsc::Sender<FileWatchEvent>,
        wake_ui: F,
    ) -> notify::Result<Self>
    where
        F: Fn() + Send + 'static,
    {
        let root_for_callback = root.clone();
        let handler = move |res: notify::Result<Event>| {
            let mut delivered = false;
            match res {
                Ok(event) => {
                    for our_event in classify_project_event(&event, &root_for_callback) {
                        if events_tx.send(our_event).is_err() {
                            // Receiver dropped — surface no more
                            // events. notify will continue calling
                            // us; subsequent iterations will short-
                            // circuit the same way.
                            return;
                        }
                        delivered = true;
                    }
                }
                Err(err) => {
                    let _ = events_tx.send(FileWatchEvent::Error(err.to_string()));
                    delivered = true;
                }
            }
            if delivered {
                wake_ui();
            }
        };
        let mut inner = notify::recommended_watcher(handler)?;
        inner.watch(&root, RecursiveMode::Recursive)?;
        Ok(Self {
            _inner: inner,
            root,
        })
    }

    /// Path the watcher is rooted at. Useful for debugging / tests.
    pub fn watched_root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind, RemoveKind, RenameMode};

    fn event(kind: EventKind, paths: Vec<&str>) -> Event {
        Event {
            kind,
            paths: paths.into_iter().map(PathBuf::from).collect(),
            attrs: Default::default(),
        }
    }

    #[test]
    fn modify_yields_changed() {
        let ev = event(
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            vec!["/tmp/a.md"],
        );
        assert_eq!(
            classify_event(&ev),
            vec![FileWatchEvent::Changed {
                path: PathBuf::from("/tmp/a.md")
            }]
        );
    }

    #[test]
    fn create_yields_changed() {
        // Atomic-save editors often produce Create instead of Modify.
        // We conflate both into Changed so the App reloads either way.
        let ev = event(EventKind::Create(CreateKind::File), vec!["/tmp/new.md"]);
        assert_eq!(
            classify_event(&ev),
            vec![FileWatchEvent::Changed {
                path: PathBuf::from("/tmp/new.md")
            }]
        );
    }

    #[test]
    fn remove_yields_removed() {
        let ev = event(EventKind::Remove(RemoveKind::File), vec!["/tmp/gone.md"]);
        assert_eq!(
            classify_event(&ev),
            vec![FileWatchEvent::Removed {
                path: PathBuf::from("/tmp/gone.md")
            }]
        );
    }

    #[test]
    fn rename_modify_yields_changed() {
        // Rename-modify is what some editors use for atomic saves
        // (rename tmp → target). We treat it the same as a content
        // modify so the reload kicks in.
        let ev = event(
            EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            vec!["/tmp/renamed.md"],
        );
        assert_eq!(
            classify_event(&ev),
            vec![FileWatchEvent::Changed {
                path: PathBuf::from("/tmp/renamed.md")
            }]
        );
    }

    #[test]
    fn access_events_are_ignored() {
        // FSEvents on macOS emits AccessMode::Open / Close for every
        // open — they don't help the preview and would force a
        // reload-on-every-read storm if surfaced.
        let ev = event(
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            vec!["/tmp/a.md"],
        );
        assert!(classify_event(&ev).is_empty());
    }

    #[test]
    fn any_and_other_kinds_are_ignored() {
        // Backend-fallback variants: prefer to miss an event than to
        // spuriously reload on an unclassifiable signal.
        assert!(classify_event(&event(EventKind::Any, vec!["/tmp/x.md"])).is_empty());
        assert!(classify_event(&event(EventKind::Other, vec!["/tmp/x.md"])).is_empty());
    }

    #[test]
    fn fan_out_one_event_per_path() {
        // A single notify event can describe multiple affected paths
        // (e.g., rename From + To). We emit one FileWatchEvent per
        // path so the consumer can dedupe / debounce uniformly.
        let ev = event(
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            vec!["/tmp/a.md", "/tmp/b.md"],
        );
        assert_eq!(
            classify_event(&ev),
            vec![
                FileWatchEvent::Changed {
                    path: PathBuf::from("/tmp/a.md")
                },
                FileWatchEvent::Changed {
                    path: PathBuf::from("/tmp/b.md")
                },
            ]
        );
    }

    #[test]
    fn event_with_no_paths_yields_nothing() {
        // notify may emit kernel-overflow events with empty `paths`.
        // We have nothing to report to App in that case (an Error
        // would be more appropriate, but it arrives via the Result
        // branch of the handler, not via Event::paths).
        let ev = event(
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            vec![],
        );
        assert!(classify_event(&ev).is_empty());
    }

    #[test]
    fn start_and_drop_does_not_panic() {
        // The integration-y test: build a watcher with the real notify
        // backend, drop it. This exercises `notify::recommended_watcher`
        // creation on the host OS (FSEvents on macOS / inotify on Linux
        // / ReadDirectoryChanges on Windows). No paths are registered,
        // so no events should ever fire.
        let (tx, rx) = mpsc::channel::<FileWatchEvent>();
        let watcher = FileWatcher::start(tx, || {}).expect("start watcher");
        assert!(watcher.watched_paths().is_empty());
        drop(watcher);
        // Channel should still be drainable (and empty).
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn watch_and_unwatch_track_bookkeeping() {
        // Use tempfile so we have a real path the backend can register.
        // This is the only test in the module that touches the FS; it
        // doesn't wait for any events, only verifies watch/unwatch
        // succeed and the internal Vec stays in sync.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("watched.md");
        std::fs::write(&path, b"# hi").unwrap();

        let (tx, _rx) = mpsc::channel::<FileWatchEvent>();
        let mut watcher = FileWatcher::start(tx, || {}).expect("start watcher");
        watcher.watch(&path).expect("watch existing file");
        assert_eq!(watcher.watched_paths(), std::slice::from_ref(&path));

        // Adding the same path twice must not duplicate bookkeeping.
        watcher.watch(&path).expect("re-watch existing file");
        assert_eq!(watcher.watched_paths(), std::slice::from_ref(&path));

        watcher.unwatch(&path).expect("unwatch");
        assert!(watcher.watched_paths().is_empty());
    }

    #[test]
    fn reload_decision_is_idle_without_deadline() {
        assert_eq!(reload_decision(None, Instant::now()), ReloadStep::Idle);
    }

    #[test]
    fn reload_decision_fires_after_deadline() {
        let now = Instant::now();
        let past = now.checked_sub(Duration::from_millis(5)).unwrap();
        assert_eq!(reload_decision(Some(past), now), ReloadStep::Fire);
    }

    #[test]
    fn reload_decision_waits_before_deadline() {
        let now = Instant::now();
        let future = now + Duration::from_millis(30);
        match reload_decision(Some(future), now) {
            ReloadStep::Wait { remaining } => {
                assert!(
                    remaining <= Duration::from_millis(30) && remaining > Duration::from_millis(0),
                    "remaining out of range: {remaining:?}",
                );
            }
            other => panic!("expected Wait, got {other:?}"),
        }
    }

    #[test]
    fn reload_decision_fires_exactly_at_deadline() {
        // `now >= deadline` is inclusive — at the boundary, fire.
        let now = Instant::now();
        assert_eq!(reload_decision(Some(now), now), ReloadStep::Fire);
    }

    #[test]
    fn paths_match_exact_equality_short_circuits() {
        // No syscall hits when the strings already match — guard
        // against a future refactor accidentally always-canonicalizing.
        // `/etc/does-not-exist` would fail canonicalize, so equality
        // returning true proves the short-circuit path fired.
        let p = PathBuf::from("/etc/does-not-exist/never");
        assert!(paths_match(&p, &p));
    }

    #[test]
    fn paths_match_canonicalizes_symlinks() {
        // macOS exposes the real `/private/var/...` for an
        // FSEvents-delivered `/var/...` path; tempfile sits in
        // `/var/folders/...` which is itself a symlink on macOS, so
        // canonicalizing a relative reference into that directory
        // verifies the fallback branch.
        let dir = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        let file = canonical.join("hit.md");
        std::fs::write(&file, b"x").unwrap();
        // Re-form the path with a "./" prefix; logically the same
        // file, byte-different path string.
        let alt = canonical.join(".").join("hit.md");
        assert!(paths_match(&file, &alt), "{file:?} should match {alt:?}",);
    }

    #[test]
    fn paths_match_returns_false_for_different_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        assert!(!paths_match(&a, &b));
    }

    #[test]
    fn paths_match_returns_false_for_missing_files() {
        // Both paths fail canonicalize → cannot prove equality →
        // return false. The App will see a Removed event for the
        // original path separately, so this is safe.
        let p1 = PathBuf::from("/does/not/exist/a");
        let p2 = PathBuf::from("/does/not/exist/b");
        assert!(!paths_match(&p1, &p2));
    }

    #[test]
    fn unwatch_all_clears_bookkeeping() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();

        let (tx, _rx) = mpsc::channel::<FileWatchEvent>();
        let mut watcher = FileWatcher::start(tx, || {}).expect("start watcher");
        watcher.watch(&a).unwrap();
        watcher.watch(&b).unwrap();
        assert_eq!(watcher.watched_paths().len(), 2);

        watcher.unwatch_all();
        assert!(watcher.watched_paths().is_empty());
    }

    // ----- Phase 6.2: project-tree filter helpers ---------------

    #[test]
    fn is_excluded_dir_matches_listed_names() {
        for name in EXCLUDED_DIRS {
            assert!(is_excluded_dir(name), "{name} should be excluded");
        }
    }

    #[test]
    fn is_excluded_dir_misses_unrelated_names() {
        for name in ["docs", "src", "tests", ".github", "node-modules"] {
            // `.github` and `node-modules` look close to excluded
            // entries but are distinct; verify we don't false-positive.
            assert!(!is_excluded_dir(name), "{name} should not be excluded");
        }
    }

    #[test]
    fn is_markdown_path_is_case_insensitive() {
        for path in ["a.md", "A.MD", "guide.markdown", "Guide.Markdown"] {
            assert!(
                is_markdown_path(Path::new(path)),
                "{path} should classify as markdown",
            );
        }
    }

    #[test]
    fn is_markdown_path_rejects_other_extensions() {
        for path in ["a.txt", "a", "a.markdownx", "a.md.bak"] {
            assert!(
                !is_markdown_path(Path::new(path)),
                "{path} should not classify as markdown",
            );
        }
    }

    #[test]
    fn excluded_subtree_filters_paths_inside_skipped_dirs() {
        let root = Path::new("/proj");
        assert!(is_in_excluded_subtree(Path::new("/proj/.git/config"), root,));
        assert!(is_in_excluded_subtree(
            Path::new("/proj/node_modules/foo/index.md"),
            root,
        ));
        assert!(is_in_excluded_subtree(
            Path::new("/proj/sub/target/notes.md"),
            root,
        ));
    }

    #[test]
    fn excluded_subtree_does_not_filter_root_level_files() {
        // A *file* named ".git" at root is improbable but allowed —
        // the filter only fires on intermediate directory components.
        let root = Path::new("/proj");
        assert!(!is_in_excluded_subtree(Path::new("/proj/README.md"), root));
        assert!(!is_in_excluded_subtree(
            Path::new("/proj/docs/guide.md"),
            root,
        ));
    }

    #[test]
    fn excluded_subtree_rejects_paths_outside_root() {
        let root = Path::new("/proj");
        // notify may deliver events with paths outside the watch
        // root (rename From / To). We treat those as not-our-subtree
        // and the project classifier drops them via the markdown
        // check anyway, but verify the predicate behavior.
        assert!(!is_in_excluded_subtree(Path::new("/other/place.md"), root,));
    }

    #[test]
    fn project_classifier_emits_markdown_changes_under_root() {
        let root = Path::new("/proj");
        let ev = event(
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            vec!["/proj/docs/guide.md"],
        );
        assert_eq!(
            classify_project_event(&ev, root),
            vec![FileWatchEvent::Changed {
                path: PathBuf::from("/proj/docs/guide.md"),
            }]
        );
    }

    #[test]
    fn project_classifier_skips_non_markdown_files() {
        let root = Path::new("/proj");
        let ev = event(
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            vec!["/proj/src/main.rs"],
        );
        assert!(classify_project_event(&ev, root).is_empty());
    }

    #[test]
    fn project_classifier_skips_excluded_subtrees() {
        let root = Path::new("/proj");
        // A markdown file under .git/ would be a weird artifact, but
        // we should still ignore it — same with node_modules.
        let ev = event(
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            vec![
                "/proj/.git/COMMIT_EDITMSG.md",
                "/proj/node_modules/pkg/README.md",
            ],
        );
        assert!(classify_project_event(&ev, root).is_empty());
    }

    #[test]
    fn project_classifier_routes_remove_kind_to_removed() {
        let root = Path::new("/proj");
        let ev = event(EventKind::Remove(RemoveKind::File), vec!["/proj/old.md"]);
        assert_eq!(
            classify_project_event(&ev, root),
            vec![FileWatchEvent::Removed {
                path: PathBuf::from("/proj/old.md"),
            }]
        );
    }

    #[test]
    fn is_image_path_recognizes_all_supported_extensions() {
        for path in [
            "a.png", "A.PNG", "b.jpg", "b.jpeg", "B.JPEG", "c.gif", "d.webp", "e.svg", "X.SVG",
        ] {
            assert!(is_image_path(Path::new(path)), "{path} should be image");
        }
    }

    #[test]
    fn is_image_path_rejects_non_image_extensions() {
        for path in ["a.bmp", "a.tiff", "a.md", "a", "a.png.bak"] {
            assert!(
                !is_image_path(Path::new(path)),
                "{path} should not be image",
            );
        }
    }

    #[test]
    fn project_classifier_emits_image_changes() {
        let root = Path::new("/proj");
        let ev = event(
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            vec!["/proj/assets/hero.png"],
        );
        assert_eq!(
            classify_project_event(&ev, root),
            vec![FileWatchEvent::Changed {
                path: PathBuf::from("/proj/assets/hero.png"),
            }]
        );
    }

    #[test]
    fn project_classifier_emits_mixed_md_and_image_changes() {
        let root = Path::new("/proj");
        let ev = event(
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            vec!["/proj/docs/guide.md", "/proj/assets/diagram.svg"],
        );
        assert_eq!(
            classify_project_event(&ev, root),
            vec![
                FileWatchEvent::Changed {
                    path: PathBuf::from("/proj/docs/guide.md"),
                },
                FileWatchEvent::Changed {
                    path: PathBuf::from("/proj/assets/diagram.svg"),
                },
            ]
        );
    }

    #[test]
    fn project_classifier_drops_access_events() {
        let root = Path::new("/proj");
        let ev = event(
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            vec!["/proj/guide.md"],
        );
        assert!(classify_project_event(&ev, root).is_empty());
    }

    #[test]
    fn project_watcher_attaches_to_recursive_root() {
        // Real notify backend smoke test (no event timing — we just
        // verify start + attach succeed against a real directory
        // tree). Mirrors the FileWatcher start_and_drop test.
        let dir = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(canonical.join("docs")).unwrap();
        std::fs::write(canonical.join("docs/guide.md"), b"# g").unwrap();

        let (tx, _rx) = mpsc::channel::<FileWatchEvent>();
        let watcher =
            ProjectWatcher::start(canonical.clone(), tx, || {}).expect("start project watcher");
        assert_eq!(watcher.watched_root(), canonical.as_path());
    }
}
