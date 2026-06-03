//! Minimal command-line argument parsing.
//!
//! The accepted surface today (Phase 6.1) is:
//!
//! - `--enable-dev-tools` — opt-in to the developer-only debug
//!   surface (currently only the `MDPILOT_DEBUG_SCREENSHOT` capture
//!   in `src/app.rs`).
//! - Exactly one optional positional `<path>` — either a directory
//!   (project root) or a `.md` file (project root = parent dir,
//!   and the file becomes the initial preview target in Phase 6.4).
//!
//! Unknown flags are ignored rather than rejected; a clap migration
//! would tighten this once we have richer arg shapes (Phase 9).

use std::path::PathBuf;

/// Parsed CLI options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliOptions {
    /// `--enable-dev-tools`: gate the developer-only debug surface
    /// (currently the `MDPILOT_DEBUG_SCREENSHOT` viewport capture).
    /// Distribution / release runs must not pass this; CI / local
    /// dev runs do.
    pub enable_dev_tools: bool,
    /// First positional argument (a path) if present. `src/project.rs`
    /// classifies it into project root + optional initial-preview
    /// file at startup. `None` falls back to the current working
    /// directory (Phase 7.1 will replace that fallback with a
    /// project-selection dialog).
    pub positional: Option<PathBuf>,
}

/// Parse the process's command-line arguments. Reads from
/// `std::env::args()` — the wrapper exists so tests can use
/// [`parse_args`] with a synthetic argv.
pub fn parse() -> CliOptions {
    parse_args(std::env::args().skip(1))
}

/// Pure parser variant: consumes an iterator of argv tokens
/// (post-program-name) and returns the resulting options.
///
/// Recognition rules:
/// - `--enable-dev-tools` flips that flag (independent of position).
/// - The first non-`--`-prefixed token is captured as `positional`;
///   subsequent positional tokens are silently dropped (callers
///   passing multiple paths get the first one, intentional MVP
///   behavior until a richer parser lands).
/// - Other `--…` flags are silently ignored.
pub fn parse_args<I>(args: I) -> CliOptions
where
    I: IntoIterator<Item = String>,
{
    let mut opts = CliOptions::default();
    for arg in args {
        if arg == "--enable-dev-tools" {
            opts.enable_dev_tools = true;
        } else if !arg.starts_with("--") && opts.positional.is_none() {
            opts.positional = Some(PathBuf::from(arg));
        }
        // Other `--…` flags or extra positional args: ignored.
    }
    opts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn defaults_disable_dev_tools_and_have_no_positional() {
        let opts = parse_args(argv(&[]));
        assert!(!opts.enable_dev_tools);
        assert_eq!(opts.positional, None);
    }

    #[test]
    fn flag_enables_dev_tools() {
        let opts = parse_args(argv(&["--enable-dev-tools"]));
        assert!(opts.enable_dev_tools);
        assert_eq!(opts.positional, None);
    }

    #[test]
    fn first_positional_path_is_captured() {
        let opts = parse_args(argv(&["docs/README.md"]));
        assert_eq!(opts.positional, Some(PathBuf::from("docs/README.md")));
    }

    #[test]
    fn flag_and_positional_compose_in_any_order() {
        let opts = parse_args(argv(&["--enable-dev-tools", "/proj"]));
        assert!(opts.enable_dev_tools);
        assert_eq!(opts.positional, Some(PathBuf::from("/proj")));

        let opts2 = parse_args(argv(&["/proj", "--enable-dev-tools"]));
        assert!(opts2.enable_dev_tools);
        assert_eq!(opts2.positional, Some(PathBuf::from("/proj")));
    }

    #[test]
    fn second_positional_is_silently_dropped() {
        // MVP: keep the first positional, ignore the rest. A future
        // clap-based parser would reject this.
        let opts = parse_args(argv(&["first.md", "second.md"]));
        assert_eq!(opts.positional, Some(PathBuf::from("first.md")));
    }

    #[test]
    fn unknown_flags_do_not_become_positional() {
        // `--bogus` looks flag-y, so it must not occupy the
        // positional slot even though we don't recognize it.
        let opts = parse_args(argv(&["--bogus", "real.md"]));
        assert_eq!(opts.positional, Some(PathBuf::from("real.md")));
    }
}
