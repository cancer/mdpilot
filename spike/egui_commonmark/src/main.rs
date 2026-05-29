mod app;

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("spike_egui_commonmark")
            .with_inner_size([900.0, 700.0])
            .with_min_inner_size([500.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "spike_egui_commonmark",
        opts,
        Box::new(|_cc| Ok(Box::<app::Spike>::default())),
    )
}
