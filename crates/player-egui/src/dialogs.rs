//! Egui dialog provider — implements `player_ui_traits::DialogProvider` using
//! egui modal windows.
//!
//! Because egui is immediate-mode, dialogs are rendered in the main `update()`
//! loop. The provider sends requests via shared state and blocks the calling
//! thread (typically the PPAPI plugin thread) until the egui loop processes
//! the dialog and the user responds.

use std::sync::{mpsc, Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use player_ui_traits::DialogProvider;

// ===========================================================================
// Request / response types
// ===========================================================================

/// A pending dialog request, waiting for the egui event loop to render it.
pub struct PendingDialog {
    pub kind: DialogKind,
    pub response_tx: mpsc::Sender<DialogResponse>,
}

/// The kind of dialog to display.
#[derive(Clone)]
pub enum DialogKind {
    Alert(String),
    Confirm(String),
    Prompt {
        message: String,
        default: String,
    },
}

/// The user's response to a dialog.
pub enum DialogResponse {
    /// Alert was dismissed.
    Dismissed,
    /// Confirm result.
    Confirmed(bool),
    /// Prompt result (`None` = cancelled).
    PromptResult(Option<String>),
}

// ===========================================================================
// Shared dialog state (between provider thread and egui thread)
// ===========================================================================

/// Shared state that both the `EguiDialogProvider` and the egui app access.
pub struct DialogState {
    /// A pending dialog request, set by the provider and consumed by the app.
    pub pending: Mutex<Option<PendingDialog>>,
}

impl DialogState {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(None),
        }
    }
}

// ===========================================================================
// EguiDialogProvider
// ===========================================================================

/// Thread-safe dialog provider that sends requests to the egui event loop.
///
/// Constructed with a shared `DialogState` and an `egui::Context` so it can
/// call `request_repaint` to wake the UI thread.
pub struct EguiDialogProvider {
    state: Arc<DialogState>,
    ctx: egui::Context,
}

impl EguiDialogProvider {
    pub fn new(state: Arc<DialogState>, ctx: egui::Context) -> Self {
        Self { state, ctx }
    }

    /// Internal helper: send a request and block until response.
    fn send_and_wait(&self, kind: DialogKind) -> DialogResponse {
        let (tx, rx) = mpsc::channel();
        {
            let mut pending = self.state.pending.lock().unwrap();
            *pending = Some(PendingDialog {
                kind,
                response_tx: tx,
            });
        }
        // Wake the egui event loop so it renders the dialog.
        self.ctx.request_repaint();
        // Block the calling thread until the user responds.
        rx.recv().unwrap_or(DialogResponse::Dismissed)
    }
}

impl DialogProvider for EguiDialogProvider {
    fn alert(&self, message: &str) {
        tracing::trace!("EguiDialogProvider::alert({:?})", message);
        self.send_and_wait(DialogKind::Alert(message.to_string()));
    }

    fn confirm(&self, message: &str) -> bool {
        tracing::trace!("EguiDialogProvider::confirm({:?})", message);
        match self.send_and_wait(DialogKind::Confirm(message.to_string())) {
            DialogResponse::Confirmed(v) => v,
            _ => true,
        }
    }

    fn prompt(&self, message: &str, default: &str) -> Option<String> {
        tracing::trace!("EguiDialogProvider::prompt({:?}, {:?})", message, default);
        match self.send_and_wait(DialogKind::Prompt {
            message: message.to_string(),
            default: default.to_string(),
        }) {
            DialogResponse::PromptResult(v) => v,
            _ => Some(default.to_string()),
        }
    }
}

// ===========================================================================
// Egui-side dialog rendering
// ===========================================================================

/// Active dialog state held by the egui app for rendering.
pub struct ActiveDialog {
    pub kind: DialogKind,
    pub input: String,
    response_tx: mpsc::Sender<DialogResponse>,
}

impl ActiveDialog {
    /// Send the given response and consume this dialog.
    pub fn respond(self, response: DialogResponse) {
        let _ = self.response_tx.send(response);
    }
}

