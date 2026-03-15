//! egui application — the Flash Player GUI.
//!
//! Provides a simple UI with:
//! - Menu bar: File > Open, Open URL, Close, Exit
//! - Central panel: renders the Flash content frame
//! - Status bar: shows current player state

use eframe::egui;
use parking_lot::Mutex;
use player_core::{FlashPlayer, SharedFrameBuffer};
use player_ui_traits::PlayerState;
use ppapi_sys::*;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::dialogs;

/// The egui application state.
pub struct FlashPlayerApp {
    /// The player core (owns the PPAPI host and plugin).
    player: FlashPlayer,
    /// Shared frame buffer for incremental texture updates.
    frame_handle: Arc<Mutex<Option<SharedFrameBuffer>>>,
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
    /// Shared context menu state for Flash right-click menus.
    context_menu_state: Arc<dialogs::ContextMenuState>,
    /// Currently active Flash context menu being rendered.
    active_context_menu: Option<dialogs::ActiveContextMenu>,
    /// Last mouse position sent (to avoid duplicate MOUSEMOVE events).
    last_mouse_pos: Option<PP_Point>,
    /// Current cursor type requested by the plugin.
    cursor_type: Arc<AtomicI32>,
    /// Last content area size sent to the plugin (to detect resize).
    last_content_size: (i32, i32),
    /// Whether the window currently has focus.
    has_focus: bool,
    /// Egui context (for fullscreen provider).
    egui_ctx: egui::Context,
}

impl FlashPlayerApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, initial_swf: Option<String>) -> Self {
        let mut player = FlashPlayer::new();
        let frame_handle = player.latest_frame();
        let state_handle = player.state();
        let cursor_type = player.cursor_type();

        // Tell the player core to wake egui whenever a new frame arrives.
        let repaint_ctx = _cc.egui_ctx.clone();
        player.set_repaint_callback(move || repaint_ctx.request_repaint());

        // Default plugin path — can be overridden.
        #[cfg(windows)]
        let plugin_path =
            std::env::var("FLASH_PLUGIN_PATH").unwrap_or_else(|_| String::from("pepflashplayer.dll"));
        #[cfg(not(windows))]
        let plugin_path =
            std::env::var("FLASH_PLUGIN_PATH").unwrap_or_else(|_| String::from("libpepflashplayer.so"));

        let actual_plugin_path = std::fs::canonicalize(&plugin_path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| plugin_path.clone());

        player.set_plugin_path(&actual_plugin_path);

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

