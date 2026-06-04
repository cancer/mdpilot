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
    let cli_opts = cli::parse();
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
