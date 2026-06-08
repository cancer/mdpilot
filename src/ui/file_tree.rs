//! Project file tree side-panel (Phase 9.X.4 + 10.9 keynav).
//!
//! Renders directories under `project_root` recursively, hiding the
//! same dirs the `ProjectWatcher` already filters out, and showing
//! only `.md` / `.markdown` files as openable entries.
//!
//! Phase 10.9 reworked the implementation: instead of leaning on
//! `egui::CollapsingHeader` (which only responds to mouse clicks),
//! we maintain our own `FileTreeState` with a flat list of currently
//! visible entries, an expansion `HashSet<PathBuf>`, and a `selected`
//! index. That lets `j/k/Enter/Space/Esc` drive the tree from the
//! keyboard. Mouse clicks still work — they just route through the
//! same state.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::preview::watcher::{is_excluded_dir, is_markdown_path};

/// User intent emitted by the tree for the current frame.
#[derive(Debug, PartialEq, Eq)]
pub enum FileTreeAction {
    None,
    Open(PathBuf),
    /// Phase 10.9: user pressed Esc → focus should go back to the
    /// preview pane. The caller flips `tree_focused = false`.
    ExitToPreview,
}

/// Phase 10.9: tree state persisted across frames.
#[derive(Debug, Default)]
pub struct FileTreeState {
    /// Absolute paths of directories whose children are currently
    /// rendered. Other dirs are collapsed.
    pub expanded: HashSet<PathBuf>,
    /// Index into the per-frame flat-entries list. Wraps clamped
    /// to the list length on every render.
    pub selected: usize,
    /// When `true`, the tree consumes j/k/Enter/Space/Esc this frame.
    /// Drives both keynav and the row background highlight.
    pub focused: bool,
}

#[derive(Debug, Clone)]
struct FlatEntry {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
    /// Tracked separately from `state.expanded` so the row label can
    /// show the right indicator (▾ / ▸) without re-querying the set.
    expanded: bool,
}

/// Draw the file tree rooted at `root`. Returns the action the user
/// triggered this frame (a file open or an Esc).
pub fn show(ui: &mut egui::Ui, root: &Path, state: &mut FileTreeState) -> FileTreeAction {
    let mut action = FileTreeAction::None;
    let flat = build_flat_entries(root, &state.expanded);
    if flat.is_empty() {
        ui.heading("ファイル");
        ui.separator();
        ui.weak("（空のプロジェクト）");
        return action;
    }
    if state.selected >= flat.len() {
        state.selected = flat.len() - 1;
    }

    // Keyboard handling. We do this *before* drawing so the new
    // selection / expansion state is reflected in this frame's UI.
    if state.focused {
        if let Some(next_action) = handle_keynav(ui.ctx(), &flat, state) {
            action = next_action;
        }
    }

    ui.heading("ファイル");
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (idx, entry) in flat.iter().enumerate() {
                let row_action = draw_entry(ui, entry, idx, state);
                match row_action {
                    FileTreeAction::Open(p) if matches!(action, FileTreeAction::None) => {
                        action = FileTreeAction::Open(p);
                    }
                    _ => {}
                }
            }
        });
    action
}

fn draw_entry(
    ui: &mut egui::Ui,
    entry: &FlatEntry,
    idx: usize,
    state: &mut FileTreeState,
) -> FileTreeAction {
    let name = entry
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<unknown>");
    let indicator = if entry.is_dir {
        if entry.expanded {
            "▾ "
        } else {
            "▸ "
        }
    } else {
        "  "
    };
    let indent = " ".repeat(entry.depth * 2);
    let label = format!("{indent}{indicator}{name}");
    let selected = idx == state.selected && state.focused;
    let resp = ui.selectable_label(selected, label);
    let mut out = FileTreeAction::None;
    if resp.clicked() {
        state.selected = idx;
        if entry.is_dir {
            toggle_expanded(state, &entry.path);
        } else {
            out = FileTreeAction::Open(entry.path.clone());
        }
    }
    if selected {
        // Make sure the focused row stays on screen when keynav
        // moves the selection off-viewport.
        resp.scroll_to_me(Some(egui::Align::Center));
    }
    out
}

/// Walk j/k/Enter/Space/Esc presses from this frame's input.
fn handle_keynav(
    ctx: &egui::Context,
    flat: &[FlatEntry],
    state: &mut FileTreeState,
) -> Option<FileTreeAction> {
    let mut action: Option<FileTreeAction> = None;
    let events = ctx.input(|i| i.events.clone());
    for event in events {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            continue;
        };
        if modifiers.any() {
            continue;
        }
        match key {
            egui::Key::J | egui::Key::ArrowDown => {
                if state.selected + 1 < flat.len() {
                    state.selected += 1;
                }
            }
            egui::Key::K | egui::Key::ArrowUp => {
                if state.selected > 0 {
                    state.selected -= 1;
                }
            }
            egui::Key::Enter => {
                if let Some(entry) = flat.get(state.selected) {
                    if entry.is_dir {
                        toggle_expanded(state, &entry.path);
                    } else {
                        action = Some(FileTreeAction::Open(entry.path.clone()));
                    }
                }
            }
            egui::Key::Space => {
                if let Some(entry) = flat.get(state.selected) {
                    if entry.is_dir {
                        toggle_expanded(state, &entry.path);
                    }
                }
            }
            egui::Key::Escape => {
                action = Some(FileTreeAction::ExitToPreview);
            }
            _ => {}
        }
    }
    action
}

