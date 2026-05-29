mod app;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("spike_egui_term")
            .with_inner_size([900.0, 600.0])
            .with_min_inner_size([500.0, 300.0]),
        ..Default::default()
    };

    eframe::run_native(
        "spike_egui_term",
        native_options,
        Box::new(|cc| Ok(Box::new(app::SpikeApp::new(cc)))),
    )
}