/// Check for a new pending dialog in `state` and, if there is one, return
/// an `ActiveDialog` for the egui app to render.
pub fn take_pending_dialog(state: &DialogState) -> Option<ActiveDialog> {
    let mut pending = state.pending.lock().unwrap();
    pending.take().map(|p| {
        let input = match &p.kind {
            DialogKind::Prompt { default, .. } => default.clone(),
            _ => String::new(),
        };
        ActiveDialog {
            kind: p.kind,
            input,
            response_tx: p.response_tx,
        }
    })
}

/// Draw the active dialog in the egui context. Returns `true` if the dialog
/// was closed (responded to) during this frame.
pub fn draw_dialog(dialog: &mut ActiveDialog, ctx: &egui::Context) -> Option<DialogResponse> {
    let mut response: Option<DialogResponse> = None;

    // Pre-extract the message (avoids borrow issues in closures).
    let message = match &dialog.kind {
        DialogKind::Alert(m) | DialogKind::Confirm(m) => m.clone(),
        DialogKind::Prompt { message, .. } => message.clone(),
    };

    let is_alert = matches!(dialog.kind, DialogKind::Alert(_));
    let is_confirm = matches!(dialog.kind, DialogKind::Confirm(_));
    let _is_prompt = matches!(dialog.kind, DialogKind::Prompt { .. });

    let title = if is_alert {
        "Alert"
    } else if is_confirm {
        "Confirm"
    } else {
        "Prompt"
    };

    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(&message);

            if !is_alert && !is_confirm {
                // Prompt: show text input
                ui.text_edit_singleline(&mut dialog.input);
            }

            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    if is_alert {
                        response = Some(DialogResponse::Dismissed);
                    } else if is_confirm {
                        response = Some(DialogResponse::Confirmed(true));
                    } else {
                        response = Some(DialogResponse::PromptResult(Some(dialog.input.clone())));
                    }
                }
                if !is_alert {
                    if ui.button("Cancel").clicked() {
                        if is_confirm {
                            response = Some(DialogResponse::Confirmed(false));
                        } else {
                            response = Some(DialogResponse::PromptResult(None));
                        }
                    }
                }
            });
        });

    response
}

// ===========================================================================
// File chooser provider — uses rfd for native file dialogs
// ===========================================================================

/// File chooser provider using the `rfd` crate for native file dialogs.
///
/// This is thread-safe and can be called from the PPAPI plugin thread.
pub struct RfdFileChooserProvider;

impl RfdFileChooserProvider {
    pub fn new() -> Self {
        Self
    }
}

impl player_ui_traits::FileChooserProvider for RfdFileChooserProvider {
    fn show_file_chooser(
        &self,
        mode: player_ui_traits::FileChooserMode,
        accept_types: &str,
        suggested_name: &str,
    ) -> Vec<String> {
        match mode {
            player_ui_traits::FileChooserMode::Open => {
                let mut dialog = rfd::FileDialog::new();

                // Parse accept_types into file filters
                if !accept_types.is_empty() {
                    let extensions = parse_accept_types(accept_types);
                    if !extensions.is_empty() {
                        let ext_refs: Vec<&str> = extensions.iter().map(|s| s.as_str()).collect();
                        dialog = dialog.add_filter("Accepted Files", &ext_refs);
                    }
                }
                dialog = dialog.add_filter("All Files", &["*"]);

                match dialog.pick_file() {
                    Some(path) => vec![path.to_string_lossy().to_string()],
                    None => Vec::new(),
                }
            }
            player_ui_traits::FileChooserMode::OpenMultiple => {
                let mut dialog = rfd::FileDialog::new();

                if !accept_types.is_empty() {
                    let extensions = parse_accept_types(accept_types);
                    if !extensions.is_empty() {
                        let ext_refs: Vec<&str> = extensions.iter().map(|s| s.as_str()).collect();
                        dialog = dialog.add_filter("Accepted Files", &ext_refs);
                    }
                }
                dialog = dialog.add_filter("All Files", &["*"]);

                match dialog.pick_files() {
                    Some(paths) => paths
                        .into_iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect(),
                    None => Vec::new(),
                }
            }
            player_ui_traits::FileChooserMode::Save => {
                let mut dialog = rfd::FileDialog::new();

                if !suggested_name.is_empty() {
                    dialog = dialog.set_file_name(suggested_name);
                }

                if !accept_types.is_empty() {
                    let extensions = parse_accept_types(accept_types);
                    if !extensions.is_empty() {
                        let ext_refs: Vec<&str> = extensions.iter().map(|s| s.as_str()).collect();
                        dialog = dialog.add_filter("Accepted Files", &ext_refs);
                    }
                }
                dialog = dialog.add_filter("All Files", &["*"]);

                match dialog.save_file() {
                    Some(path) => vec![path.to_string_lossy().to_string()],
                    None => Vec::new(),
                }
            }
        }
    }
}

