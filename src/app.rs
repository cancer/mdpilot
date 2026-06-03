use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui;
use uuid::Uuid;

use crate::chat::history::{ChatHistory, SystemMessage};
use crate::chat::session::{ChatSession, SpawnOptions};
use crate::chat::stream::ChatEvent;
use crate::preview::loader;
use crate::preview::render::PreviewState;

pub struct App {
    chat: ChatHistory,
    preview: PreviewState,
    session: Option<ChatSession>,
    events_rx: Option<Receiver<ChatEvent>>,
    disconnect_announced: bool,
    #[cfg(debug_assertions)]
    debug_screenshot: Option<DebugScreenshot>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::ui::fonts::install_japanese(&cc.egui_ctx);

        let mut chat = ChatHistory::default();
        let (session, events_rx) = match spawn_session(&cc.egui_ctx) {
            Ok((session, rx)) => (Some(session), Some(rx)),
            Err(err) => {
                tracing::warn!(error = %err, "failed to spawn claude session");
                chat.push_system(SystemMessage::SpawnFailed {
                    error: err.to_string(),
                });
                (None, None)
            }
        };

        let preview = preview_state_from_env();

        Self {
            chat,
            preview,
            session,
            events_rx,
            disconnect_announced: false,
            #[cfg(debug_assertions)]
            debug_screenshot: DebugScreenshot::from_env(),
        }
    }

    /// Write a user prompt to claude's stdin and record it in the local
    /// history. The matching assistant placeholder is only created on
    /// successful write so a `BrokenPipe` doesn't leave an empty assistant
    /// row sitting above the Disconnected banner.
    fn handle_send(&mut self, text: String) {
        self.chat.push_user(text.clone());
        let Some(session) = self.session.as_mut() else {
            // session is None only after a SpawnFailed at startup — that
            // banner is already in history, so don't double up on a second
            // Disconnected line. The Send button should already be disabled
            // in this state; reaching here means the input got past the
            // disabled gate, which is worth logging.
            tracing::warn!("send dispatched without an active claude session");
            return;
        };
        match session.send_user_message(&text) {
            Ok(()) => self.chat.start_assistant(None),
            Err(err) => {
                tracing::warn!(error = %err, "failed to write to claude stdin");
                if !self.disconnect_announced {
                    self.chat.push_system(SystemMessage::Disconnected);
                    self.disconnect_announced = true;
                }
            }
        }
    }

    /// Pull every pending event off the channel and fold it into the
    /// history. Called in `logic()` so the side effects are visible by the
    /// time `ui()` renders the same frame.
    fn drain_chat_events(&mut self) {
        let Some(rx) = self.events_rx.as_ref() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(event) => self.chat.apply(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !self.disconnect_announced {
                        self.chat.push_system(SystemMessage::Disconnected);
                        self.disconnect_announced = true;
                    }
                    self.events_rx = None;
                    break;
                }
            }
        }
    }
}

/// Pre-loads a preview file from `MDPILOT_PREVIEW_FILE` if set. This is
/// a Phase 4.2 dev hook — the real entry path is `mdpilot <file>` in
/// Phase 6.1 / `Cmd+O` in Phase 7.1. The env-var path lets us drive the
/// renderer without those UI surfaces yet.
fn preview_state_from_env() -> PreviewState {
    let Ok(raw) = std::env::var("MDPILOT_PREVIEW_FILE") else {
        return PreviewState::default();
    };
    let path = PathBuf::from(&raw);
    match loader::load_markdown(&path) {
        Ok(document) => PreviewState::loaded(document),
        Err(error) => {
            tracing::warn!(path = %raw, ?error, "failed to load MDPILOT_PREVIEW_FILE");
            let mut state = PreviewState::default();
            state.set_error(raw, error);
            state
        }
    }
}

/// Spawn the claude child process with cwd = current working directory and a
/// fresh session id. Returns the session and the receiver half of the event
/// channel. Phase 6 will replace `current_dir()` with the resolved project
/// root and start reading session ids from `SessionStore`.
fn spawn_session(ctx: &egui::Context) -> std::io::Result<(ChatSession, Receiver<ChatEvent>)> {
    let project_root = std::env::current_dir()?;
    let (tx, rx) = mpsc::channel::<ChatEvent>();
    let wake_ctx = ctx.clone();
    let session = ChatSession::start(
        SpawnOptions {
            project_root,
            session_id: Uuid::new_v4(),
            continue_session: false,
            model: None,
        },
        tx,
        move || wake_ctx.request_repaint(),
    )?;
    Ok((session, rx))
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_chat_events();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The view records send/cancel intent into these locals; the actual
        // history mutation and stdin write happen after the UI pass so the
        // borrow on `self.chat` inside layout::show stays the only one in
        // play at a time.
        let session_alive = self.session.is_some();
        let mut send_text: Option<String> = None;
        {
            let mut on_send = |text: String| {
                send_text = Some(text);
            };
            crate::ui::layout::show(
                ui,
                &mut self.chat,
                &mut self.preview,
                session_alive,
                &mut on_send,
            );
        }

        if let Some(text) = send_text {
            self.handle_send(text);
        }

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
    closed: bool,
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
                closed: false,
            })
    }

    fn step(&mut self, ctx: &egui::Context) {
        if self.closed {
            return;
        }
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
                // Walk through eframe's shutdown so future child processes
                // (claude in Phase 2+) get a chance to Drop cleanly. Bypassing
                // this with std::process::exit would orphan them.
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                self.closed = true;
                return;
            }
        }

        ctx.request_repaint();
    }
}
