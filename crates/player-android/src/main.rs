//! Flash Player Android Host
//!
//! This binary runs inside a PRoot chroot on Android, communicating with
//! the Android app over Unix domain sockets.  It loads libpepflashplayer.so
//! (x86_64, translated by Box64) and proxies all platform interactions
//! through the IPC channel to the Android app.

mod android_audio;
mod android_audio_input;
mod android_clipboard;
mod android_context_menu;
mod android_cookie;
mod android_dialog;
mod android_file_chooser;
mod android_fullscreen;
mod android_http;
mod android_settings;
mod android_url;
mod android_video_capture;
mod ipc_transport;
mod protocol;

use ipc_transport::IpcTransport;
use parking_lot::Mutex;
use player_core::FlashPlayer;
use player_ui_traits::ViewInfo;
use protocol::{tags, PayloadReader, PayloadWriter, RawMessage};
use ppapi_sys::*;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Protocol version — must match the Android app.
const HOST_VERSION: &str = "1.0.0";

fn main() {
    // Initialize tracing
    init_tracing();

    // Redirect stdout/stderr to prevent plugin from writing garbage.
    unsafe {
        let devnull = libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_WRONLY);
        if devnull >= 0 {
            libc::dup2(devnull, 1);
            libc::dup2(devnull, 2);
            libc::close(devnull);
        }
    }

    tracing::info!("Flash Player Android Host starting");

    // Read IPC socket path from environment
    let socket_path = std::env::var("FLASH_IPC_SOCKET")
        .unwrap_or_else(|_| "/tmp/flash/control.sock".to_string());

    tracing::info!("Connecting to IPC socket: {}", socket_path);

    let ipc = match IpcTransport::connect(&socket_path) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to connect to IPC socket: {}", e);
            std::process::exit(1);
        }
    };

    // Send version
    let mut pw = PayloadWriter::new();
    pw.write_string(HOST_VERSION);
    if let Err(e) = ipc.send(tags::VERSION, pw.finish()) {
        tracing::error!("Failed to send version: {}", e);
        std::process::exit(1);
    }

    // Wait for the "open" command from Android
    tracing::info!("Waiting for open command...");
    let open_cmd = loop {
        match ipc.try_recv_command() {
            Some(msg) if msg.tag == tags::OPEN => break msg,
            Some(msg) => {
                tracing::debug!("Ignoring pre-open message tag=0x{:02x}", msg.tag);
            }
            None => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    };

    // Parse open command
    let mut pr = PayloadReader::new(&open_cmd.payload);
    let swf_url = pr.read_string().unwrap_or_default();
    let width = pr.read_u32().unwrap_or(800) as i32;
    let height = pr.read_u32().unwrap_or(600) as i32;
    let settings_json: serde_json::Value = pr
        .read_string()
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null);

    tracing::info!(
        "Open command: url={}, size={}x{}", swf_url, width, height
    );

    // Create shared providers
    let settings_provider = Arc::new(android_settings::AndroidSettingsProvider::new());
    if !settings_json.is_null() {
        settings_provider.update_from_json(&settings_json);
    }

    let fullscreen_provider = Arc::new(android_fullscreen::AndroidFullscreenProvider::new(ipc.clone()));
    fullscreen_provider.update_screen_size(width, height);

    let audio_input_provider = Arc::new(android_audio_input::AndroidAudioInputProvider::new(ipc.clone()));
    let video_capture_provider = Arc::new(android_video_capture::AndroidVideoCaptureProvider::new(ipc.clone()));
    let context_menu_provider = Arc::new(android_context_menu::AndroidContextMenuProvider::new(ipc.clone()));

    // Create the FlashPlayer
    let mut player = FlashPlayer::new();
    let frame_handle = player.latest_frame();
    let cursor_type_handle = player.cursor_type();

    // Signal new frame to the main loop
    let frame_ready = Arc::new(Mutex::new(false));
    let frame_ready_cb = frame_ready.clone();
    player.set_repaint_callback(move || {
        *frame_ready_cb.lock() = true;
    });

    // Navigation callback — tell Android to open URL
    let ipc_nav = ipc.clone();
    player.set_navigate_callback(move |url, target| {
        let mut pw = PayloadWriter::new();
        pw.write_string(url);
        pw.write_string(target);
        let _ = ipc_nav.send(tags::NAVIGATE, pw.finish());
    });

    // Resolve plugin path
    player.apply_default_plugin_path();

    // Initialize host
    let dialog_provider = Arc::new(android_dialog::AndroidDialogProvider::new(ipc.clone()));
    player.set_dialog_provider(dialog_provider);

    let file_chooser_provider = Arc::new(android_file_chooser::AndroidFileChooserProvider::new(ipc.clone()));
    player.set_file_chooser_provider(file_chooser_provider);

    if let Err(e) = player.init_host() {
        tracing::error!("Failed to initialize host: {}", e);
        send_state_change(&ipc, 3, 0, 0); // error state
        std::process::exit(1);
    }

    // Register all providers on the PPAPI host
    {
        let host = ppapi_host::HOST.get().expect("HOST not initialized");

        // URL provider
        host.set_url_provider(Box::new(android_url::AndroidUrlProvider::new(swf_url.clone())));

        // Settings
        host.set_settings_provider(Box::new(AndroidSettingsProviderWrapper(settings_provider.clone())));

        // Audio output
        host.set_audio_provider(Box::new(android_audio::AndroidAudioProvider::new(ipc.clone())));

        // Audio input
        host.set_audio_input_provider(Box::new(AndroidAudioInputProviderWrapper(audio_input_provider.clone())));

        // HTTP
        host.set_http_request_provider(Box::new(android_http::AndroidHttpProvider::new(ipc.clone())));

        // Clipboard
        host.set_clipboard_provider(Box::new(android_clipboard::AndroidClipboardProvider::new(ipc.clone())));

        // Fullscreen
        host.set_fullscreen_provider(Box::new(AndroidFullscreenProviderWrapper(fullscreen_provider.clone())));

        // Context menu
        host.set_context_menu_provider(Box::new(AndroidContextMenuProviderWrapper(context_menu_provider.clone())));

        // Cookie
        host.set_cookie_provider(Box::new(android_cookie::AndroidCookieProvider::new(ipc.clone())));

        // Video capture
        host.set_video_capture_provider(Box::new(AndroidVideoCaptureProviderWrapper(video_capture_provider.clone())));
    }

    // Load plugin
    tracing::info!("Loading Flash plugin...");
    if let Err(e) = player.load_plugin() {
        tracing::error!("Failed to load plugin: {}", e);
        send_state_change(&ipc, 3, 0, 0);
        std::process::exit(1);
    }
    tracing::info!("Plugin loaded successfully");

    // Open the SWF
    if let Err(e) = player.open_swf(&swf_url) {
        tracing::error!("Failed to open SWF: {}", e);
        send_state_change(&ipc, 3, 0, 0);
        std::process::exit(1);
    }

    // Send initial view change
    player.notify_view_change(width, height, Some(&ViewInfo {
        is_fullscreen: true,
        is_visible: true,
        is_page_visible: true,
        ..Default::default()
    }));

    // Send running state
    send_state_change(&ipc, 1, width, height);

    tracing::info!("Entering main loop");

    let mut last_cursor: i32 = -1;

    // Main loop
    loop {
        // 1. Poll PPAPI
        player.poll_main_loop();

        // 2. Send dirty frames
        {
            let ready = {
                let mut guard = frame_ready.lock();
                let r = *guard;
                *guard = false;
                r
            };

            if ready {
                send_dirty_frame(&frame_handle, &ipc);
            }
        }

        // 3. Check cursor changes
        {
            let cur = cursor_type_handle.load(Ordering::Relaxed);
            if cur != last_cursor {
                last_cursor = cur;
                let mut pw = PayloadWriter::new();
                pw.write_i32(cur);
                let _ = ipc.send(tags::CURSOR_CHANGE, pw.finish());
            }
        }

        // 4. Process incoming IPC commands
        while let Some(cmd) = ipc.try_recv_command() {
            match cmd.tag {
                tags::CLOSE => {
                    tracing::info!("Received close command");
                    player.shutdown();
                    return;
                }

                tags::RESIZE => {
                    if let Ok((w, h)) = parse_resize(&cmd) {
                        player.notify_view_change(w, h, Some(&ViewInfo {
                            is_fullscreen: true,
                            is_visible: true,
                            is_page_visible: true,
                            ..Default::default()
                        }));
                        fullscreen_provider.update_screen_size(w, h);
                    }
                }

                tags::FOCUS => {
                    let mut pr = PayloadReader::new(&cmd.payload);
                    if let Ok(has_focus) = pr.read_u8() {
                        player.notify_focus_change(has_focus != 0);
                    }
                }

                tags::MOUSE_DOWN | tags::MOUSE_UP | tags::MOUSE_MOVE => {
                    handle_mouse_event(&player, &cmd);
                }

                tags::MOUSE_ENTER => {
                    if let Some(instance_id) = player.instance_id() {
                        if let Some(host) = ppapi_host::HOST.get() {
                            let ev = ppapi_host::interfaces::input_event::InputEventResource::new_mouse(
                                PP_INPUTEVENT_TYPE_MOUSEENTER,
                                0.0,
                                0,
                                PP_INPUTEVENT_MOUSEBUTTON_NONE,
                                PP_Point { x: 0, y: 0 },
                                0,
                                PP_Point { x: 0, y: 0 },
                            );
                            let res_id = host.resources.insert(instance_id, Box::new(ev));
                            player.send_input_event(res_id);
                            host.resources.release(res_id);
                        }
                    }
                }

                tags::MOUSE_LEAVE => {
                    if let Some(instance_id) = player.instance_id() {
                        if let Some(host) = ppapi_host::HOST.get() {
                            let ev = ppapi_host::interfaces::input_event::InputEventResource::new_mouse(
                                PP_INPUTEVENT_TYPE_MOUSELEAVE,
                                0.0,
                                0,
                                PP_INPUTEVENT_MOUSEBUTTON_NONE,
                                PP_Point { x: 0, y: 0 },
                                0,
                                PP_Point { x: 0, y: 0 },
                            );
                            let res_id = host.resources.insert(instance_id, Box::new(ev));
                            player.send_input_event(res_id);
                            host.resources.release(res_id);
                        }
                    }
                }

                tags::WHEEL => {
                    handle_wheel_event(&player, &cmd);
                }

                tags::KEY_DOWN => {
                    handle_key_event(&player, &cmd, PP_INPUTEVENT_TYPE_KEYDOWN);
                }

                tags::KEY_UP => {
                    handle_key_event(&player, &cmd, PP_INPUTEVENT_TYPE_KEYUP);
                }

                tags::KEY_CHAR => {
                    handle_key_event(&player, &cmd, PP_INPUTEVENT_TYPE_CHAR);
                }

                tags::IME_COMPOSITION_START => {
                    player.send_ime_event(
                        PP_INPUTEVENT_TYPE_IME_COMPOSITION_START,
                        "",
                        &[],
                        -1,
                        0,
                        0,
                    );
                }

                tags::IME_COMPOSITION_UPDATE => {
                    let mut pr = PayloadReader::new(&cmd.payload);
                    if let Ok(text) = pr.read_string() {
                        let len = text.len() as u32;
                        player.send_ime_event(
                            PP_INPUTEVENT_TYPE_IME_COMPOSITION_UPDATE,
                            &text,
                            &[0, len],
                            0,
                            0,
                            len,
                        );
                    }
                }

                tags::IME_COMPOSITION_END => {
                    let mut pr = PayloadReader::new(&cmd.payload);
                    if let Ok(text) = pr.read_string() {
                        let len = text.len() as u32;
                        player.send_ime_event(
                            PP_INPUTEVENT_TYPE_IME_COMPOSITION_END,
                            &text,
                            &[0, len],
                            0,
                            0,
                            len,
                        );
                        // Also send as committed text
                        player.send_ime_event(
                            PP_INPUTEVENT_TYPE_IME_TEXT,
                            &text,
                            &[0, len],
                            0,
                            0,
                            len,
                        );
                    }
                }

                tags::AUDIO_INPUT_DATA => {
                    let mut pr = PayloadReader::new(&cmd.payload);
                    if let (Ok(stream_id), Ok(data)) = (pr.read_u32(), pr.read_bytes()) {
                        audio_input_provider.on_audio_input_data(stream_id, data);
                    }
                }

                tags::VIDEO_CAPTURE_DATA => {
                    let mut pr = PayloadReader::new(&cmd.payload);
                    if let (Ok(stream_id), Ok(w), Ok(h), Ok(data)) = (
                        pr.read_u32(),
                        pr.read_u32(),
                        pr.read_u32(),
                        pr.read_bytes(),
                    ) {
                        video_capture_provider.on_video_capture_data(stream_id, w, h, data);
                    }
                }

                tags::SETTINGS_UPDATE => {
                    if let Ok(json_str) = PayloadReader::new(&cmd.payload).read_string() {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&json_str) {
                            settings_provider.update_from_json(&json);
                        }
                    }
                }

                tags::VIEW_UPDATE => {
                    let mut pr = PayloadReader::new(&cmd.payload);
                    if let (Ok(visible), Ok(focused)) = (pr.read_u8(), pr.read_u8()) {
                        if visible != 0 {
                            player.notify_focus_change(focused != 0);
                        }
                    }
                }

                _ => {
                    tracing::debug!("Unknown command tag: 0x{:02x}", cmd.tag);
                }
            }
        }

        // 5. Brief sleep
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn init_tracing() {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::from_default_env()
        .add_directive(tracing::Level::INFO.into());

    let log_dir = std::env::var("FLASH_LOG_DIR")
        .unwrap_or_else(|_| "/tmp/flash".to_string());
    let _ = std::fs::create_dir_all(&log_dir);

    let log_path = std::path::Path::new(&log_dir).join("flash-host.log");
    if let Ok(log_file) = std::fs::File::create(&log_path) {
        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::sync::Mutex::new(log_file))
            .with_ansi(false);

        tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .init();
    } else {
        // Fallback: stderr logging (won't be visible after redirect)
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().with_ansi(false))
            .init();
    }
}

