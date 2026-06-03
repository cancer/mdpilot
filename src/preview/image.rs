// Reachable from PreviewState::loaded / set_document via
// render::render_override_for. Unit tests cover the pure rewrite path;
// the empty-base / no-image branches still look dead to grep-style
// callers, so silence dead-code for the module surface.
#![allow(dead_code)]

//! Rewriting markdown image URIs so egui_commonmark's `file://` default
//! scheme resolves them against the preview file's directory.
//!
//! egui_commonmark prepends `default_implicit_uri_scheme` (`file://`) to
//! any image URL without a scheme — but the resulting `file://relative.png`
//! is then resolved by egui_extras::FileLoader as a *cwd-relative* path,
//! not relative to the .md file. We work around that by walking the
//! markdown with pulldown-cmark, finding image events, and rewriting
//! every local URL into a `file://<absolute>` form before handing the
//! text to egui_commonmark.
//!
//! Per `docs/preview.md` §6:
//! - HTTP/HTTPS: MVP non-supported — pass through unchanged so
//!   egui_commonmark's loader chain fails and shows a warning icon.
//! - `data:` URLs: pass through (`embedded_image` feature handles them).
//! - `file://` URLs: pass through (already resolved).
//! - Local relative: resolve against `base_dir`, prepend `file://`.
//! - Local absolute (Unix `/...` or Windows `C:\...` / `C:/...`):
//!   prepend `file://` so the host OS path becomes a valid URI.
//!
//! Known MVP gap: pulldown-cmark unescapes `\(` → `(` and HTML entities
//! (`&amp;` → `&`) in `dest_url`, so the substring search back into the
//! source text misses for those URLs. The rewrite is then a no-op and
//! the image fails to load — a safe degradation (same outcome as a
//! missing file).

use std::ops::Range;
use std::path::{Path, PathBuf};

use pulldown_cmark::{Event, Options, Parser, Tag};

/// Rewrite every inline image URI so egui_commonmark's `file://` default
/// scheme resolves it against `base_dir`. Returns the rewritten markdown
/// — identical to the input when no rewrite was needed (no images, all
/// external URLs, etc.). `base_dir` is `None` when no document is loaded
/// yet, in which case relative URLs are left alone.
pub fn rewrite_image_uris(markdown: &str, base_dir: Option<&Path>) -> String {
    // pulldown-cmark sometimes scans even when there are no images —
    // cheap pre-check keeps the no-image case allocation-free in the
    // common case (most chat-style markdown has no images at all).
    if !markdown.contains("![") {
        return markdown.to_string();
    }

    let parser = Parser::new_ext(markdown, parser_options()).into_offset_iter();

    let mut substitutions: Vec<(Range<usize>, String)> = Vec::new();
    for (event, range) in parser {
        let Event::Start(Tag::Image { dest_url, .. }) = event else {
            continue;
        };
        let original = dest_url.as_ref();
        let Some(new_uri) = resolve_image_uri(original, base_dir) else {
            continue;
        };

        // pulldown-cmark gives us the dest URL post-unescape (`\(` →
        // `(`, `&amp;` → `&`). If the same byte sequence isn't found in
        // the source span, leave the URL alone rather than blindly
        // splicing — see module docs for the trade-off.
        let span = &markdown[range.clone()];
        let Some(rel) = span.rfind(original) else {
            continue;
        };
        let abs = (range.start + rel)..(range.start + rel + original.len());
        substitutions.push((abs, new_uri));
    }

    if substitutions.is_empty() {
        return markdown.to_string();
    }

    // Sort by start offset; spans for distinct images do not overlap, so
    // a forward pass with a single cursor is sufficient.
    substitutions.sort_by_key(|(r, _)| r.start);

    let mut out = String::with_capacity(markdown.len());
    let mut cursor = 0;
    for (range, replacement) in substitutions {
        // Defensive: skip any substitution that would overlap a
        // previous one. pulldown-cmark shouldn't emit overlapping image
        // spans, but a malformed input could theoretically cause it.
        if range.start < cursor {
            continue;
        }
        out.push_str(&markdown[cursor..range.start]);
        out.push_str(&replacement);
        cursor = range.end;
    }
    out.push_str(&markdown[cursor..]);
    out
}

