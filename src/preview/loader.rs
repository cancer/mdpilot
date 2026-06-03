// Wired into App in Phase 4.2 / 5.x together with the egui_commonmark
// renderer and notify watcher. Until then the API surface looks dead from
// the bin crate's perspective.
#![allow(dead_code)]

//! Markdown file loader for the preview pane.
//!
//! Per `docs/preview.md` §10, the loader classifies files by size:
//!
//! - `< 1 MiB`  — `SizeClass::Small`, render normally
//! - `1 .. 10 MiB` — `SizeClass::Large`, render but warn about
//!   single-frame stutter
//! - `>= 10 MiB` — `LoadError::TooLarge`, reject without reading
//!
//! The hard limit is enforced *before* we touch the file body: we stat the
//! file, compare against `HARD_LIMIT_BYTES`, and bail out without
//! allocating a 10 MiB+ buffer. The soft limit only affects the returned
//! `SizeClass`.
//!
//! The spec writes "1MB" / "10MB" casually; this module reads those as
//! MiB (1,048,576 / 10,485,760) since those are the natural boundaries
//! for buffer-allocation decisions. If the spec is ever tightened to mean
//! SI MB, only the two constants below change.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Boundary between `Small` and `Large` size classes. Files at or above
/// this size still render, but the UI should surface a warning per
/// `docs/preview.md` §10.
pub const SOFT_LIMIT_BYTES: u64 = 1024 * 1024;

