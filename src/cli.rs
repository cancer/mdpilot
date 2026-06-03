//! Minimal command-line argument parsing.
//!
//! Phase 6.1 will introduce positional arguments (`mdpilot <dir>` /
//! `<file>`) and may swap this for a `clap` setup. Until then, the
//! parser only recognizes the `--enable-dev-tools` flag — everything
//! else is silently ignored so future arguments (added incrementally)
//! don't break this entry point.

/// Parsed CLI options. Cheap to clone / copy and small enough to pass
/// by value into `App::new`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CliOptions {
    /// `--enable-dev-tools`: gate the developer-only debug surface
    /// (currently the `MDPILOT_DEBUG_SCREENSHOT` viewport capture).
    /// Distribution / release runs must not pass this; CI / local
    /// dev runs do.
    pub enable_dev_tools: bool,
}

/// Parse the process's command-line arguments. Reads from
/// `std::env::args()` — the wrapper exists so tests can use
/// [`parse_args`] with a synthetic argv.
pub fn parse() -> CliOptions {
    parse_args(std::env::args().skip(1))
}

/// Pure parser variant: consumes an iterator of argv tokens
/// (post-program-name) and returns the resulting options. Unknown
/// arguments are silently ignored for forward compatibility with
/// Phase 6.1.
pub fn parse_args<I>(args: I) -> CliOptions
where
    I: IntoIterator<Item = String>,
{
    let mut opts = CliOptions::default();
    for arg in args {
        if arg == "--enable-dev-tools" {
            opts.enable_dev_tools = true;
        }
        // Phase 6.1: validate / route positional file/dir args here.
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
    fn defaults_disable_dev_tools() {
        let opts = parse_args(argv(&[]));
        assert!(!opts.enable_dev_tools);
    }

    #[test]
    fn flag_enables_dev_tools() {
        let opts = parse_args(argv(&["--enable-dev-tools"]));
        assert!(opts.enable_dev_tools);
    }

    #[test]
    fn unknown_args_are_ignored() {
        // Phase 6.1 will tighten this. For now, ignoring unknowns
        // keeps the entry point forward-compatible with the
        // positional file/dir arguments coming next.
        let opts = parse_args(argv(&["foo.md", "--bogus", "/tmp"]));
        assert!(!opts.enable_dev_tools);
    }

    #[test]
    fn flag_works_with_surrounding_unknown_args() {
        let opts = parse_args(argv(&["foo.md", "--enable-dev-tools", "/tmp"]));
        assert!(opts.enable_dev_tools);
    }
}