fn toggle_expanded(state: &mut FileTreeState, path: &Path) {
    if !state.expanded.remove(path) {
        state.expanded.insert(path.to_path_buf());
    }
}

/// Walk the project tree top-down, producing a flat list of entries
/// that should be drawn this frame. Excluded directories never
/// appear; `.md` / `.markdown` files appear as leaves; directories
/// appear with an expand indicator and their children follow if
/// `expanded` includes them.
fn build_flat_entries(root: &Path, expanded: &HashSet<PathBuf>) -> Vec<FlatEntry> {
    let mut out = Vec::new();
    walk(root, 0, expanded, &mut out);
    out
}

fn walk(dir: &Path, depth: usize, expanded: &HashSet<PathBuf>, out: &mut Vec<FlatEntry>) {
    let entries = match read_dir_sorted(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    // Directories first, then markdown files (mirrors the previous
    // CollapsingHeader-based layout).
    for (path, is_dir) in &entries {
        if !is_dir {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_excluded_dir(name) {
            continue;
        }
        let is_expanded = expanded.contains(path);
        out.push(FlatEntry {
            path: path.clone(),
            depth,
            is_dir: true,
            expanded: is_expanded,
        });
        if is_expanded {
            walk(path, depth + 1, expanded, out);
        }
    }
    for (path, is_dir) in &entries {
        if *is_dir {
            continue;
        }
        if !is_markdown_path(path) {
            continue;
        }
        out.push(FlatEntry {
            path: path.clone(),
            depth,
            is_dir: false,
            expanded: false,
        });
    }
}

fn read_dir_sorted(dir: &Path) -> std::io::Result<Vec<(PathBuf, bool)>> {
    let mut out: Vec<(PathBuf, bool)> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        out.push((path, is_dir));
    }
    out.sort_by(|a, b| {
        let a_name = a.0.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let b_name = b.0.file_name().and_then(|n| n.to_str()).unwrap_or("");
        a_name.to_lowercase().cmp(&b_name.to_lowercase())
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_dir_sorted_orders_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Zeta.md"), "").unwrap();
        fs::write(dir.path().join("alpha.md"), "").unwrap();
        fs::write(dir.path().join("beta.md"), "").unwrap();
        let entries = read_dir_sorted(dir.path()).unwrap();
        let names: Vec<&str> = entries
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["alpha.md", "beta.md", "Zeta.md"]);
    }

    #[test]
    fn read_dir_sorted_marks_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("subdir")).unwrap();
        fs::write(dir.path().join("file.md"), "").unwrap();
        let entries = read_dir_sorted(dir.path()).unwrap();
        let by_name: std::collections::HashMap<String, bool> = entries
            .into_iter()
            .map(|(p, d)| (p.file_name().unwrap().to_str().unwrap().to_string(), d))
            .collect();
        assert_eq!(by_name.get("subdir"), Some(&true));
        assert_eq!(by_name.get("file.md"), Some(&false));
    }

    #[test]
    fn build_flat_entries_collapsed_root() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("a.md"), "").unwrap();
        fs::write(dir.path().join("sub/inner.md"), "").unwrap();
        let flat = build_flat_entries(dir.path(), &HashSet::new());
        let names: Vec<_> = flat
            .iter()
            .map(|e| e.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["sub", "a.md"]);
    }

    #[test]
    fn build_flat_entries_expanded_subdir() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("a.md"), "").unwrap();
        fs::write(dir.path().join("sub/inner.md"), "").unwrap();
        let mut expanded = HashSet::new();
        expanded.insert(dir.path().join("sub"));
        let flat = build_flat_entries(dir.path(), &expanded);
        let names: Vec<_> = flat
            .iter()
            .map(|e| e.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["sub", "inner.md", "a.md"]);
        assert_eq!(flat[1].depth, 1);
    }

    #[test]
    fn toggle_expanded_flips_membership() {
        let mut state = FileTreeState::default();
        let p = PathBuf::from("/proj/sub");
        toggle_expanded(&mut state, &p);
        assert!(state.expanded.contains(&p));
        toggle_expanded(&mut state, &p);
        assert!(!state.expanded.contains(&p));
    }

    #[test]
    fn build_flat_entries_skips_excluded() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::create_dir(dir.path().join("node_modules")).unwrap();
        fs::create_dir(dir.path().join("docs")).unwrap();
        fs::write(dir.path().join("README.md"), "").unwrap();
        let flat = build_flat_entries(dir.path(), &HashSet::new());
        let names: Vec<_> = flat
            .iter()
            .map(|e| e.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["docs", "README.md"]);
    }
}