fn send_state_change(ipc: &IpcTransport, state: u8, width: i32, height: i32) {
    let mut pw = PayloadWriter::new();
    pw.write_u8(state);
    pw.write_i32(width);
    pw.write_i32(height);
    let _ = ipc.send(tags::STATE_CHANGE, pw.finish());
}

fn send_dirty_frame(
    frame_handle: &Arc<Mutex<Option<player_core::SharedFrameBuffer>>>,
    ipc: &IpcTransport,
) {
    let mut frame_guard = frame_handle.lock();
    let frame = match frame_guard.as_mut() {
        Some(f) => f,
        None => return,
    };

    let dirty = match frame.pending_dirty.take() {
        Some(d) => d,
        None => return,
    };

    let (dx, dy, dw, dh) = dirty;
    let frame_w = frame.width;
    let frame_h = frame.height;
    let stride = frame.stride as usize;

    // Extract the dirty region pixels
    let mut pixels = Vec::with_capacity((dw * dh * 4) as usize);
    for row in dy..(dy + dh) {
        let start = (row as usize) * stride + (dx as usize) * 4;
        let end = start + (dw as usize) * 4;
        if end <= frame.pixels.len() {
            pixels.extend_from_slice(&frame.pixels[start..end]);
        }
    }

    // Build the frame message
    let mut pw = PayloadWriter::with_capacity(24 + pixels.len());
    pw.write_u32(dx);
    pw.write_u32(dy);
    pw.write_u32(dw);
    pw.write_u32(dh);
    pw.write_u32(frame_w);
    pw.write_u32(frame_h);
    pw.write_raw(&pixels);

    if let Err(e) = ipc.send(tags::FRAME_READY, pw.finish()) {
        tracing::warn!("Failed to send frame: {}", e);
    }
}