/// Parse the accept_types string (comma-separated MIME types or extensions)
/// into a list of file extensions suitable for file dialog filters.
fn parse_accept_types(accept_types: &str) -> Vec<String> {
    let mut extensions = Vec::new();

    for part in accept_types.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if part.starts_with('.') {
            // Already an extension like ".swf"
            extensions.push(part.trim_start_matches('.').to_string());
        } else if part.contains('/') {
            // MIME type — map common ones to extensions
            match part {
                "image/*" => extensions.extend(["png", "jpg", "jpeg", "gif", "bmp", "webp"].iter().map(|s| s.to_string())),
                "image/png" => extensions.push("png".to_string()),
                "image/jpeg" => extensions.extend(["jpg", "jpeg"].iter().map(|s| s.to_string())),
                "image/gif" => extensions.push("gif".to_string()),
                "text/plain" => extensions.push("txt".to_string()),
                "text/html" => extensions.extend(["html", "htm"].iter().map(|s| s.to_string())),
                "application/x-shockwave-flash" => extensions.push("swf".to_string()),
                "application/pdf" => extensions.push("pdf".to_string()),
                "video/*" => extensions.extend(["mp4", "webm", "avi", "mkv", "flv"].iter().map(|s| s.to_string())),
                "audio/*" => extensions.extend(["mp3", "wav", "ogg", "flac", "aac"].iter().map(|s| s.to_string())),
                _ => {
                    // Unknown MIME type — take the subtype as extension
                    if let Some(subtype) = part.split('/').nth(1) {
                        if subtype != "*" {
                            extensions.push(subtype.to_string());
                        }
                    }
                }
            }
        } else {
            // Treat as bare extension
            extensions.push(part.to_string());
        }
    }

    extensions
}

// ===========================================================================
// Egui fullscreen provider — uses eframe ViewportCommand
// ===========================================================================

/// Fullscreen provider that toggles eframe's native fullscreen mode via
/// `egui::ViewportCommand::Fullscreen`.
///
/// The `egui::Context` is safe to use from any thread; calling
/// `send_viewport_cmd` is non-blocking.
pub struct EguiFullscreenProvider {
    ctx: egui::Context,
    is_fullscreen: AtomicBool,
}

impl EguiFullscreenProvider {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            ctx,
            is_fullscreen: AtomicBool::new(false),
        }
    }
}

impl player_ui_traits::FullscreenProvider for EguiFullscreenProvider {
    fn is_fullscreen(&self) -> bool {
        self.is_fullscreen.load(Ordering::Relaxed)
    }

    fn set_fullscreen(&self, fullscreen: bool) -> bool {
        self.is_fullscreen.store(fullscreen, Ordering::Relaxed);
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::Fullscreen(fullscreen));
        self.ctx.request_repaint();
        true
    }

    fn get_screen_size(&self) -> Option<(i32, i32)> {
        // Query the monitor size from the viewport.
        self.ctx.input(|i| {
            let r = i.viewport().monitor_size.unwrap_or(
                egui::Vec2::new(
                    i.viewport().inner_rect.map_or(800.0, |r| r.width()),
                    i.viewport().inner_rect.map_or(600.0, |r| r.height()),
                ),
            );
            Some((r.x as i32, r.y as i32))
        })
    }
}

