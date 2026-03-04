//! egui application — the Flash Player GUI.
//!
//! Provides a simple UI with:
//! - Menu bar: File > Open, Open URL, Close, Exit
//! - Central panel: renders the Flash content frame
//! - Status bar: shows current player state

use eframe::egui;
use parking_lot::Mutex;
use player_core::FlashPlayer;
use player_ui_traits::{FrameData, PlayerState};
use std::sync::Arc;

use crate::dialogs;

/// The egui application state.
pub struct FlashPlayerApp {
    /// The player core (owns the PPAPI host and plugin).
    player: FlashPlayer,
    /// Shared handle to the latest frame for rendering.
    frame_handle: Arc<Mutex<Option<FrameData>>>,
    /// Shared handle to the player state.
    state_handle: Arc<Mutex<PlayerState>>,
    /// The egui texture handle for the current frame.
    frame_texture: Option<egui::TextureHandle>,
    /// Last rendered frame dimensions (to detect changes).
    last_frame_size: (u32, u32),
    /// Open URL dialog state.
    url_dialog_open: bool,
    url_input: String,
    /// Plugin .so path (configurable).
    _plugin_path: String,
    /// Status message.
    status_message: String,
    /// Deferred SWF path to open on first frame.
    pending_open: Option<String>,
    /// Shared dialog state for alert/confirm/prompt.
    dialog_state: Arc<dialogs::DialogState>,
    /// Currently active dialog (moved from shared state for rendering).
    active_dialog: Option<dialogs::ActiveDialog>,
}

impl FlashPlayerApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, initial_swf: Option<String>) -> Self {
        let mut player = FlashPlayer::new();
        let frame_handle = player.latest_frame();
        let state_handle = player.state();

        // Default plugin path — can be overridden.
        let plugin_path =
            std::env::var("FLASH_PLUGIN_PATH").unwrap_or_else(|_| String::from("libpepflashplayer.so"));

        player.set_plugin_path(&plugin_path);

        // Set up the dialog provider.
        let dialog_state = Arc::new(dialogs::DialogState::new());
        let dialog_provider = Arc::new(dialogs::EguiDialogProvider::new(
            dialog_state.clone(),
            _cc.egui_ctx.clone(),
        ));
        player.set_dialog_provider(dialog_provider);

        // Set up the file chooser provider (using rfd).
        let file_chooser_provider = Arc::new(dialogs::RfdFileChooserProvider::new());
        player.set_file_chooser_provider(file_chooser_provider);