fn parse_resize(cmd: &RawMessage) -> Result<(i32, i32), ()> {
    let mut pr = PayloadReader::new(&cmd.payload);
    let w = pr.read_u32().map_err(|_| ())? as i32;
    let h = pr.read_u32().map_err(|_| ())? as i32;
    Ok((w, h))
}

fn handle_mouse_event(player: &FlashPlayer, cmd: &RawMessage) {
    let mut pr = PayloadReader::new(&cmd.payload);
    let x = pr.read_u32().unwrap_or(0) as i32;
    let y = pr.read_u32().unwrap_or(0) as i32;
    let button = pr.read_u8().unwrap_or(0);
    let modifiers = pr.read_u32().unwrap_or(0);

    let event_type = match cmd.tag {
        tags::MOUSE_DOWN => PP_INPUTEVENT_TYPE_MOUSEDOWN,
        tags::MOUSE_UP => PP_INPUTEVENT_TYPE_MOUSEUP,
        tags::MOUSE_MOVE => PP_INPUTEVENT_TYPE_MOUSEMOVE,
        _ => return,
    };

    let pp_button = match button {
        0 => PP_INPUTEVENT_MOUSEBUTTON_LEFT,
        1 => PP_INPUTEVENT_MOUSEBUTTON_MIDDLE,
        2 => PP_INPUTEVENT_MOUSEBUTTON_RIGHT,
        _ => PP_INPUTEVENT_MOUSEBUTTON_NONE,
    };

    let click_count = if cmd.tag == tags::MOUSE_DOWN { 1 } else { 0 };

    player.send_mouse_event(
        event_type,
        pp_button,
        PP_Point { x, y },
        click_count,
        modifiers,
    );
}