// ===========================================================================
// Egui context menu provider — Flash right-click menus
// ===========================================================================

/// Shared state for pending context menu requests between the PPAPI thread
/// and the egui render loop.
pub struct ContextMenuState {
    /// A pending context menu request.
    pub pending: Mutex<Option<PendingContextMenu>>,
}

impl ContextMenuState {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(None),
        }
    }
}

/// A pending context menu request from Flash.
pub struct PendingContextMenu {
    pub items: Vec<player_ui_traits::ContextMenuItem>,
    pub x: i32,
    pub y: i32,
    pub response_tx: mpsc::Sender<Option<i32>>,
}

/// Active context menu being rendered by egui.
pub struct ActiveContextMenu {
    pub items: Vec<player_ui_traits::ContextMenuItem>,
    pub x: i32,
    pub y: i32,
    response_tx: mpsc::Sender<Option<i32>>,
}

impl ActiveContextMenu {
    /// Send the selection and consume this menu.
    pub fn respond(self, selected_id: Option<i32>) {
        let _ = self.response_tx.send(selected_id);
    }
}

/// Check for a new pending context menu request.
pub fn take_pending_context_menu(state: &ContextMenuState) -> Option<ActiveContextMenu> {
    let mut pending = state.pending.lock().unwrap();
    pending.take().map(|p| ActiveContextMenu {
        items: p.items,
        x: p.x,
        y: p.y,
        response_tx: p.response_tx,
    })
}

/// Context menu provider using egui popup windows.
///
/// Constructed with a shared `ContextMenuState` and an `egui::Context`
/// to wake the UI thread when a menu request arrives.
pub struct EguiContextMenuProvider {
    state: Arc<ContextMenuState>,
    ctx: egui::Context,
}

impl EguiContextMenuProvider {
    pub fn new(state: Arc<ContextMenuState>, ctx: egui::Context) -> Self {
        Self { state, ctx }
    }
}

impl player_ui_traits::ContextMenuProvider for EguiContextMenuProvider {
    fn show_context_menu(
        &self,
        items: &[player_ui_traits::ContextMenuItem],
        x: i32,
        y: i32,
    ) -> Option<i32> {
        let (tx, rx) = mpsc::channel();
        {
            let mut pending = self.state.pending.lock().unwrap();
            *pending = Some(PendingContextMenu {
                items: items.to_vec(),
                x,
                y,
                response_tx: tx,
            });
        }
        self.ctx.request_repaint();
        // Block the PPAPI thread until the user selects or dismisses.
        rx.recv().unwrap_or(None)
    }
}

/// Draw an active context menu in the egui context.
/// Returns `Some(selected_id)` or `Some(None)` if dismissed, `None` if still open.
pub fn draw_context_menu(
    menu: &ActiveContextMenu,
    ctx: &egui::Context,
) -> Option<Option<i32>> {
    let mut result: Option<Option<i32>> = None;

    // Use a fixed-position egui Area at the menu position.
    let area_id = egui::Id::new("flash_context_menu");

    // Detect clicks outside the menu area to dismiss.
    let pointer_pos = ctx.input(|i| i.pointer.interact_pos());
    let clicked_outside = ctx.input(|i| i.pointer.any_pressed());

    let area_resp = egui::Area::new(area_id)
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(menu.x as f32, menu.y as f32))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                result = draw_menu_items(ui, &menu.items);
            });
        });

    // Check if the user clicked outside the menu.
    if clicked_outside {
        if let Some(pos) = pointer_pos {
            if !area_resp.response.rect.contains(pos) {
                result = Some(None); // dismissed
            }
        }
    }

    // Check for Escape key.
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        result = Some(None);
    }

    result
}