        Self {
            player,
            frame_handle,
            state_handle,
            frame_texture: None,
            last_frame_size: (0, 0),
            url_dialog_open: false,
            url_input: String::new(),
            _plugin_path: plugin_path,
            status_message: "Ready. Use File > Open to load a .swf file.".into(),
            pending_open: initial_swf,
            dialog_state,
            active_dialog: None,
        }
    }

    /// Draw the menu bar.
    fn draw_menu_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("📂 File", |ui| {
                if ui.button("Open...").clicked() {
                    ui.close_menu();
                    self.handle_open_file();
                }

                if ui.button("Open URL...").clicked() {
                    ui.close_menu();
                    self.url_dialog_open = true;
                    self.url_input.clear();
                }

                ui.separator();

                if ui.button("Close").clicked() {
                    ui.close_menu();
                    self.handle_close();
                }

                ui.separator();

                if ui.button("Exit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }

    /// Handle the "Open" file dialog.
    fn handle_open_file(&mut self) {
        // Use rfd for native file dialogs.
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("SWF Files", &["swf"])
            .add_filter("All Files", &["*"])
            .pick_file()
        {
            let path_str = path.to_string_lossy().to_string();
            self.open_content(&path_str);
        }
    }

    /// Open a .swf file or URL.
    fn open_content(&mut self, path: &str) {
        // Initialize host if not already done.
        if !self.player.is_plugin_loaded() {
            match self.player.init_host() {
                Ok(()) => {
                    self.status_message = "Plugin loaded.".into();
                }
                Err(e) => {
                    self.status_message = format!("Error: {}", e);
                    return;
                }
            }
        }

        // Close any existing content.
        if self.player.is_running() {
            self.player.close();
        }

        // Open the SWF.
        match self.player.open_swf(path) {
            Ok(()) => {
                self.status_message = format!("Playing: {}", path);
                // Notify of initial view size.
                self.player.notify_view_change(800, 600);
            }
            Err(e) => {
                self.status_message = format!("Error opening {}: {}", path, e);
            }
        }
    }

    /// Handle the "Close" action.
    fn handle_close(&mut self) {
        self.player.close();
        self.frame_texture = None;
        self.last_frame_size = (0, 0);
        self.status_message = "Content closed.".into();
    }

    /// Check for a pending dialog from the PPAPI thread and draw it.
    fn draw_pending_dialog(&mut self, ctx: &egui::Context) {
        // If we don't already have an active dialog, check for a new one.
        if self.active_dialog.is_none() {
            self.active_dialog = dialogs::take_pending_dialog(&self.dialog_state);
        }

        // If there's an active dialog, draw it.
        if let Some(ref mut dialog) = self.active_dialog {
            if let Some(response) = dialogs::draw_dialog(dialog, ctx) {
                // The user responded — send the response and remove the dialog.
                if let Some(dialog) = self.active_dialog.take() {
                    dialog.respond(response);
                }
            }
            // While a dialog is open, keep repainting.
            ctx.request_repaint();
        }
    }

    /// Draw the URL dialog.
    fn draw_url_dialog(&mut self, ctx: &egui::Context) {
        if !self.url_dialog_open {
            return;
        }

        let mut open = self.url_dialog_open;
        egui::Window::new("Open URL")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Enter the URL of a .swf file:");
                ui.text_edit_singleline(&mut self.url_input);
                ui.horizontal(|ui| {
                    if ui.button("Open").clicked() {
                        let url = self.url_input.clone();
                        self.url_dialog_open = false;
                        if !url.is_empty() {
                            self.open_content(&url);
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.url_dialog_open = false;
                    }
                });
            });
        self.url_dialog_open = open;
    }

    /// Update the frame texture from the latest frame data.
    fn update_frame_texture(&mut self, ctx: &egui::Context) {
        let frame_opt = self.frame_handle.lock().clone();
        if let Some(frame) = frame_opt {
            let size = (frame.width, frame.height);
            if size != self.last_frame_size || self.frame_texture.is_none() {
                self.last_frame_size = size;
            }

            // Convert BGRA_PREMUL to RGBA for egui.
            let mut rgba = vec![0u8; frame.pixels.len()];
            for i in (0..frame.pixels.len()).step_by(4) {
                if i + 3 < frame.pixels.len() {
                    rgba[i] = frame.pixels[i + 2]; // R = B
                    rgba[i + 1] = frame.pixels[i + 1]; // G = G
                    rgba[i + 2] = frame.pixels[i]; // B = R
                    rgba[i + 3] = frame.pixels[i + 3]; // A = A
                }
            }

            let image = egui::ColorImage::from_rgba_unmultiplied(
                [frame.width as usize, frame.height as usize],
                &rgba,
            );

            self.frame_texture = Some(ctx.load_texture(
                "flash_frame",
                image,
                egui::TextureOptions::NEAREST,
            ));
        }
    }

    /// Draw the central content area.
    fn draw_content(&self, ui: &mut egui::Ui) {
        if let Some(ref texture) = self.frame_texture {
            let available = ui.available_size();
            let tex_size = texture.size_vec2();

            // Scale to fit while maintaining aspect ratio.
            let scale = (available.x / tex_size.x).min(available.y / tex_size.y).min(1.0);
            let display_size = tex_size * scale;

            ui.centered_and_justified(|ui| {
                ui.image(egui::load::SizedTexture::new(texture.id(), display_size));
            });
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("No content loaded.\nUse File > Open to load a .swf file.");
            });
        }
    }
}

impl eframe::App for FlashPlayerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle deferred SWF open (from command line).
        if let Some(path) = self.pending_open.take() {
            self.open_content(&path);
        }

        // Poll the PPAPI main-thread message loop so that CallOnMainThread
        // callbacks are dispatched (timers, deferred rendering work, etc.).
        self.player.poll_main_loop();

        // Update frame texture from the latest plugin output.
        self.update_frame_texture(ctx);

        // Check for and draw any pending dialog (alert/confirm/prompt).
        self.draw_pending_dialog(ctx);

        // Draw URL dialog if open.
        self.draw_url_dialog(ctx);

        // Top panel: menu bar.
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            self.draw_menu_bar(ui, ctx);
        });

        // Bottom panel: status bar.
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status_message);

                // Show player state on the right.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let state = self.state_handle.lock().clone();
                    match state {
                        PlayerState::Idle => {
                            ui.label("⏹ Idle");
                        }
                        PlayerState::Loading { ref source } => {
                            ui.label(format!("⏳ Loading: {}", source));
                        }
                        PlayerState::Running { width, height } => {
                            ui.label(format!("▶ {}×{}", width, height));
                        }
                        PlayerState::Error { ref message } => {
                            ui.colored_label(egui::Color32::RED, format!("❌ {}", message));
                        }
                    }
                });
            });
        });

        // Central panel: Flash content.
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_content(ui);
        });

        // Request continuous repainting when running (for animation).
        if self.player.is_running() {
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.player.shutdown();
    }
}
