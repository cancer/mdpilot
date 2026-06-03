// Phase 6.4 will use `initial_file`. Phase 6.5 will pass `root` into
// the `MDPILOT_PROJECT_ROOT` env var for claude. Phase 6.1 only feeds
// `root` into `spawn_session`, so until those phases the struct
// surface looks half-unused.
#![allow(dead_code)]

//! Resolve the project root (and optional initial preview file)
//! from a single CLI positional argument.
//!
//! Per `docs/claude-integration.md` §2:
//!
//! - `mdpilot <project-dir>` — root = that directory.
//! - `mdpilot <file.md>` — root = parent dir, initial preview = file.
//! - `mdpilot` (no arg) — Phase 7.1 will show a selection dialog.
//!   Until that lands, MVP falls back to the current working
//!   directory and logs the fallback at info level.
//!
//! Errors map to a single non-exhaustive enum so callers (currently
//! only `main.rs` and `App::new`) get a clear message without
//! depending on `std::io::Error` shapes.

use std::path::{Path, PathBuf};

/// Resolved project bootstrap state. `initial_file` is `Some` exactly
/// when the user passed a *file* path on the command line; passing a
/// directory (or nothing) leaves it `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInit {
    /// Absolute (canonicalized) project root. Used as cwd for the
    /// claude child process (`docs/claude-integration.md` §3) and
    /// as the recursion base for Phase 6.2's project watcher.
    pub root: PathBuf,
    /// Absolute path to the initial preview file when the CLI arg
    /// was a file. Phase 6.4 consumes this on startup; Phase 6.1
    /// only carries it through.
    pub initial_file: Option<PathBuf>,
}

/// Errors that block startup. mdpilot exits with a clear stderr
/// message on any of these; we never silently fall back from an
/// invalid arg to cwd, because that would mask user typos.
#[derive(Debug, thiserror::Error)]
pub enum ProjectInitError {
    /// CLI arg pointed at a path that doesn't exist (or isn't
    /// readable enough to canonicalize). Distinguished from `Io`
    /// because it's the common typo case and warrants a short
    /// user-facing message.
    #[error("指定されたパスが見つかりません: {}", .0.display())]
    NotFound(PathBuf),
    /// The arg exists but is neither a file nor a directory (e.g.,
    /// a socket or FIFO). Treated as a hard error rather than
    /// guessing.
    #[error("ファイルでもディレクトリでもないパスです: {}", .0.display())]
    Unsupported(PathBuf),
    /// Any other I/O failure (permission denied, etc.) — pass the
    /// path being processed plus the underlying os message.
    #[error("パスを解決できません ({}): {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// `std::env::current_dir()` failed in the no-arg fallback.
    #[error("カレントディレクトリを取得できません: {0}")]
    CurrentDir(#[source] std::io::Error),
}

/// Public entry point. Hits the filesystem to canonicalize and stat
/// the positional arg.
pub fn resolve(positional: Option<&Path>) -> Result<ProjectInit, ProjectInitError> {
    match positional {
        None => resolve_no_arg(),
        Some(p) => resolve_path(p),
    }
}

fn resolve_no_arg() -> Result<ProjectInit, ProjectInitError> {
    let cwd = std::env::current_dir().map_err(ProjectInitError::CurrentDir)?;
    let canonical = std::fs::canonicalize(&cwd).map_err(|source| ProjectInitError::Io {
        path: cwd.clone(),
        source,
    })?;
    tracing::info!(
        root = %canonical.display(),
        "no positional argument supplied; defaulting to cwd until the Phase 7.1 selection dialog lands",
    );
    Ok(ProjectInit {
        root: canonical,
        initial_file: None,
    })
}

fn resolve_path(positional: &Path) -> Result<ProjectInit, ProjectInitError> {
    // `canonicalize` returns NotFound when the path is missing OR
    // when an intermediate component is missing. We surface both as
    // ProjectInitError::NotFound — neither distinction is useful
    // to the user at this layer.
    let canonical = std::fs::canonicalize(positional).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ProjectInitError::NotFound(positional.to_path_buf())
        } else {
            ProjectInitError::Io {
                path: positional.to_path_buf(),
                source: err,
            }
        }
    })?;

    let metadata = std::fs::metadata(&canonical).map_err(|source| ProjectInitError::Io {
        path: canonical.clone(),
        source,
    })?;

    if metadata.is_dir() {
        Ok(ProjectInit {
            root: canonical,
            initial_file: None,
        })
    } else if metadata.is_file() {
        // `canonical.parent()` cannot reasonably be `None` here: a
        // file path always has at least one path component
        // preceding it (`/foo`'s parent is `/`), and `canonicalize`
        // is absolute so the result is always rooted. Treat the
        // theoretical None as Unsupported rather than panicking.
        let parent = canonical
            .parent()
            .ok_or_else(|| ProjectInitError::Unsupported(canonical.clone()))?;
        Ok(ProjectInit {
            root: parent.to_path_buf(),
            initial_file: Some(canonical),
        })
    } else {
        Err(ProjectInitError::Unsupported(canonical))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn missing_path_returns_not_found() {
        let err = resolve(Some(Path::new("/this/should/not/exist/at-all-12345"))).unwrap_err();
        match err {
            ProjectInitError::NotFound(p) => {
                assert_eq!(p, PathBuf::from("/this/should/not/exist/at-all-12345"))
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn directory_arg_becomes_root_without_initial_file() {
        let dir = tempfile::tempdir().unwrap();
        let init = resolve(Some(dir.path())).unwrap();
        // Compare via canonicalize because tempdir on macOS lives
        // under /var → /private/var symlink.
        assert_eq!(init.root, fs::canonicalize(dir.path()).unwrap());
        assert_eq!(init.initial_file, None);
    }

    #[test]
    fn file_arg_makes_parent_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        fs::write(&file, b"# notes").unwrap();

        let init = resolve(Some(&file)).unwrap();

        let expected_root = fs::canonicalize(dir.path()).unwrap();
        let expected_file = fs::canonicalize(&file).unwrap();
        assert_eq!(init.root, expected_root);
        assert_eq!(init.initial_file, Some(expected_file));
    }

    #[test]
    fn no_arg_falls_back_to_cwd() {
        // `current_dir()` for cargo tests is the crate root, which
        // exists. We don't assert the specific path; just that
        // resolution succeeds and `initial_file` is None.
        let init = resolve(None).unwrap();
        assert!(init.root.is_absolute(), "cwd must be absolute: {init:?}");
        assert_eq!(init.initial_file, None);
    }

    #[test]
    fn nested_file_path_resolves_root_to_its_parent_not_grandparent() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("subdir");
        fs::create_dir(&nested).unwrap();
        let file = nested.join("guide.md");
        fs::write(&file, b"# g").unwrap();

        let init = resolve(Some(&file)).unwrap();

        let expected_root = fs::canonicalize(&nested).unwrap();
        assert_eq!(
            init.root, expected_root,
            "root must be the immediate parent dir, not the tempdir grandparent",
        );
    }
}
