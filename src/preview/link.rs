// Wired into App in Phase 4.4. The dispatch path is reachable but the
// individual branches only run on real link clicks, so dead-code
// analysis won't see them all from unit tests alone.
#![allow(dead_code)]

//! Link click dispatcher for the preview pane.
//!
//! Per `docs/preview.md` §5, a click on a markdown link should:
//!
//! | scheme / shape                | action                        |
//! | ----------------------------- | ----------------------------- |
//! | `http://` / `https://` / `mailto:` / `data:` | OS default app (browser, mailer, …) |
//! | path ending in `.md` (rel or abs) | swap preview target          |
//! | other path (rel or abs)       | OS default app for the file   |
//! | `#anchor`                     | scroll within current preview |
//!
//! Relative paths resolve against the **current preview file's directory**,
//! not the process cwd. If no preview is loaded yet, relative paths fall
//! through to `OpenWithOsApp` with their raw text and the OS layer
//! decides what to do (usually nothing useful — but the spec doesn't
//! specify a richer fallback).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkAction {
    /// Hand the URL back to the OS — egui's `open_url` path is fine for
    /// `http(s)://`, `mailto:`, `data:`, etc. We keep the raw string
    /// rather than parse, since the OS opener does scheme dispatch too.
    External { url: String },
    /// Load this Markdown file into the preview pane.
    SwitchMarkdown { path: PathBuf },
    /// Hand this file path to the OS default application.
    OpenWithOsApp { path: PathBuf },
    /// Anchor link (`#section`). MVP scrolls within the current preview;
    /// see `docs/preview.md` §5. egui_commonmark doesn't expose heading
    /// anchors yet, so the App-side handler logs this as a no-op for now.
    Anchor { fragment: String },
    /// Empty link or whitespace-only — ignore.
    Empty,
}

/// Classify a markdown link destination. Pure so the URL-shape decisions
/// are unit-testable without involving the filesystem or eframe.
///
/// `current_dir` is the directory of the currently displayed Markdown
/// file (i.e. `PreviewState::Loaded::document.path.parent()`). `None`
/// means no document is loaded — relative paths then resolve to
/// `OpenWithOsApp` with their literal text and the OS decides.
pub fn classify(href: &str, current_dir: Option<&Path>) -> LinkAction {
    let trimmed = href.trim();
    if trimmed.is_empty() {
        return LinkAction::Empty;
    }

    if let Some(fragment) = trimmed.strip_prefix('#') {
        return LinkAction::Anchor {
            fragment: fragment.to_string(),
        };
    }

    if is_external_scheme(trimmed) {
        return LinkAction::External {
            url: trimmed.to_string(),
        };
    }

    // Everything else is treated as a filesystem path.
    let raw = Path::new(trimmed);
    let absolute = if raw.is_absolute() || is_windows_absolute(trimmed) {
        PathBuf::from(trimmed)
    } else if let Some(base) = current_dir {
        base.join(raw)
    } else {
        // No base to resolve against; pass the raw text through.
        PathBuf::from(trimmed)
    };

    if is_markdown_extension(&absolute) {
        LinkAction::SwitchMarkdown { path: absolute }
    } else {
        LinkAction::OpenWithOsApp { path: absolute }
    }
}

/// True for any href with a URL scheme egui's `open_url` (and therefore
/// the OS opener) handles natively. We match the schemes the spec calls
/// out explicitly (`http`, `https`, `mailto`, `data`) plus a few common
/// ones the OS opener already knows (`file`, `ftp`). Unknown schemes
/// (e.g. `magnet:`) fall through to the path branch — wrong for those,
/// but the spec doesn't cover them and the OS opener will reject the
/// path attempt with a visible error.
fn is_external_scheme(href: &str) -> bool {
    const SCHEMES: &[&str] = &[
        "http://", "https://", "mailto:", "data:", "file://", "ftp://",
    ];
    SCHEMES.iter().any(|s| href.starts_with(s))
}

