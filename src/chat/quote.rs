//! Pure helpers for the "send selection to chat" flow (Phase 9.X).
//!
//! Given a selected text snippet and the source document, build the
//! quoted text we append to the chat input. Format:
//!
//! ```text
//! <file:README.md L12-L15>
//! > selected text line 1
//! > selected text line 2
//! ```
//!
//! The `<file:…>` tag carries the file name plus a 1-based line
//! range when the selection appears as a unique substring of the
//! source; otherwise the line range is dropped and only the file
//! name remains.
//!
//! Egui's `LabelSelectionState` strips markdown syntax in
//! selections (e.g. selecting `Hello` from `# Hello` gives `Hello`
//! not `# Hello`), so we substring-match against the *raw* source
//! markdown rather than expecting an exact match including
//! formatting characters.

/// Build the full quote block appended to the chat input. Pure;
/// `current_filename` may be `None` when the preview is `Empty`
/// or `Failed` and no usable filename is known (in which case the
/// `<file:…>` tag is omitted entirely).
pub fn format_quote_block(
    selection: &str,
    source: Option<&str>,
    current_filename: Option<&str>,
) -> String {
    if selection.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    if let Some(file) = current_filename {
        out.push_str(&source_reference(selection, source, file));
        out.push('\n');
    }
    for line in selection.lines() {
        out.push_str("> ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Format the `<file:…>` reference tag. Public to the crate so the
/// quote-format unit tests can exercise it independently.
pub(crate) fn source_reference(selection: &str, source: Option<&str>, filename: &str) -> String {
    if let Some(source) = source {
        if let Some((start_line, end_line)) = unique_line_range(selection, source) {
            return format!("<file:{filename} L{start_line}-L{end_line}>");
        }
    }
    format!("<file:{filename}>")
}

/// Returns the (1-based) line range of `selection` within `source`
/// when the selection appears exactly once. Returns `None` if the
/// selection doesn't appear at all, or if it appears more than
/// once (in which case the line range would be ambiguous).
pub(crate) fn unique_line_range(selection: &str, source: &str) -> Option<(u32, u32)> {
    if selection.is_empty() {
        return None;
    }
    let first = source.find(selection)?;
    // Bug fix (2026-06-11): the previous `first + 1` offset was not
    // guaranteed to land on a UTF-8 char boundary, so the
    // `source[second_candidate_start..]` slice panicked on multibyte
    // text (CJK in particular). Searching after the end of the first
    // match avoids the boundary problem and is fine even when the
    // selection contains repeating characters, because vim hands us
    // contiguous regions rather than overlapping ones.
    let after_first = first + selection.len();
    if after_first <= source.len() && source[after_first..].contains(selection) {
        return None;
    }
    let start_line = 1 + source[..first].bytes().filter(|b| *b == b'\n').count() as u32;
    let newlines_in_selection = selection.bytes().filter(|b| *b == b'\n').count() as u32;
    Some((start_line, start_line + newlines_in_selection))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_line_range_single_line_match() {
        let source = "alpha\nbravo\ncharlie\n";
        assert_eq!(unique_line_range("bravo", source), Some((2, 2)));
    }

    #[test]
    fn unique_line_range_multi_line_selection() {
        let source = "alpha\nbravo\ncharlie\ndelta\n";
        assert_eq!(unique_line_range("bravo\ncharlie", source), Some((2, 3)));
    }

    #[test]
    fn unique_line_range_first_line_is_line_one() {
        // 1-based: a hit at byte 0 is line 1, not line 0.
        let source = "alpha\nbravo\n";
        assert_eq!(unique_line_range("alpha", source), Some((1, 1)));
    }

    #[test]
    fn unique_line_range_returns_none_when_absent() {
        assert_eq!(unique_line_range("zulu", "alpha\nbravo\n"), None);
    }

    #[test]
    fn unique_line_range_returns_none_when_ambiguous() {
        // The substring appears twice — we can't tell which one the
        // user selected, so we punt rather than guessing.
        let source = "TODO: x\nlater\nTODO: y\n";
        assert_eq!(unique_line_range("TODO", source), None);
    }

    #[test]
    fn unique_line_range_matches_inside_markdown_syntax() {
        // Selecting "Hello" from a rendered `# Hello` heading should
        // still pinpoint the heading line — substring search ignores
        // the leading `# `.
        let source = "intro\n# Hello\nbody\n";
        assert_eq!(unique_line_range("Hello", source), Some((2, 2)));
    }

    #[test]
    fn unique_line_range_matches_inline_formatting() {
        // Selecting "important" from `**important**` source.
        let source = "para with **important** word\n";
        assert_eq!(unique_line_range("important", source), Some((1, 1)));
    }

    #[test]
    fn unique_line_range_handles_cjk_without_panic() {
        // Regression for 2026-06-11 crash: `first + 1` after `find`
        // landed inside a multibyte sequence, panicking the slice.
        let source = "前文\nあい\n後文\n";
        assert_eq!(unique_line_range("あい", source), Some((2, 2)));
    }

    #[test]
    fn unique_line_range_returns_none_on_repeated_cjk() {
        // Same multibyte selection appearing twice → ambiguous, no
        // line range. Must not panic on the second-search slice.
        let source = "ほげ\nふが\nほげ\n";
        assert_eq!(unique_line_range("ほげ", source), None);
    }

    #[test]
    fn source_reference_includes_line_range_when_unique() {
        let source = "alpha\nbravo\ncharlie\n";
        assert_eq!(
            source_reference("bravo", Some(source), "guide.md"),
            "<file:guide.md L2-L2>",
        );
    }

    #[test]
    fn source_reference_drops_line_range_when_ambiguous() {
        // Multi-occurrence falls back to filename only — the user
        // gets a usable reference without an incorrect line number.
        let source = "TODO\nbody\nTODO\n";
        assert_eq!(
            source_reference("TODO", Some(source), "README.md"),
            "<file:README.md>",
        );
    }

    #[test]
    fn source_reference_drops_line_range_when_no_source_available() {
        // Empty / Failed preview state has no source text.
        assert_eq!(source_reference("hi", None, "draft.md"), "<file:draft.md>",);
    }

    #[test]
    fn format_quote_block_emits_file_tag_and_blockquote() {
        let source = "alpha\nbravo\ncharlie\n";
        let result = format_quote_block("bravo", Some(source), Some("guide.md"));
        assert_eq!(result, "<file:guide.md L2-L2>\n> bravo\n");
    }

    #[test]
    fn format_quote_block_multiline_selection() {
        let source = "alpha\nbravo\ncharlie\n";
        let result = format_quote_block("bravo\ncharlie", Some(source), Some("guide.md"));
        assert_eq!(result, "<file:guide.md L2-L3>\n> bravo\n> charlie\n");
    }

    #[test]
    fn format_quote_block_without_filename_skips_tag() {
        // No filename → no tag, just blockquote lines.
        let result = format_quote_block("hello", Some("hello"), None);
        assert_eq!(result, "> hello\n");
    }

    #[test]
    fn format_quote_block_empty_selection_yields_empty_string() {
        assert_eq!(format_quote_block("", Some("anything"), Some("a.md")), "");
    }
}