/// Decide what to rewrite `original` into, or `None` to leave it alone.
/// Pure: no filesystem touch.
pub(crate) fn resolve_image_uri(original: &str, base_dir: Option<&Path>) -> Option<String> {
    let trimmed = original.trim();
    if trimmed.is_empty() {
        return None;
    }
    if has_external_or_handled_scheme(trimmed) {
        return None;
    }

    let path = Path::new(trimmed);
    let absolute: PathBuf = if path.is_absolute() || is_windows_absolute(trimmed) {
        PathBuf::from(trimmed)
    } else if let Some(base) = base_dir {
        base.join(path)
    } else {
        // No base — leave the original alone so a later load with a
        // proper base can still find it (and avoid emitting a broken
        // `file://relative.png` that the loader would mis-resolve).
        return None;
    };

    Some(to_file_uri(&absolute))
}

/// True when `uri` already carries a scheme we don't need to rewrite.
/// Mirrors `link::is_external_scheme` but adds `file://` (already
/// resolved) and `data:` (handled by the embedded_image loader).
fn has_external_or_handled_scheme(uri: &str) -> bool {
    const SCHEMES: &[&str] = &[
        "http://", "https://", "data:", "file://", "ftp://", "mailto:",
    ];
    SCHEMES.iter().any(|s| uri.starts_with(s))
}