/// Recursively draw menu items. Returns `Some(Some(id))` if an item was clicked,
/// `None` if the menu is still open.
fn draw_menu_items(
    ui: &mut egui::Ui,
    items: &[player_ui_traits::ContextMenuItem],
) -> Option<Option<i32>> {
    for item in items {
        match item.item_type {
            player_ui_traits::ContextMenuItemType::Separator => {
                ui.separator();
            }
            player_ui_traits::ContextMenuItemType::Normal
            | player_ui_traits::ContextMenuItemType::Checkbox => {
                let label = if item.item_type == player_ui_traits::ContextMenuItemType::Checkbox
                    && item.checked
                {
                    format!("✓ {}", item.name)
                } else {
                    item.name.clone()
                };

                let button = egui::Button::new(&label).wrap_mode(egui::TextWrapMode::Extend);
                let resp = ui.add_enabled(item.enabled, button);
                if resp.clicked() {
                    return Some(Some(item.id));
                }
            }
            player_ui_traits::ContextMenuItemType::Submenu => {
                ui.menu_button(&item.name, |ui| {
                    if let Some(result) = draw_menu_items(ui, &item.submenu) {
                        // Propagate the selection up.
                        // Store in a temp so the caller can detect it.
                        ui.memory_mut(|m| {
                            m.data
                                .insert_temp(egui::Id::new("flash_ctx_menu_result"), result);
                        });
                        ui.close();
                    }
                });
                // Check if a submenu item was selected.
                let sub_result: Option<Option<i32>> = ui.memory(|m| {
                    m.data
                        .get_temp(egui::Id::new("flash_ctx_menu_result"))
                });
                if let Some(r) = sub_result {
                    ui.memory_mut(|m| {
                        m.data.remove::<Option<i32>>(egui::Id::new("flash_ctx_menu_result"));
                    });
                    return Some(r);
                }
            }
        }
    }
    None
}

// ===========================================================================
// Print provider — captures the current frame and prints via OS facilities
// ===========================================================================

/// Provides printing for the desktop egui player.
///
/// Captures the current Flash frame from the shared buffer and writes it
/// to a temporary BMP file, then launches the platform's print command.
pub struct EguiPrintProvider {
    frame_handle: Arc<parking_lot::Mutex<Option<player_core::SharedFrameBuffer>>>,
}

impl EguiPrintProvider {
    pub fn new(
        frame_handle: Arc<parking_lot::Mutex<Option<player_core::SharedFrameBuffer>>>,
    ) -> Self {
        Self { frame_handle }
    }

    /// Capture the current frame as RGBA pixel data, returns (width, height, rgba_pixels).
    fn capture_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
        let guard = self.frame_handle.lock();
        let fb = guard.as_ref()?;
        if fb.width == 0 || fb.height == 0 || fb.pixels.is_empty() {
            return None;
        }
        // Convert BGRA_PREMUL to RGBA.
        let pixel_count = (fb.width * fb.height) as usize;
        let mut rgba = Vec::with_capacity(pixel_count * 4);
        for i in 0..pixel_count {
            let offset = i * 4;
            if offset + 3 < fb.pixels.len() {
                let b = fb.pixels[offset];
                let g = fb.pixels[offset + 1];
                let r = fb.pixels[offset + 2];
                let a = fb.pixels[offset + 3];
                // Un-premultiply alpha.
                if a == 0 {
                    rgba.extend_from_slice(&[0, 0, 0, 0]);
                } else if a == 255 {
                    rgba.extend_from_slice(&[r, g, b, 255]);
                } else {
                    let af = a as f32 / 255.0;
                    let ur = ((r as f32 / af).min(255.0)) as u8;
                    let ug = ((g as f32 / af).min(255.0)) as u8;
                    let ub = ((b as f32 / af).min(255.0)) as u8;
                    rgba.extend_from_slice(&[ur, ug, ub, a]);
                }
            }
        }
        Some((fb.width, fb.height, rgba))
    }

    /// Write a minimal BMP file from RGBA pixel data.
    fn write_bmp(width: u32, height: u32, rgba: &[u8], path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        let row_bytes = width * 3;
        let row_padding = (4 - (row_bytes % 4)) % 4;
        let padded_row = row_bytes + row_padding;
        let pixel_data_size = padded_row * height;
        let file_size = 54 + pixel_data_size;

        let mut f = std::fs::File::create(path)?;
        // BMP file header (14 bytes)
        f.write_all(b"BM")?;
        f.write_all(&file_size.to_le_bytes())?;
        f.write_all(&0u16.to_le_bytes())?; // reserved
        f.write_all(&0u16.to_le_bytes())?; // reserved
        f.write_all(&54u32.to_le_bytes())?; // pixel data offset

        // DIB header (BITMAPINFOHEADER, 40 bytes)
        f.write_all(&40u32.to_le_bytes())?; // header size
        f.write_all(&(width as i32).to_le_bytes())?;
        f.write_all(&(height as i32).to_le_bytes())?; // positive = bottom-up
        f.write_all(&1u16.to_le_bytes())?; // planes
        f.write_all(&24u16.to_le_bytes())?; // bpp
        f.write_all(&0u32.to_le_bytes())?; // compression (none)
        f.write_all(&pixel_data_size.to_le_bytes())?;
        f.write_all(&2835u32.to_le_bytes())?; // h-res (72 DPI)
        f.write_all(&2835u32.to_le_bytes())?; // v-res
        f.write_all(&0u32.to_le_bytes())?; // colors
        f.write_all(&0u32.to_le_bytes())?; // important colors

        // Pixel data (bottom-up, BGR)
        let pad = vec![0u8; row_padding as usize];
        for y in (0..height).rev() {
            for x in 0..width {
                let i = ((y * width + x) * 4) as usize;
                let r = rgba[i];
                let g = rgba[i + 1];
                let b = rgba[i + 2];
                f.write_all(&[b, g, r])?;
            }
            if row_padding > 0 {
                f.write_all(&pad)?;
            }
        }
        Ok(())
    }
}

