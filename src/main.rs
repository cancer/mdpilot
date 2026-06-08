mod app;
mod chat;
mod cli;
mod config;
mod error;
mod preview;
mod project;
mod tab;
mod ui;

fn main() -> eframe::Result {
    init_tracing();
    install_panic_hook();
    log_app_paths();
    let mut cli_opts = cli::parse();
    // Phase 9.X.6: when launched without a positional path, reopen
    // the project the user was last working on. Treats the stored
    // path as if it had been passed on the command line so the
    // `is_unbound` logic in App::new still works (an explicit
    // launch is "bound", reopen-by-history is also "bound").
    if cli_opts.positional.is_none() {
        if let Some(last) = read_last_project() {
            if last.is_dir() {
                tracing::info!(
                    path = %last.display(),
                    "reopening last-used project (Phase 9.X.6)",
                );
                cli_opts.positional = Some(last);
            } else {
                tracing::warn!(
                    path = %last.display(),
                    "stored last_project no longer exists; falling back to unbound launch",
                );
            }
        }
    }
    let project_init = match project::resolve(cli_opts.positional.as_deref()) {
        Ok(p) => p,
        Err(err) => {
            // Hard error before the GUI is up: log + print to stderr
            // + exit. Using `process::exit(2)` over an
            // `eframe::Error` because the failure is a CLI input
            // problem, not an eframe runtime problem.
            tracing::error!(error = %err, "failed to resolve project root");
            eprintln!("error: {err}");
            std::process::exit(2);
        }
    };

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("mdpilot")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "mdpilot",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, cli_opts, project_init)))),
    )
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(target: "mdpilot::panic", "{info}");
        default(info);
    }));
}

fn log_app_paths() {
    match config::paths::AppPaths::resolve() {
        Some(paths) => tracing::info!(
            config_dir = %paths.config_dir.display(),
            data_dir = %paths.data_dir.display(),
            cache_dir = %paths.cache_dir.display(),
            "resolved application paths",
        ),
        None => tracing::warn!("could not resolve application paths (no home directory?)"),
    }
}

/// Phase 9.X.6: peek at `sessions.json` to recover the last-used
/// project root. Pure read; the writer side lives in `App` after
/// the GUI starts. Returns `None` when no store exists, no
/// home dir is resolvable, or the stored field is missing.
fn read_last_project() -> Option<std::path::PathBuf> {
    let paths = config::paths::AppPaths::resolve()?;
    let store_path = paths.data_dir.join("sessions.json");
    let store = chat::session_store::SessionStore::load_or_default(&store_path);
    store.get_last_project().map(|p| p.to_path_buf())
}