/// Files at or above this size are rejected without reading.
pub const HARD_LIMIT_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedDocument {
    pub path: PathBuf,
    pub text: String,
    pub size_bytes: u64,
    pub size_class: SizeClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeClass {
    Small,
    Large,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    NotFound,
    PermissionDenied,
    /// `fs::read_to_string` returns `io::ErrorKind::InvalidData` exactly
    /// when the file is not valid UTF-8. We surface that as a distinct
    /// variant so the UI can show a specific message.
    NotUtf8,
    /// File size is at or above `HARD_LIMIT_BYTES`. The body was not read.
    TooLarge {
        size_bytes: u64,
    },
    /// Catch-all for everything else (NotADirectory, FileTooLarge from the
    /// kernel, etc.). The string is `io::Error::to_string()` so the call
    /// site doesn't have to depend on `std::io`.
    Io(String),
}

/// Load `path` as a Markdown document. See module docs for the size-class
/// contract.
pub fn load_markdown(path: &Path) -> Result<LoadedDocument, LoadError> {
    load_with_limits(path, SOFT_LIMIT_BYTES, HARD_LIMIT_BYTES)
}

/// Same as [`load_markdown`] but with caller-supplied size limits. Public
/// only to the crate so tests can exercise the `Large` and `TooLarge`
/// branches without writing a 10 MiB file.
pub(crate) fn load_with_limits(
    path: &Path,
    soft_limit_bytes: u64,
    hard_limit_bytes: u64,
) -> Result<LoadedDocument, LoadError> {
    let metadata = fs::metadata(path).map_err(classify_io_error)?;
    let size_bytes = metadata.len();
    if size_bytes >= hard_limit_bytes {
        return Err(LoadError::TooLarge { size_bytes });
    }

    let text = fs::read_to_string(path).map_err(classify_io_error)?;
    let size_class = if size_bytes >= soft_limit_bytes {
        SizeClass::Large
    } else {
        SizeClass::Small
    };
    Ok(LoadedDocument {
        path: path.to_path_buf(),
        text,
        size_bytes,
        size_class,
    })
}

fn classify_io_error(err: io::Error) -> LoadError {
    match err.kind() {
        io::ErrorKind::NotFound => LoadError::NotFound,
        io::ErrorKind::PermissionDenied => LoadError::PermissionDenied,
        io::ErrorKind::InvalidData => LoadError::NotUtf8,
        _ => LoadError::Io(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    fn write_temp(dir: &tempfile::TempDir, name: &str, contents: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        let mut f = File::create(&path).unwrap();
        f.write_all(contents).unwrap();
        f.sync_all().unwrap();
        path
    }

    #[test]
    fn small_utf8_file_loads_with_small_class() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(
            &dir,
            "tiny.md",
            b"# Hello\n\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e",
        );
        let doc = load_markdown(&path).unwrap();
        assert_eq!(doc.path, path);
        assert!(doc.text.starts_with("# Hello"));
        assert!(doc.text.contains("日本語"));
        assert!(doc.size_bytes > 0);
        assert!(doc.size_bytes < SOFT_LIMIT_BYTES);
        assert_eq!(doc.size_class, SizeClass::Small);
    }

    #[test]
    fn empty_file_is_small() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(&dir, "empty.md", b"");
        let doc = load_markdown(&path).unwrap();
        assert_eq!(doc.text, "");
        assert_eq!(doc.size_bytes, 0);
        assert_eq!(doc.size_class, SizeClass::Small);
    }

    #[test]
    fn at_soft_limit_classifies_as_large() {
        // Inject a tiny soft limit so we don't have to write a real 1 MiB
        // file; the size-class branch we care about is purely numeric.
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(&dir, "med.md", &[b'a'; 100]);
        let doc = load_with_limits(&path, 50, 1_000).unwrap();
        assert_eq!(doc.size_bytes, 100);
        assert_eq!(doc.size_class, SizeClass::Large);
    }

    #[test]
    fn just_below_soft_limit_is_small() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(&dir, "med.md", &[b'a'; 49]);
        let doc = load_with_limits(&path, 50, 1_000).unwrap();
        assert_eq!(doc.size_class, SizeClass::Small);
    }

    #[test]
    fn at_hard_limit_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(&dir, "huge.md", &[b'a'; 1_000]);
        let err = load_with_limits(&path, 50, 1_000).unwrap_err();
        match err {
            LoadError::TooLarge { size_bytes } => assert_eq!(size_bytes, 1_000),
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn just_below_hard_limit_loads_as_large() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(&dir, "almost.md", &[b'a'; 999]);
        let doc = load_with_limits(&path, 50, 1_000).unwrap();
        assert_eq!(doc.size_class, SizeClass::Large);
        assert_eq!(doc.size_bytes, 999);
    }

    #[test]
    fn too_large_does_not_read_body() {
        // The hard-limit branch must not allocate; we cannot directly
        // observe that, but we can prove it doesn't *fail* on a binary
        // payload that would trip the UTF-8 check if read.
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(&dir, "binary.md", &[0xff, 0xfe, 0xfd, 0xfc]);
        let err = load_with_limits(&path, 1, 3).unwrap_err();
        assert!(
            matches!(err, LoadError::TooLarge { .. }),
            "TooLarge should win over NotUtf8: {err:?}",
        );
    }

    #[test]
    fn missing_path_returns_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.md");
        let err = load_markdown(&path).unwrap_err();
        assert_eq!(err, LoadError::NotFound);
    }

    #[test]
    fn non_utf8_file_returns_not_utf8() {
        let dir = tempfile::tempdir().unwrap();
        // Lone continuation byte: not valid UTF-8 in any position.
        let path = write_temp(&dir, "binary.md", &[0xff, 0xfe, 0xfd, 0xfc]);
        let err = load_markdown(&path).unwrap_err();
        assert_eq!(err, LoadError::NotUtf8);
    }

    #[cfg(unix)]
    #[test]
    fn permission_denied_is_classified() {
        use std::os::unix::fs::PermissionsExt;

        // Root bypasses unix mode bits, so the test premise doesn't hold.
        // Skip up front instead of running the body and conditionally
        // accepting a successful read.
        if is_unix_root() {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(&dir, "locked.md", b"secret");
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&path, perms).unwrap();

        let result = load_markdown(&path);

        // Restore perms before asserting so tempdir cleanup succeeds even
        // if the assertion below fails.
        let mut restore = fs::metadata(&path).unwrap().permissions();
        restore.set_mode(0o600);
        fs::set_permissions(&path, restore).unwrap();

        assert_eq!(result, Err(LoadError::PermissionDenied));
    }

    #[cfg(unix)]
    fn is_unix_root() -> bool {
        // Safety: geteuid is a leaf syscall with no preconditions.
        unsafe { libc::geteuid() == 0 }
    }
}