fn handle_wheel_event(player: &FlashPlayer, cmd: &RawMessage) {
    let mut pr = PayloadReader::new(&cmd.payload);
    let dx = pr.read_f32().unwrap_or(0.0);
    let dy = pr.read_f32().unwrap_or(0.0);
    let modifiers = pr.read_u32().unwrap_or(0);

    player.send_wheel_event(
        PP_FloatPoint { x: dx, y: dy },
        PP_FloatPoint {
            x: dx / 120.0,
            y: dy / 120.0,
        },
        false,
        modifiers,
    );
}

fn handle_key_event(player: &FlashPlayer, cmd: &RawMessage, event_type: PP_InputEvent_Type) {
    let mut pr = PayloadReader::new(&cmd.payload);
    let key_code = pr.read_u32().unwrap_or(0);
    let modifiers = pr.read_u32().unwrap_or(0);
    let char_text = pr.read_string().unwrap_or_default();
    let code = pr.read_string().unwrap_or_default();

    player.send_keyboard_event(event_type, key_code, &char_text, &code, modifiers);
}

// =========================================================================
// Provider wrapper types (to satisfy trait object bounds)
// =========================================================================

struct AndroidSettingsProviderWrapper(Arc<android_settings::AndroidSettingsProvider>);
impl player_ui_traits::SettingsProvider for AndroidSettingsProviderWrapper {
    fn get_settings(&self) -> player_ui_traits::PlayerSettings {
        self.0.get_settings()
    }
}

