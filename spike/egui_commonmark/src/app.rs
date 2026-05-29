use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

#[derive(Default)]
pub struct Spike {
    cache: CommonMarkCache,
}

const MARKDOWN: &str = include_str!("sample.md");

impl eframe::App for Spike {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                CommonMarkViewer::new().show(ui, &mut self.cache, MARKDOWN);
            });
        });
    }
}
