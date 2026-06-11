//! Phase 10.11 (2026-06-11): VS Code Cmd+P-style fuzzy file picker.
//!
//! `nucleo-matcher::Pattern::match_list` scores a flat list of
//! relative paths against the user's query, sorted high-score first.
//! The modal is built into a Window that anchors center-top and
//! consumes its own keyboard input (TextEdit + j/k/Enter/Esc).

use std::path::{Path, PathBuf};

use eframe::egui;
use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config, Matcher,
};

use crate::preview::watcher::{is_excluded_dir, is_markdown_path};

const RESULT_LIMIT: usize = 50;

#[derive(Debug, PartialEq, Eq)]
pub enum FuzzyFinderAction {
    None,
    Open(PathBuf),
    Close,
}

/// One frame's intent.
pub struct FuzzyFinderState {
    pub query: String,
    /// Project-root-relative candidate paths. Rebuilt lazily on
    /// `refresh`; kept across frames so typing doesn't re-walk the
    /// filesystem on every keystroke.
    pub candidates: Vec<RelPath>,
    pub selected: usize,
    /// `true` for one frame after open: the TextEdit grabs focus.
    pub focus_requested: bool,
}

/// One candidate. Holds both the relative-path string (for display
/// + matching) and the absolute path (for actually opening on Enter).
#[derive(Debug, Clone)]
pub struct RelPath {
    pub relative: String,
    pub absolute: PathBuf,
}

impl AsRef<str> for RelPath {
    fn as_ref(&self) -> &str {
        &self.relative
    }
}

impl FuzzyFinderState {
    /// Build by walking `root` recursively for `.md` files, skipping
    /// the same excluded dirs the `ProjectWatcher` filters.
    pub fn open(root: &Path) -> Self {
        let mut candidates = Vec::new();
        walk(root, root, &mut candidates);
        // Stable initial order: alphabetical by relative path so the
        // empty-query case lists the project in a predictable order.
        candidates.sort_by(|a, b| a.relative.cmp(&b.relative));
        Self {
            query: String::new(),
            candidates,
            selected: 0,
            focus_requested: true,
        }
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<RelPath>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if is_excluded_dir(name) {
                continue;
            }
            walk(root, &path, out);
        } else if is_markdown_path(&path) {
            if let Ok(rel) = path.strip_prefix(root) {
                let rel_str = rel.to_string_lossy().into_owned();
                out.push(RelPath {
                    relative: rel_str,
                    absolute: path,
                });
            }
        }
    }
}

pub fn show(ctx: &egui::Context, state: &mut FuzzyFinderState) -> FuzzyFinderAction {
    let mut action = FuzzyFinderAction::None;
    let mut open = true;

    // Handle keynav before drawing so the modal reflects the move
    // (j/k) or the close (Esc) on the same frame. Enter is also
    // handled here so the TextEdit doesn't insert a literal newline.
    let events = ctx.input(|i| i.events.clone());
    // Score first so we can clamp `selected` and route Enter to the
    // right candidate.
    let scored = score(&state.candidates, &state.query);
    let scored_count = scored.len();
    if scored_count == 0 {
        state.selected = 0;
    } else if state.selected >= scored_count {
        state.selected = scored_count - 1;
    }
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
        match (key, modifiers.command, modifiers.shift) {
            (egui::Key::Escape, _, _) => {
                action = FuzzyFinderAction::Close;
            }
            (egui::Key::Enter, _, _) => {
                if let Some((path, _)) = scored.get(state.selected) {
                    action = FuzzyFinderAction::Open(path.absolute.clone());
                }
            }
            (egui::Key::ArrowDown, _, _) | (egui::Key::N, true, false)
                if state.selected + 1 < scored_count =>
            {
                state.selected += 1;
            }
            (egui::Key::ArrowUp, _, _) | (egui::Key::P, true, false) if state.selected > 0 => {
                state.selected -= 1;
            }
            _ => {}
        }
    }

    egui::Window::new("ファイルを開く")
        .open(&mut open)
        .resizable(true)
        .default_width(560.0)
        .default_height(440.0)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
        .show(ctx, |ui| {
            let input_id = egui::Id::new("fuzzy_finder_input");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .id(input_id)
                    .hint_text("ファイル名で絞り込み… (Esc で閉じる)")
                    .desired_width(f32::INFINITY),
            );
            if state.focus_requested {
                resp.request_focus();
                state.focus_requested = false;
            }
            ui.separator();
            if state.candidates.is_empty() {
                ui.weak("このプロジェクトに .md ファイルがありません。");
                return;
            }
            ui.weak(format!("{} / {} 件", scored_count, state.candidates.len()));
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (idx, (rel, _score)) in scored.iter().take(RESULT_LIMIT).enumerate() {
                    let selected = idx == state.selected;
                    let label = egui::RichText::new(&rel.relative).monospace();
                    let resp = ui.selectable_label(selected, label);
                    if resp.clicked() {
                        state.selected = idx;
                        action = FuzzyFinderAction::Open(rel.absolute.clone());
                    }
                    if selected {
                        resp.scroll_to_me(Some(egui::Align::Center));
                    }
                }
            });
        });

    if !open && matches!(action, FuzzyFinderAction::None) {
        action = FuzzyFinderAction::Close;
    }
    action
}

