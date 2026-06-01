use eframe::egui;

pub struct App {
    #[cfg(debug_assertions)]
    debug_screenshot: Option<DebugScreenshot>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::ui::fonts::install_japanese(&cc.egui_ctx);
        Self::default()
    }
}

impl Default for App {
    fn default() -> Self {
        Self {
            #[cfg(debug_assertions)]
            debug_screenshot: DebugScreenshot::from_env(),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        crate::ui::layout::show(ui);

        #[cfg(debug_assertions)]
        if let Some(cap) = self.debug_screenshot.as_mut() {
            cap.step(ui.ctx());
        }
    }
}

/// One-shot screenshot helper compiled only in debug builds.
///
/// Activated by setting `MDPILOT_DEBUG_SCREENSHOT=/path/to/out.png`. Waits a
/// handful of frames so layout settles, then requests one viewport screenshot,
/// saves it as PNG, and exits the process. Release builds skip the entire
/// module so this leaves no production footprint.
#[cfg(debug_assertions)]
struct DebugScreenshot {
    path: String,
    frame_count: u32,
    requested: bool,
}

#[cfg(debug_assertions)]
impl DebugScreenshot {
    fn from_env() -> Option<Self> {
        std::env::var("MDPILOT_DEBUG_SCREENSHOT")
            .ok()
            .map(|path| Self {
                path,
                frame_count: 0,
                requested: false,
            })
    }

    fn step(&mut self, ctx: &egui::Context) {
        self.frame_count += 1;

        if !self.requested && self.frame_count >= 30 {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }

        if self.requested {
            let mut grabbed: Option<std::sync::Arc<egui::ColorImage>> = None;
            ctx.input(|i| {
                for event in &i.raw.events {
                    if let egui::Event::Screenshot { image, .. } = event {
                        grabbed = Some(image.clone());
                    }
                }
            });
            if let Some(image) = grabbed {
                let w = image.width() as u32;
                let h = image.height() as u32;
                let mut bytes = Vec::with_capacity(image.pixels.len() * 4);
                for c in image.pixels.iter() {
                    bytes.extend_from_slice(&c.to_array());
                }
                let buf = image::RgbaImage::from_raw(w, h, bytes)
                    .expect("debug screenshot: color image size mismatch");
                buf.save(&self.path)
                    .expect("debug screenshot: png save failed");
                eprintln!("debug screenshot saved to {}", &self.path);
                std::process::exit(0);
            }
        }

        ctx.request_repaint();
    }
}