struct AndroidFullscreenProviderWrapper(Arc<android_fullscreen::AndroidFullscreenProvider>);
impl player_ui_traits::FullscreenProvider for AndroidFullscreenProviderWrapper {
    fn is_fullscreen(&self) -> bool {
        self.0.is_fullscreen()
    }
    fn set_fullscreen(&self, fullscreen: bool) -> bool {
        self.0.set_fullscreen(fullscreen)
    }
    fn get_screen_size(&self) -> Option<(i32, i32)> {
        self.0.get_screen_size()
    }
}

struct AndroidAudioInputProviderWrapper(Arc<android_audio_input::AndroidAudioInputProvider>);
impl player_ui_traits::AudioInputProvider for AndroidAudioInputProviderWrapper {
    fn enumerate_devices(&self) -> Vec<(String, String)> {
        self.0.enumerate_devices()
    }
    fn open_stream(&self, device_id: Option<&str>, sample_rate: u32, sample_frame_count: u32) -> u32 {
        self.0.open_stream(device_id, sample_rate, sample_frame_count)
    }
    fn start_capture(&self, stream_id: u32) -> bool {
        self.0.start_capture(stream_id)
    }
    fn stop_capture(&self, stream_id: u32) {
        self.0.stop_capture(stream_id)
    }
    fn read_samples(&self, stream_id: u32, buffer: &mut [u8]) -> usize {
        self.0.read_samples(stream_id, buffer)
    }
    fn close_stream(&self, stream_id: u32) {
        self.0.close_stream(stream_id)
    }
}

struct AndroidContextMenuProviderWrapper(Arc<android_context_menu::AndroidContextMenuProvider>);
impl player_ui_traits::ContextMenuProvider for AndroidContextMenuProviderWrapper {
    fn show_context_menu(&self, items: &[player_ui_traits::ContextMenuItem], x: i32, y: i32) -> Option<i32> {
        self.0.show_context_menu(items, x, y)
    }
}

struct AndroidVideoCaptureProviderWrapper(Arc<android_video_capture::AndroidVideoCaptureProvider>);
impl player_ui_traits::VideoCaptureProvider for AndroidVideoCaptureProviderWrapper {
    fn enumerate_devices(&self) -> Vec<(String, String)> {
        self.0.enumerate_devices()
    }
    fn open_stream(&self, device_id: Option<&str>, width: u32, height: u32, fps: u32) -> u32 {
        self.0.open_stream(device_id, width, height, fps)
    }
    fn start_capture(&self, stream_id: u32) -> bool {
        self.0.start_capture(stream_id)
    }
    fn stop_capture(&self, stream_id: u32) {
        self.0.stop_capture(stream_id)
    }
    fn read_frame(&self, stream_id: u32) -> Option<player_ui_traits::VideoCaptureFrame> {
        self.0.read_frame(stream_id)
    }
    fn close_stream(&self, stream_id: u32) {
        self.0.close_stream(stream_id)
    }
}
