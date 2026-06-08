//! Project file tree side-panel (Phase 9.X.4).
//!
//! Renders directories under `project_root` recursively, hiding the
//! same dirs the `ProjectWatcher` already filters out (`.git`,
//! `node_modules`, etc.) and showing only `.md` / `.markdown` files
//! as clickable entries. Returns the clicked file via
//! `FileTreeAction::Open(path)` so the caller can hand the path off
//! to `loader::load_markdown` + `preview.set_document`.
//!
//! Subdirectory expansion state lives in egui's `CollapsingHeader`
//! memory keyed by the absolute path, so it survives panel hide/show
//! and is per-egui-context.

use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::preview::watcher::{is_excluded_dir, is_markdown_path};

/// User intent emitted by the tree for the current frame.
#[derive(Debug, PartialEq, Eq)]
pub enum FileTreeAction {
    None,
    Open(PathBuf),
}

/// Draw the file tree rooted at `root`. The widget fills whatever
/// width the caller gives it. Returns `Open(path)` when a `.md` file
/// is clicked this frame.
pub fn show(ui: &mut egui::Ui, root: &Path) -> FileTreeAction {
    let mut action = FileTreeAction::None;
    ui.heading("ファイル");
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            draw_dir(ui, root, &mut action);
        });
    action
}

/// Render the contents of `dir` directly into `ui` (no surrounding
/// CollapsingHeader). Used for the root and recursively for every
/// expanded subdirectory.
fn draw_dir(ui: &mut egui::Ui, dir: &Path, action: &mut FileTreeAction) {
    let entries = match read_dir_sorted(dir) {
        Ok(e) => e,
        Err(err) => {
            ui.colored_label(
                egui::Color32::from_rgb(220, 90, 80),
                format!("読み込みエラー: {err}"),
            );
            return;
        }
    };

    // Directories first, then markdown files. We pre-classify so we
    // don't have to read metadata twice per entry.
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
        let header = egui::CollapsingHeader::new(name)
            .id_salt(path)
            .default_open(false);
        header.show(ui, |ui| draw_dir(ui, path, action));
    }
    for (path, is_dir) in &entries {
        if *is_dir {
            continue;
        }
        if !is_markdown_path(path) {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unknown>");
        if ui.selectable_label(false, name).clicked() {
            *action = FileTreeAction::Open(path.clone());
        }
    }
}

/// Read `dir` and return entries sorted ASCII-case-insensitively,
/// each tagged with whether it's a directory. Lookup of `is_dir`
/// happens here so the recursive `draw_dir` doesn't touch the
/// filesystem more than once per entry.
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
    fn file_tree_action_default_is_none() {
        // Sanity guard that variants don't collapse to identical PartialEq.
        let p = PathBuf::from("/x.md");
        assert_ne!(FileTreeAction::None, FileTreeAction::Open(p.clone()));
        assert_ne!(
            FileTreeAction::Open(PathBuf::from("/a.md")),
            FileTreeAction::Open(PathBuf::from("/b.md"))
        );
    }
}