        let context_menu_state = Arc::new(dialogs::ContextMenuState::new());

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
            context_menu_state,
            active_context_menu: None,
            last_mouse_pos: None,
            cursor_type,
            last_content_size: (0, 0),
            has_focus: true,
            egui_ctx: _cc.egui_ctx.clone(),
        }
    }

    /// Draw the menu bar.
    fn draw_menu_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("📂 File", |ui| {
                if ui.button("Open...").clicked() {
                    ui.close();
                    self.handle_open_file();
                }

                if ui.button("Open URL...").clicked() {
                    ui.close();
                    self.url_dialog_open = true;
                    self.url_input.clear();
                }

                ui.separator();

                if ui.button("Close").clicked() {
                    ui.close();
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

                    // Set up the cpal-based audio input provider for
                    // microphone capture (PPB_AudioInput).
                    let host = ppapi_host::HOST.get().expect("HOST not initialised");
                    host.set_audio_input_provider(Box::new(
                        ppapi_host::audio_input_cpal::CpalAudioInputProvider::new(),
                    ));

                    // Set up the arboard-based clipboard provider for
                    // system clipboard access (PPB_Flash_Clipboard).
                    host.set_clipboard_provider(Box::new(
                        ppapi_host::clipboard_arboard::ArboardClipboardProvider::new(),
                    ));

                    // Set up the egui fullscreen provider.
                    host.set_fullscreen_provider(Box::new(
                        dialogs::EguiFullscreenProvider::new(self.egui_ctx.clone()),
                    ));

                    // Set up the egui context menu provider for Flash right-click menus.
                    host.set_context_menu_provider(Box::new(
                        dialogs::EguiContextMenuProvider::new(
                            self.context_menu_state.clone(),
                            self.egui_ctx.clone(),
                        ),
                    ));

                    // Set up the print provider for Flash printing (PPB_PDF::Print).
                    host.set_print_provider(Box::new(
                        dialogs::EguiPrintProvider::new(self.frame_handle.clone()),
                    ));
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
                let (width, height) = self.last_content_size;
                self.player.notify_view_change(width, height);
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

    /// Check for and draw a pending Flash context menu.
    fn draw_pending_context_menu(&mut self, ctx: &egui::Context) {
        // If we don't already have an active context menu, check for a new one.
        if self.active_context_menu.is_none() {
            self.active_context_menu =
                dialogs::take_pending_context_menu(&self.context_menu_state);
        }

        // If there's an active context menu, draw it.
        if self.active_context_menu.is_some() {
            let result = dialogs::draw_context_menu(
                self.active_context_menu.as_ref().unwrap(),
                ctx,
            );
            if let Some(selected) = result {
                // The user selected an item or dismissed the menu.
                if let Some(menu) = self.active_context_menu.take() {
                    menu.respond(selected);
                }
            }
            // While the context menu is open, keep repainting.
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

    /// Update the frame texture from the shared frame buffer.
    ///
    /// On the first frame (or after a size change) the full texture is
    /// allocated.  Subsequent updates use partial GPU uploads covering
    /// only the dirty region reported by the PPAPI host.
    fn update_frame_texture(&mut self, ctx: &egui::Context) {
        // Scope the lock: extract dirty info and convert pixels, then release.
        let update = {
            let mut guard = self.frame_handle.lock();
            let Some(ref mut buf) = *guard else { return };
            let Some((dirty_x, dirty_y, dirty_w, dirty_h)) = buf.pending_dirty.take() else {
                return;
            };

            let frame_w = buf.width;
            let frame_h = buf.height;
            let need_full =
                self.last_frame_size != (frame_w, frame_h) || self.frame_texture.is_none();

            let converted = if need_full {
                // Convert the entire buffer BGRA_PREMUL → Color32.
                buf.pixels
                    .chunks_exact(4)
                    .map(|bgra| {
                        egui::Color32::from_rgba_premultiplied(
                            bgra[2], bgra[1], bgra[0], bgra[3],
                        )
                    })
                    .collect::<Vec<_>>()
            } else {
                // Convert only the dirty sub-region.
                let stride = buf.stride as usize;
                let mut sub = Vec::with_capacity((dirty_w * dirty_h) as usize);
                for row in 0..dirty_h {
                    let y = (dirty_y + row) as usize;
                    let off = y * stride + dirty_x as usize * 4;
                    let end = off + dirty_w as usize * 4;
                    for bgra in buf.pixels[off..end].chunks_exact(4) {
                        sub.push(egui::Color32::from_rgba_premultiplied(
                            bgra[2], bgra[1], bgra[0], bgra[3],
                        ));
                    }
                }
                sub
            };

            (need_full, converted, frame_w, frame_h, dirty_x, dirty_y, dirty_w, dirty_h)
        }; // guard (and shared-frame lock) released here

        let (need_full, pixels, frame_w, frame_h, dirty_x, dirty_y, dirty_w, dirty_h) = update;

        if need_full {
            // Size changed or first frame — (re)create the full texture.
            self.last_frame_size = (frame_w, frame_h);
            let image = egui::ColorImage {
                size: [frame_w as usize, frame_h as usize],
                pixels,
                source_size: egui::Vec2::new(frame_w as f32, frame_h as f32),
            };
            self.frame_texture = Some(ctx.load_texture(
                "flash_frame",
                image,
                egui::TextureOptions::NEAREST,
            ));
        } else {
            // Partial update — upload only the dirty sub-region.
            let sub_image = egui::ColorImage {
                size: [dirty_w as usize, dirty_h as usize],
                pixels,
                source_size: egui::Vec2::new(dirty_w as f32, dirty_h as f32),
            };
            let tex_id = self.frame_texture.as_ref().unwrap().id();
            ctx.tex_manager().write().set(
                tex_id,
                egui::epaint::ImageDelta::partial(
                    [dirty_x as usize, dirty_y as usize],
                    sub_image,
                    egui::TextureOptions::NEAREST,
                ),
            );
        }
    }

    /// Draw the central content area and handle input events.
    fn draw_content(&mut self, ui: &mut egui::Ui) {
        // Always track the available content area so we can notify the
        // plugin when the window is resized, even before a frame exists.
        let available = ui.available_size();
        let new_w = available.x as i32;
        let new_h = available.y as i32;
        if (new_w, new_h) != self.last_content_size && new_w > 0 && new_h > 0 {
            self.last_content_size = (new_w, new_h);
            if self.player.is_running() {
                self.player.notify_view_change(new_w, new_h);
            }
        }

        if let Some(ref texture) = self.frame_texture {
            let tex_size = texture.size_vec2();

            // Fill the entire available area — the plugin is told
            // the real size and will render at that resolution.
            let display_size = available;
            let content_rect = egui::Rect::from_min_size(ui.min_rect().min, display_size);

            // Paint the frame.
            ui.painter().image(
                texture.id(),
                content_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );

            // Allocate an interactive area over the image so we get input.
            let response = ui.allocate_rect(content_rect, egui::Sense::click_and_drag());

            // Also capture keyboard focus when hovered / interacted.
            if response.hovered() || response.has_focus() {
                response.request_focus();
            }

            // --- Mouse events ---
            if self.player.is_running() {
                self.handle_mouse_events(ui, &response, content_rect, display_size, tex_size);
                self.handle_keyboard_events(ui);
                self.handle_scroll_events(ui, &response, content_rect, display_size, tex_size);
            }
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("No content loaded.\nUse File > Open to load a .swf file.");
            });
        }
    }

    /// Convert screen position to plugin-local coordinates.
    fn screen_to_plugin(
        pos: egui::Pos2,
        content_rect: egui::Rect,
        display_size: egui::Vec2,
        tex_size: egui::Vec2,
    ) -> PP_Point {
        let local_x = (pos.x - content_rect.min.x) / display_size.x * tex_size.x;
        let local_y = (pos.y - content_rect.min.y) / display_size.y * tex_size.y;
        PP_Point {
            x: local_x as i32,
            y: local_y as i32,
        }
    }

    /// Build PPAPI modifier flags from egui's modifier state.
    fn egui_modifiers_to_ppapi(modifiers: &egui::Modifiers) -> u32 {
        let mut flags = 0u32;
        if modifiers.shift {
            flags |= PP_INPUTEVENT_MODIFIER_SHIFTKEY;
        }
        if modifiers.ctrl {
            flags |= PP_INPUTEVENT_MODIFIER_CONTROLKEY;
        }
        if modifiers.alt {
            flags |= PP_INPUTEVENT_MODIFIER_ALTKEY;
        }
        if modifiers.mac_cmd || modifiers.command {
            flags |= PP_INPUTEVENT_MODIFIER_METAKEY;
        }
        flags
    }

    /// Map egui PointerButton to PPAPI MouseButton.
    fn egui_button_to_ppapi(button: egui::PointerButton) -> PP_InputEvent_MouseButton {
        match button {
            egui::PointerButton::Primary => PP_INPUTEVENT_MOUSEBUTTON_LEFT,
            egui::PointerButton::Secondary => PP_INPUTEVENT_MOUSEBUTTON_RIGHT,
            egui::PointerButton::Middle => PP_INPUTEVENT_MOUSEBUTTON_MIDDLE,
            _ => PP_INPUTEVENT_MOUSEBUTTON_NONE,
        }
    }

    /// Handle mouse events (press, release, move) over the content area.
    fn handle_mouse_events(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        content_rect: egui::Rect,
        display_size: egui::Vec2,
        tex_size: egui::Vec2,
    ) {
        let modifiers = ui.input(|i| i.modifiers);
        let mut pp_modifiers = Self::egui_modifiers_to_ppapi(&modifiers);

        // Check for pointer position.
        let pointer_pos = ui.input(|i| i.pointer.interact_pos()).or_else(|| {
            ui.input(|i| i.pointer.hover_pos())
        });

        // Mouse button presses.
        for &button in &[
            egui::PointerButton::Primary,
            egui::PointerButton::Secondary,
            egui::PointerButton::Middle,
        ] {
            if response.clicked_by(button) || ui.input(|i| i.pointer.button_pressed(button)) {
                if let Some(pos) = pointer_pos {
                    if content_rect.contains(pos) {
                        let pp_pos = Self::screen_to_plugin(pos, content_rect, display_size, tex_size);
                        let pp_button = Self::egui_button_to_ppapi(button);

                        // Add button-down modifier.
                        let btn_mod = match button {
                            egui::PointerButton::Primary => PP_INPUTEVENT_MODIFIER_LEFTBUTTONDOWN,
                            egui::PointerButton::Secondary => PP_INPUTEVENT_MODIFIER_RIGHTBUTTONDOWN,
                            egui::PointerButton::Middle => PP_INPUTEVENT_MODIFIER_MIDDLEBUTTONDOWN,
                            _ => 0,
                        };
                        pp_modifiers |= btn_mod;

                        self.player.send_mouse_event(
                            PP_INPUTEVENT_TYPE_MOUSEDOWN,
                            pp_button,
                            pp_pos,
                            1,
                            pp_modifiers,
                        );
                    }
                }
            }

            if ui.input(|i| i.pointer.button_released(button)) {
                if let Some(pos) = pointer_pos {
                    if content_rect.contains(pos) {
                        let pp_pos = Self::screen_to_plugin(pos, content_rect, display_size, tex_size);
                        let pp_button = Self::egui_button_to_ppapi(button);
                        self.player.send_mouse_event(
                            PP_INPUTEVENT_TYPE_MOUSEUP,
                            pp_button,
                            pp_pos,
                            0,
                            pp_modifiers,
                        );

                        // Synthesize a context menu event for right-click,
                        // matching browser behaviour (player-web gets this
                        // from the DOM "contextmenu" event).  Flash uses
                        // this to trigger PPB_Flash_Menu::Show.
                        if button == egui::PointerButton::Secondary {
                            self.player.send_mouse_event(
                                PP_INPUTEVENT_TYPE_CONTEXTMENU,
                                PP_INPUTEVENT_MOUSEBUTTON_RIGHT,
                                pp_pos,
                                0,
                                pp_modifiers,
                            );
                        }
                    }
                }
            }
        }

        // Mouse move.
        if response.hovered() {
            if let Some(pos) = pointer_pos {
                if content_rect.contains(pos) {
                    // Only send mouse move if position changed.
                    let pp_pos = Self::screen_to_plugin(pos, content_rect, display_size, tex_size);
                    if self.last_mouse_pos != Some(pp_pos) {
                        self.last_mouse_pos = Some(pp_pos);
                        self.player.send_mouse_event(
                            PP_INPUTEVENT_TYPE_MOUSEMOVE,
                            PP_INPUTEVENT_MOUSEBUTTON_NONE,
                            pp_pos,
                            0,
                            pp_modifiers,
                        );
                    }
                }
            }
        }
    }

    /// Handle keyboard events.
    fn handle_keyboard_events(&mut self, ui: &egui::Ui) {
        ui.input(|i| {
            for event in &i.events {
                let modifiers = Self::egui_modifiers_to_ppapi(&i.modifiers);
                match event {
                    egui::Event::Key {
                        key,
                        pressed,
                        modifiers: _,
                        repeat,
                        ..
                    } => {
                        let key_code = egui_key_to_vk(*key);
                        if key_code == 0 {
                            continue;
                        }
                        let code_str = egui_key_to_code_str(*key);
                        let mut mods = modifiers;
                        if *repeat {
                            mods |= PP_INPUTEVENT_MODIFIER_ISAUTOREPEAT;
                        }
                        if *pressed {
                            self.player.send_keyboard_event(
                                PP_INPUTEVENT_TYPE_RAWKEYDOWN,
                                key_code,
                                "",
                                code_str,
                                mods,
                            );
                        } else {
                            self.player.send_keyboard_event(
                                PP_INPUTEVENT_TYPE_KEYUP,
                                key_code,
                                "",
                                code_str,
                                mods,
                            );
                        }
                    }
                    egui::Event::Text(text) => {
                        for ch in text.chars() {
                            self.player.send_keyboard_event(
                                PP_INPUTEVENT_TYPE_CHAR,
                                ch as u32,
                                &ch.to_string(),
                                "",
                                modifiers,
                            );
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    /// Handle scroll/wheel events.
    fn handle_scroll_events(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        _content_rect: egui::Rect,
        _display_size: egui::Vec2,
        _tex_size: egui::Vec2,
    ) {
        if !response.hovered() {
            return;
        }
        let scroll_delta = ui.input(|i| i.raw_scroll_delta);
        if scroll_delta.x.abs() > 0.0 || scroll_delta.y.abs() > 0.0 {
            let modifiers = ui.input(|i| i.modifiers);
            let pp_modifiers = Self::egui_modifiers_to_ppapi(&modifiers);

            let delta = PP_FloatPoint {
                x: scroll_delta.x,
                y: scroll_delta.y,
            };
            // Convert to discrete ticks (a rough approximation).
            let ticks = PP_FloatPoint {
                x: if scroll_delta.x.abs() > 0.0 {
                    scroll_delta.x.signum()
                } else {
                    0.0
                },
                y: if scroll_delta.y.abs() > 0.0 {
                    scroll_delta.y.signum()
                } else {
                    0.0
                },
            };
            self.player.send_wheel_event(delta, ticks, false, pp_modifiers);
        }
    }
}

/// Map egui::Key to a Windows virtual key code (VK_*), which is what
/// Pepper/Flash expects for KeyCode values.
fn egui_key_to_vk(key: egui::Key) -> u32 {
    match key {
        egui::Key::A => 0x41,
        egui::Key::B => 0x42,
        egui::Key::C => 0x43,
        egui::Key::D => 0x44,
        egui::Key::E => 0x45,
        egui::Key::F => 0x46,
        egui::Key::G => 0x47,
        egui::Key::H => 0x48,
        egui::Key::I => 0x49,
        egui::Key::J => 0x4A,
        egui::Key::K => 0x4B,
        egui::Key::L => 0x4C,
        egui::Key::M => 0x4D,
        egui::Key::N => 0x4E,
        egui::Key::O => 0x4F,
        egui::Key::P => 0x50,
        egui::Key::Q => 0x51,
        egui::Key::R => 0x52,
        egui::Key::S => 0x53,
        egui::Key::T => 0x54,
        egui::Key::U => 0x55,
        egui::Key::V => 0x56,
        egui::Key::W => 0x57,
        egui::Key::X => 0x58,
        egui::Key::Y => 0x59,
        egui::Key::Z => 0x5A,
        egui::Key::Num0 => 0x30,
        egui::Key::Num1 => 0x31,
        egui::Key::Num2 => 0x32,
        egui::Key::Num3 => 0x33,
        egui::Key::Num4 => 0x34,
        egui::Key::Num5 => 0x35,
        egui::Key::Num6 => 0x36,
        egui::Key::Num7 => 0x37,
        egui::Key::Num8 => 0x38,
        egui::Key::Num9 => 0x39,
        egui::Key::Escape => 0x1B,
        egui::Key::Tab => 0x09,
        egui::Key::Backspace => 0x08,
        egui::Key::Enter => 0x0D,
        egui::Key::Space => 0x20,
        egui::Key::ArrowUp => 0x26,
        egui::Key::ArrowDown => 0x28,
        egui::Key::ArrowLeft => 0x25,
        egui::Key::ArrowRight => 0x27,
        egui::Key::Home => 0x24,
        egui::Key::End => 0x23,
        egui::Key::PageUp => 0x21,
        egui::Key::PageDown => 0x22,
        egui::Key::Delete => 0x2E,
        egui::Key::Insert => 0x2D,
        egui::Key::F1 => 0x70,
        egui::Key::F2 => 0x71,
        egui::Key::F3 => 0x72,
        egui::Key::F4 => 0x73,
        egui::Key::F5 => 0x74,
        egui::Key::F6 => 0x75,
        egui::Key::F7 => 0x76,
        egui::Key::F8 => 0x77,
        egui::Key::F9 => 0x78,
        egui::Key::F10 => 0x79,
        egui::Key::F11 => 0x7A,
        egui::Key::F12 => 0x7B,
        egui::Key::Minus => 0xBD,
        egui::Key::Plus => 0xBB,
        _ => 0,
    }
}

/// Map egui::Key to a DOM KeyboardEvent.code string.
fn egui_key_to_code_str(key: egui::Key) -> &'static str {
    match key {
        egui::Key::A => "KeyA",
        egui::Key::B => "KeyB",
        egui::Key::C => "KeyC",
        egui::Key::D => "KeyD",
        egui::Key::E => "KeyE",
        egui::Key::F => "KeyF",
        egui::Key::G => "KeyG",
        egui::Key::H => "KeyH",
        egui::Key::I => "KeyI",
        egui::Key::J => "KeyJ",
        egui::Key::K => "KeyK",
        egui::Key::L => "KeyL",
        egui::Key::M => "KeyM",
        egui::Key::N => "KeyN",
        egui::Key::O => "KeyO",
        egui::Key::P => "KeyP",
        egui::Key::Q => "KeyQ",
        egui::Key::R => "KeyR",
        egui::Key::S => "KeyS",
        egui::Key::T => "KeyT",
        egui::Key::U => "KeyU",
        egui::Key::V => "KeyV",
        egui::Key::W => "KeyW",
        egui::Key::X => "KeyX",
        egui::Key::Y => "KeyY",
        egui::Key::Z => "KeyZ",
        egui::Key::Num0 => "Digit0",
        egui::Key::Num1 => "Digit1",
        egui::Key::Num2 => "Digit2",
        egui::Key::Num3 => "Digit3",
        egui::Key::Num4 => "Digit4",
        egui::Key::Num5 => "Digit5",
        egui::Key::Num6 => "Digit6",
        egui::Key::Num7 => "Digit7",
        egui::Key::Num8 => "Digit8",
        egui::Key::Num9 => "Digit9",
        egui::Key::Escape => "Escape",
        egui::Key::Tab => "Tab",
        egui::Key::Backspace => "Backspace",
        egui::Key::Enter => "Enter",
        egui::Key::Space => "Space",
        egui::Key::ArrowUp => "ArrowUp",
        egui::Key::ArrowDown => "ArrowDown",
        egui::Key::ArrowLeft => "ArrowLeft",
        egui::Key::ArrowRight => "ArrowRight",
        egui::Key::Home => "Home",
        egui::Key::End => "End",
        egui::Key::PageUp => "PageUp",
        egui::Key::PageDown => "PageDown",
        egui::Key::Delete => "Delete",
        egui::Key::Insert => "Insert",
        egui::Key::F1 => "F1",
        egui::Key::F2 => "F2",
        egui::Key::F3 => "F3",
        egui::Key::F4 => "F4",
        egui::Key::F5 => "F5",
        egui::Key::F6 => "F6",
        egui::Key::F7 => "F7",
        egui::Key::F8 => "F8",
        egui::Key::F9 => "F9",
        egui::Key::F10 => "F10",
        egui::Key::F11 => "F11",
        egui::Key::F12 => "F12",
        egui::Key::Minus => "Minus",
        egui::Key::Plus => "Equal",
        _ => "",
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

        // Apply the cursor type requested by the plugin.
        if self.player.is_running() {
            let cursor = pp_cursor_to_egui(self.cursor_type.load(Ordering::Relaxed));
            ctx.set_cursor_icon(cursor);
        }

        // Detect focus changes and notify the plugin.
        let focused = ctx.input(|i| i.focused);
        if focused != self.has_focus {
            self.has_focus = focused;
            if self.player.is_running() {
                self.player.notify_focus_change(focused);
            }
        }

        // Check for and draw any pending dialog (alert/confirm/prompt).
        self.draw_pending_dialog(ctx);

        // Check for and draw any pending Flash context menu.
        self.draw_pending_context_menu(ctx);

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

        // Schedule the next repaint:
        // • When running, poll the PPAPI main-thread message loop every
        //   ~4 ms so that CallOnMainThread timers fire promptly.
        //   New frames from Flash trigger an *immediate* repaint via the
        //   repaint callback wired in `new()`, so we only need this for
        //   timer dispatch — not for frame presentation.
        // • When idle, egui sleeps until user input.
        if self.player.is_running() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.player.shutdown();
    }
}

// ---------------------------------------------------------------------------
// PP_CursorType_Dev → egui::CursorIcon mapping
// ---------------------------------------------------------------------------

fn pp_cursor_to_egui(cursor_type: i32) -> egui::CursorIcon {
    match cursor_type {
        PP_CURSORTYPE_POINTER => egui::CursorIcon::Default,
        PP_CURSORTYPE_CROSS => egui::CursorIcon::Crosshair,
        PP_CURSORTYPE_HAND => egui::CursorIcon::PointingHand,
        PP_CURSORTYPE_IBEAM => egui::CursorIcon::Text,
        PP_CURSORTYPE_WAIT => egui::CursorIcon::Wait,
        PP_CURSORTYPE_HELP => egui::CursorIcon::Help,
        PP_CURSORTYPE_EASTRESIZE => egui::CursorIcon::ResizeEast,
        PP_CURSORTYPE_NORTHRESIZE => egui::CursorIcon::ResizeNorth,
        PP_CURSORTYPE_NORTHEASTRESIZE => egui::CursorIcon::ResizeNorthEast,
        PP_CURSORTYPE_NORTHWESTRESIZE => egui::CursorIcon::ResizeNorthWest,
        PP_CURSORTYPE_SOUTHRESIZE => egui::CursorIcon::ResizeSouth,
        PP_CURSORTYPE_SOUTHEASTRESIZE => egui::CursorIcon::ResizeSouthEast,
        PP_CURSORTYPE_SOUTHWESTRESIZE => egui::CursorIcon::ResizeSouthWest,
        PP_CURSORTYPE_WESTRESIZE => egui::CursorIcon::ResizeWest,
        PP_CURSORTYPE_NORTHSOUTHRESIZE => egui::CursorIcon::ResizeVertical,
        PP_CURSORTYPE_EASTWESTRESIZE => egui::CursorIcon::ResizeHorizontal,
        PP_CURSORTYPE_NORTHEASTSOUTHWESTRESIZE => egui::CursorIcon::ResizeNorthEast,
        PP_CURSORTYPE_NORTHWESTSOUTHEASTRESIZE => egui::CursorIcon::ResizeNorthWest,
        PP_CURSORTYPE_COLUMNRESIZE => egui::CursorIcon::ResizeColumn,
        PP_CURSORTYPE_ROWRESIZE => egui::CursorIcon::ResizeRow,
        PP_CURSORTYPE_MOVE => egui::CursorIcon::Move,
        PP_CURSORTYPE_VERTICALTEXT => egui::CursorIcon::VerticalText,
        PP_CURSORTYPE_CELL => egui::CursorIcon::Cell,
        PP_CURSORTYPE_CONTEXTMENU => egui::CursorIcon::ContextMenu,
        PP_CURSORTYPE_ALIAS => egui::CursorIcon::Alias,
        PP_CURSORTYPE_PROGRESS => egui::CursorIcon::Progress,
        PP_CURSORTYPE_NODROP => egui::CursorIcon::NoDrop,
        PP_CURSORTYPE_COPY => egui::CursorIcon::Copy,
        PP_CURSORTYPE_NONE => egui::CursorIcon::None,
        PP_CURSORTYPE_NOTALLOWED => egui::CursorIcon::NotAllowed,
        PP_CURSORTYPE_ZOOMIN => egui::CursorIcon::ZoomIn,
        PP_CURSORTYPE_ZOOMOUT => egui::CursorIcon::ZoomOut,
        PP_CURSORTYPE_GRAB => egui::CursorIcon::Grab,
        PP_CURSORTYPE_GRABBING => egui::CursorIcon::Grabbing,
        // Panning cursors → AllScroll (closest match)
        PP_CURSORTYPE_MIDDLEPANNING
        | PP_CURSORTYPE_EASTPANNING
        | PP_CURSORTYPE_NORTHPANNING
        | PP_CURSORTYPE_NORTHEASTPANNING
        | PP_CURSORTYPE_NORTHWESTPANNING
        | PP_CURSORTYPE_SOUTHPANNING
        | PP_CURSORTYPE_SOUTHEASTPANNING
        | PP_CURSORTYPE_SOUTHWESTPANNING
        | PP_CURSORTYPE_WESTPANNING => egui::CursorIcon::AllScroll,
        // Custom / unknown → default arrow
        _ => egui::CursorIcon::Default,
    }
}