/// `Path::is_absolute` returns false for Windows-style `C:\…` on Unix
/// builds. Cover that case manually so unit tests don't depend on the
/// host OS to mark `C:\foo.md` as absolute.
fn is_windows_absolute(href: &str) -> bool {
    let bytes = href.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

fn is_markdown_extension(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(ext) if ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn empty_link_is_empty() {
        assert_eq!(classify("", None), LinkAction::Empty);
        assert_eq!(classify("   ", None), LinkAction::Empty);
    }

    #[test]
    fn http_and_https_are_external() {
        assert_eq!(
            classify("https://example.com/page", None),
            LinkAction::External {
                url: "https://example.com/page".into()
            }
        );
        assert_eq!(
            classify("http://localhost:8080", None),
            LinkAction::External {
                url: "http://localhost:8080".into()
            }
        );
    }

    #[test]
    fn mailto_is_external() {
        assert_eq!(
            classify("mailto:hi@example.com", None),
            LinkAction::External {
                url: "mailto:hi@example.com".into()
            }
        );
    }

    #[test]
    fn data_url_is_external() {
        let href = "data:text/plain;base64,SGVsbG8=";
        assert_eq!(
            classify(href, None),
            LinkAction::External { url: href.into() }
        );
    }

    #[test]
    fn anchor_link_returns_anchor_variant() {
        assert_eq!(
            classify("#installation", None),
            LinkAction::Anchor {
                fragment: "installation".into()
            }
        );
        // Empty fragment is still an anchor (no scroll target, App
        // handler logs and moves on).
        assert_eq!(
            classify("#", None),
            LinkAction::Anchor {
                fragment: String::new()
            }
        );
    }

    #[test]
    fn relative_md_path_resolves_and_switches() {
        let action = classify("nested/guide.md", Some(&dir("/Users/me/docs")));
        assert_eq!(
            action,
            LinkAction::SwitchMarkdown {
                path: PathBuf::from("/Users/me/docs/nested/guide.md"),
            }
        );
    }

    #[test]
    fn relative_md_with_parent_segments_is_preserved_for_canonicalization_later() {
        // We don't canonicalize here — that requires hitting the
        // filesystem. The dispatcher passes the path with `..` to
        // load_markdown, which will fail with NotFound if the parent
        // path doesn't exist. That matches "spec security note: file
        // system permissions are the only gate."
        let action = classify("../sibling.md", Some(&dir("/a/b")));
        assert_eq!(
            action,
            LinkAction::SwitchMarkdown {
                path: PathBuf::from("/a/b/../sibling.md"),
            }
        );
    }

    #[test]
    fn absolute_unix_md_path_does_not_re_resolve() {
        let action = classify("/etc/notes/spec.md", Some(&dir("/Users/me/docs")));
        assert_eq!(
            action,
            LinkAction::SwitchMarkdown {
                path: PathBuf::from("/etc/notes/spec.md"),
            }
        );
    }

    #[test]
    fn absolute_windows_md_path_is_recognized_on_any_host() {
        // is_windows_absolute lets the unit tests pass on Unix CI
        // without `Path::is_absolute` semantics drifting.
        let action = classify("C:\\Users\\me\\notes.md", Some(&dir("/ignored")));
        assert_eq!(
            action,
            LinkAction::SwitchMarkdown {
                path: PathBuf::from("C:\\Users\\me\\notes.md"),
            }
        );
        let action_fwd = classify("D:/forward/slash.md", Some(&dir("/ignored")));
        assert_eq!(
            action_fwd,
            LinkAction::SwitchMarkdown {
                path: PathBuf::from("D:/forward/slash.md"),
            }
        );
    }

    #[test]
    fn relative_non_md_path_opens_with_os_app() {
        let action = classify("images/diagram.png", Some(&dir("/Users/me/docs")));
        assert_eq!(
            action,
            LinkAction::OpenWithOsApp {
                path: PathBuf::from("/Users/me/docs/images/diagram.png"),
            }
        );
    }

    #[test]
    fn absolute_non_md_path_opens_with_os_app() {
        let action = classify("/tmp/report.pdf", None);
        assert_eq!(
            action,
            LinkAction::OpenWithOsApp {
                path: PathBuf::from("/tmp/report.pdf"),
            }
        );
    }

    #[test]
    fn md_extension_match_is_case_insensitive() {
        let action = classify("CHANGELOG.MD", Some(&dir("/x")));
        assert_eq!(
            action,
            LinkAction::SwitchMarkdown {
                path: PathBuf::from("/x/CHANGELOG.MD"),
            }
        );
        let action_long = classify("guide.Markdown", Some(&dir("/x")));
        assert_eq!(
            action_long,
            LinkAction::SwitchMarkdown {
                path: PathBuf::from("/x/guide.Markdown"),
            }
        );
    }

    #[test]
    fn relative_path_without_base_passes_through() {
        // No current preview ⇒ no base dir ⇒ raw relative path goes to
        // OpenWithOsApp. The OS opener will probably fail; that's the
        // user's signal that the link can't be resolved.
        let action = classify("foo.png", None);
        assert_eq!(
            action,
            LinkAction::OpenWithOsApp {
                path: PathBuf::from("foo.png"),
            }
        );
    }
}