impl player_ui_traits::PrintProvider for EguiPrintProvider {
    fn print(&self) -> bool {
        let (width, height, rgba) = match self.capture_frame() {
            Some(f) => f,
            None => {
                tracing::warn!("EguiPrintProvider::print: no frame available");
                return false;
            }
        };

        // Write to a temporary BMP file.
        let tmp_dir = std::env::temp_dir();
        let bmp_path = tmp_dir.join("flash_player_print.bmp");

        if let Err(e) = Self::write_bmp(width, height, &rgba, &bmp_path) {
            tracing::error!("EguiPrintProvider: failed to write BMP: {}", e);
            return false;
        }

        tracing::debug!(
            "EguiPrintProvider: wrote {}x{} frame to {:?}",
            width,
            height,
            bmp_path
        );

        // Platform-specific print command.
        #[cfg(target_os = "linux")]
        {
            // Try xdg-open first (opens in default image viewer which usually has print),
            // fall back to lpr for direct printing.
            match std::process::Command::new("lpr").arg(&bmp_path).status() {
                Ok(status) if status.success() => {
                    tracing::debug!("EguiPrintProvider: lpr succeeded");
                    return true;
                }
                Ok(status) => {
                    tracing::warn!("EguiPrintProvider: lpr exited with {}", status);
                }
                Err(e) => {
                    tracing::warn!("EguiPrintProvider: lpr failed: {}", e);
                }
            }
            // Fallback: open the file so the user can print from their viewer.
            let _ = std::process::Command::new("xdg-open").arg(&bmp_path).spawn();
            true
        }

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            // ShellExecute "print" verb.
            match std::process::Command::new("rundll32")
                .args(&[
                    "shimgvw.dll,ImageView_PrintTo",
                    &bmp_path.to_string_lossy(),
                ])
                .creation_flags(CREATE_NO_WINDOW)
                .status()
            {
                Ok(_) => true,
                Err(e) => {
                    tracing::error!("EguiPrintProvider: Windows print failed: {}", e);
                    false
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            match std::process::Command::new("lpr").arg(&bmp_path).status() {
                Ok(status) => status.success(),
                Err(e) => {
                    tracing::error!("EguiPrintProvider: lpr failed: {}", e);
                    false
                }
            }
        }

        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            tracing::warn!("EguiPrintProvider: printing not supported on this platform");
            false
        }
    }
}
