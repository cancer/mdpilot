use eframe::egui;
use egui_term::{BackendSettings, PtyEvent, TerminalBackend, TerminalView};
use std::sync::mpsc::Receiver;

pub struct SpikeApp {
    backend: TerminalBackend,
    pty_rx: Receiver<(u64, PtyEvent)>,
}

impl SpikeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        #[cfg(unix)]
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        #[cfg(windows)]
        let shell = "cmd.exe".to_string();

        let (pty_tx, pty_rx) = std::sync::mpsc::channel();
        let backend = TerminalBackend::new(
            0,
            cc.egui_ctx.clone(),
            pty_tx,
            BackendSettings {
                shell,
                ..Default::default()
            },
        )
        .expect("failed to start TerminalBackend");

        Self { backend, pty_rx }
    }
}

impl eframe::App for SpikeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Ok((_, PtyEvent::Exit)) = self.pty_rx.try_recv() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let terminal = TerminalView::new(ui, &mut self.backend)
            .set_focus(true)
            .set_size(ui.available_size());
        ui.add(terminal);
    }
}