/// Match Windows drive paths like `C:\foo` / `C:/foo` even on Unix
/// hosts so unit tests stay platform-independent. Same shape as
/// `link::is_windows_absolute`; the two will likely move to a shared
/// `path_util` module when Phase 6.1 lands.
fn is_windows_absolute(uri: &str) -> bool {
    let bytes = uri.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// Build a `file://` URI from an absolute filesystem path. On Unix the
/// path already starts with `/`, so `file://` + `/abs/x` collapses to a
/// 3-slash URI. On Windows we add the leading slash explicitly and
/// normalize backslashes per `file:///C:/...` convention (see
/// `egui_extras::FileLoader::trim_extra_slash`).
pub(crate) fn to_file_uri(absolute: &Path) -> String {
    let raw = absolute.to_string_lossy();
    if cfg!(windows) {
        let mut s = String::with_capacity(raw.len() + 8);
        s.push_str("file:///");
        for ch in raw.chars() {
            if ch == '\\' {
                s.push('/');
            } else {
                s.push(ch);
            }
        }
        s
    } else {
        let mut s = String::with_capacity(raw.len() + 7);
        s.push_str("file://");
        s.push_str(&raw);
        s
    }
}

/// Mirror egui_commonmark_backend::pulldown::parser_options so we never
/// miss an image hidden inside a feature the renderer enables (tables,
/// task lists, etc.). Math is disabled because egui_commonmark only
/// enables it when a math callback is registered (`parser_options_math`),
/// and we don't register one.
fn parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_DEFINITION_LIST
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(p: &str) -> PathBuf {
        PathBuf::from(p)
    }

    #[test]
    fn rewrites_relative_image_into_file_uri() {
        let md = "![alt](foo.png)\n";
        let out = rewrite_image_uris(md, Some(&base("/docs")));
        assert_eq!(out, "![alt](file:///docs/foo.png)\n");
    }

    #[test]
    fn rewrites_relative_with_subdir() {
        let md = "see ![diagram](images/a.svg) inline";
        let out = rewrite_image_uris(md, Some(&base("/proj/docs")));
        assert_eq!(out, "see ![diagram](file:///proj/docs/images/a.svg) inline");
    }

    #[test]
    fn rewrites_unix_absolute_path() {
        let md = "![](/var/img/x.png)";
        let out = rewrite_image_uris(md, Some(&base("/somewhere/else")));
        assert_eq!(out, "![](file:///var/img/x.png)");
    }

    #[test]
    fn windows_drive_path_is_treated_as_absolute() {
        // Even on Unix hosts the unit test should see the Windows-drive
        // string treated as absolute (no base-dir join).
        let md = "![](C:\\images\\hero.png)";
        let out = rewrite_image_uris(md, Some(&base("/ignored")));
        // The exact URI form is host-dependent (Unix CI sees the path
        // through to_string_lossy unchanged, which still produces a
        // valid `file://C:...` for FileLoader on Windows). Either
        // backslashes or forward slashes are acceptable as long as the
        // drive segment is intact and prefixed with `file://`.
        assert!(out.starts_with("![](file://"), "got {out:?}");
        assert!(out.contains("C:"), "got {out:?}");
        assert!(out.contains("hero.png"), "got {out:?}");
    }

    #[test]
    fn http_image_is_left_alone() {
        let md = "![](https://example.com/banner.png)";
        let out = rewrite_image_uris(md, Some(&base("/docs")));
        assert_eq!(out, md);
    }

    #[test]
    fn data_url_is_left_alone() {
        let md = "![pixel](data:image/png;base64,iVBORw0KGgo=)";
        let out = rewrite_image_uris(md, Some(&base("/docs")));
        assert_eq!(out, md);
    }

    #[test]
    fn already_file_uri_is_left_alone() {
        let md = "![](file:///already/abs.png)";
        let out = rewrite_image_uris(md, Some(&base("/docs")));
        assert_eq!(out, md);
    }

    #[test]
    fn empty_url_is_left_alone() {
        // `![]()` is degenerate but valid CommonMark; we don't try to
        // be clever about it.
        let md = "![alt]()";
        let out = rewrite_image_uris(md, Some(&base("/docs")));
        assert_eq!(out, md);
    }

    #[test]
    fn no_base_dir_leaves_relative_alone() {
        let md = "![](foo.png)";
        let out = rewrite_image_uris(md, None);
        assert_eq!(out, md);
    }

    #[test]
    fn no_base_dir_still_rewrites_absolute() {
        let md = "![](/abs/x.png)";
        let out = rewrite_image_uris(md, None);
        assert_eq!(out, "![](file:///abs/x.png)");
    }

    #[test]
    fn no_image_in_text_is_a_passthrough() {
        let md = "# Title\n\nJust [a link](foo.png) — not an image.\n";
        let out = rewrite_image_uris(md, Some(&base("/docs")));
        assert_eq!(out, md, "regular links must not be rewritten");
    }

    #[test]
    fn image_in_table_cell_is_rewritten() {
        // Tables only get parsed when ENABLE_TABLES is on; this test
        // guards against accidentally dropping that option from
        // parser_options().
        let md = "| col |\n| --- |\n| ![](icon.png) |\n";
        let out = rewrite_image_uris(md, Some(&base("/proj")));
        assert!(
            out.contains("file:///proj/icon.png"),
            "table-cell image must be rewritten: {out}"
        );
    }

    #[test]
    fn multiple_images_keep_their_offsets_aligned() {
        let md = "![a](one.png) and ![b](two.jpg) and ![c](three.gif)";
        let out = rewrite_image_uris(md, Some(&base("/docs")));
        assert_eq!(
            out,
            "![a](file:///docs/one.png) and ![b](file:///docs/two.jpg) and ![c](file:///docs/three.gif)"
        );
    }

    #[test]
    fn title_after_url_is_preserved() {
        let md = "![alt](foo.png \"hover title\")";
        let out = rewrite_image_uris(md, Some(&base("/docs")));
        assert_eq!(out, "![alt](file:///docs/foo.png \"hover title\")");
    }

    #[test]
    fn escaped_dest_url_falls_through_safely() {
        // pulldown-cmark unescapes `\(` → `(` in dest_url, so rfind in
        // the source span misses and we leave the URL alone. The image
        // won't render — that's the safe outcome the module docs call
        // out as a known MVP gap.
        let md = r"![alt](foo\(1\).png)";
        let out = rewrite_image_uris(md, Some(&base("/docs")));
        assert_eq!(out, md, "escaped URL must pass through unchanged: {out}");
    }

    #[test]
    fn html_entity_in_dest_url_falls_through_safely() {
        let md = "![alt](foo&amp;bar.png)";
        let out = rewrite_image_uris(md, Some(&base("/docs")));
        assert_eq!(
            out, md,
            "entity-bearing URL must pass through unchanged: {out}"
        );
    }

    #[test]
    fn resolve_returns_none_for_external() {
        assert!(resolve_image_uri("https://example.com/x.png", Some(&base("/d"))).is_none());
        assert!(resolve_image_uri("data:image/png;base64,X", None).is_none());
        assert!(resolve_image_uri("", Some(&base("/d"))).is_none());
        assert!(resolve_image_uri("   ", Some(&base("/d"))).is_none());
    }

    #[test]
    fn resolve_handles_parent_segments() {
        // `..` is *not* canonicalized here — that requires filesystem
        // access. We hand the un-normalized path to egui_extras and let
        // std::fs::read fail or succeed based on actual perms, matching
        // link::classify's behavior.
        let out = resolve_image_uri("../sibling/img.png", Some(&base("/a/b"))).unwrap();
        assert_eq!(out, "file:///a/b/../sibling/img.png");
    }
}
