mod app;
mod error;

fn main() -> eframe::Result {
    init_tracing();
    install_panic_hook();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("mdpilot")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native("mdpilot", options, Box::new(|_cc| Ok(Box::new(app::App))))
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