/// Score `candidates` against `query` using nucleo. Empty query
/// returns candidates in their stored order with score 0 so the
/// just-opened modal shows the full list.
fn score(candidates: &[RelPath], query: &str) -> Vec<(RelPath, u32)> {
    if query.is_empty() {
        return candidates.iter().cloned().map(|c| (c, 0)).collect();
    }
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    pattern.match_list(candidates.iter().cloned(), &mut matcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_all_candidates_in_order() {
        let candidates = vec![
            RelPath {
                relative: "a.md".into(),
                absolute: PathBuf::from("/p/a.md"),
            },
            RelPath {
                relative: "b.md".into(),
                absolute: PathBuf::from("/p/b.md"),
            },
        ];
        let scored = score(&candidates, "");
        assert_eq!(scored.len(), 2);
        assert_eq!(scored[0].0.relative, "a.md");
    }

    #[test]
    fn substring_query_filters_and_orders_by_score() {
        let candidates = vec![
            RelPath {
                relative: "alpha/notes.md".into(),
                absolute: PathBuf::from("/p/alpha/notes.md"),
            },
            RelPath {
                relative: "beta/notes.md".into(),
                absolute: PathBuf::from("/p/beta/notes.md"),
            },
            RelPath {
                relative: "alpha/readme.md".into(),
                absolute: PathBuf::from("/p/alpha/readme.md"),
            },
        ];
        let scored = score(&candidates, "alpha");
        assert!(!scored.is_empty(), "alpha should match alpha/* entries");
        assert!(scored.iter().any(|(c, _)| c.relative == "alpha/notes.md"));
        assert!(scored.iter().any(|(c, _)| c.relative == "alpha/readme.md"));
        assert!(!scored.iter().any(|(c, _)| c.relative == "beta/notes.md"));
    }

    #[test]
    fn case_insensitive_smart_matching() {
        let candidates = vec![RelPath {
            relative: "README.md".into(),
            absolute: PathBuf::from("/p/README.md"),
        }];
        let scored = score(&candidates, "readme");
        assert_eq!(scored.len(), 1, "lowercase needle should match uppercase");
    }

    #[test]
    fn walk_skips_excluded_dirs_and_non_md() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/foo")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("docs/a.md"), "").unwrap();
        std::fs::write(root.join("docs/b.txt"), "").unwrap();
        std::fs::write(root.join("node_modules/foo/c.md"), "").unwrap();
        std::fs::write(root.join(".git/d.md"), "").unwrap();
        std::fs::write(root.join("README.md"), "").unwrap();

        let mut out = Vec::new();
        walk(root, root, &mut out);
        let rels: Vec<&str> = out.iter().map(|c| c.relative.as_str()).collect();
        assert!(rels.contains(&"docs/a.md"));
        assert!(rels.contains(&"README.md"));
        assert!(!rels.iter().any(|r| r.starts_with("node_modules")));
        assert!(!rels.iter().any(|r| r.starts_with(".git")));
        assert!(!rels.iter().any(|r| r.ends_with(".txt")));
    }
}
