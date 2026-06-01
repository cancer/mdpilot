use eframe::egui::{self, UserData, ViewportCommand};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

/// `(frame index, scroll_y, output_path)` triples driving the
/// non-interactive screenshot pass.
const SHOTS: &[(u32, f32, &str)] = &[
    (15, 0.0, "/tmp/spike_md_top.png"),
    (40, 700.0, "/tmp/spike_md_mid.png"),
    (65, 1400.0, "/tmp/spike_md_bot.png"),
];

pub struct Spike {
    cache: CommonMarkCache,
    frame_count: u32,
    next_shot: usize,
    awaiting: bool,
    scroll_y: f32,
}

impl Default for Spike {
    fn default() -> Self {
        Self {
            cache: CommonMarkCache::default(),
            frame_count: 0,
            next_shot: 0,
            awaiting: false,
            scroll_y: 0.0,
        }
    }
}

const MARKDOWN: &str = include_str!("sample.md");

impl eframe::App for Spike {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical()
                .vertical_scroll_offset(self.scroll_y)
                .show(ui, |ui| {
                    CommonMarkViewer::new().show(ui, &mut self.cache, MARKDOWN);
                });
        });

        self.frame_count += 1;

        if !self.awaiting && self.next_shot < SHOTS.len() {
            let (target_frame, target_y, _) = SHOTS[self.next_shot];
            if self.frame_count >= target_frame {
                self.scroll_y = target_y;
                self.awaiting = true;
                ui.ctx()
                    .send_viewport_cmd(ViewportCommand::Screenshot(UserData::default()));
            }
        }

        if self.awaiting {
            let mut grabbed: Option<std::sync::Arc<egui::ColorImage>> = None;
            ui.ctx().input(|i| {
                for event in &i.raw.events {
                    if let egui::Event::Screenshot { image, .. } = event {
                        grabbed = Some(image.clone());
                    }
                }
            });
            if let Some(image) = grabbed {
                let (_, _, path) = SHOTS[self.next_shot];
                let w = image.width() as u32;
                let h = image.height() as u32;
                let mut bytes = Vec::with_capacity(image.pixels.len() * 4);
                for c in image.pixels.iter() {
                    bytes.extend_from_slice(&c.to_array());
                }
                let buf =
                    image::RgbaImage::from_raw(w, h, bytes).expect("color image size mismatch");
                buf.save(path).expect("png save failed");
                eprintln!("saved screenshot to {path}");
                self.next_shot += 1;
                self.awaiting = false;
                if self.next_shot >= SHOTS.len() {
                    std::process::exit(0);
                }
            }
        }

        ui.ctx().request_repaint();
    }
}
