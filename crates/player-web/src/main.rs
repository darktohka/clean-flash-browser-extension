//! Flash Player Native Messaging Host
//!
//! This binary implements the Chrome/Firefox Native Messaging protocol.
//! It receives JSON commands over stdin (input events, open requests)
//! and sends frame updates over stdout (dirty regions as QOI-encoded RGBA data).
//!
//! # Protocol (extension → host)
//!
//! ```json
//! {"type": "open", "url": "https://example.com/game.swf",
//!  "args": [{"name": "allowScriptAccess", "value": "always"}]}
//! {"type": "resize", "width": 800, "height": 600}
//! {"type": "mousedown", "x": 100, "y": 200, "button": 0, "modifiers": 0}
//! {"type": "mouseup",   "x": 100, "y": 200, "button": 0, "modifiers": 0}
//! {"type": "mousemove", "x": 150, "y": 250, "modifiers": 0}
//! {"type": "mouseenter"}
//! {"type": "mouseleave"}
//! {"type": "keydown",  "keyCode": 13, "modifiers": 0}
//! {"type": "keyup",    "keyCode": 13, "modifiers": 0}
//! {"type": "char",     "keyCode": 13, "text": "\r", "code": "Enter", "modifiers": 0}
//! {"type": "wheel",    "deltaX": 0, "deltaY": -120, "modifiers": 0}
//! {"type": "focus",    "hasFocus": true}
//! {"type": "close"}
//! ```
//!
//! # Protocol (host → extension)
//!
//! ```json
//! {"type": "frame", "x": 0, "y": 0, "width": 800, "height": 600,
//!  "frameWidth": 800, "frameHeight": 600, "stride": 3200,
//!  "data": "<base64 BGRA pixels of the dirty region>"}
//! {"type": "state", "state": "running", "width": 800, "height": 600}
//! {"type": "cursor", "cursor": 0}
//! {"type": "error", "message": "..."}
//! ```

mod http_fetch;
mod protocol;
mod qoi_encode;
mod script_bridge;

use parking_lot::Mutex;
use player_core::FlashPlayer;
use player_ui_traits::{
    EmbedArg, PlayerSettings, ViewInfo,
};
use ppapi_sys::*;
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

fn main() {
    // Save the real stdout and stdin fds before we redirect stdout to
    // /dev/null.  Using saved handles makes the host immune to any later
    // interference (CRT _dup2, SetStdHandle, or Flash plugin side-effects).
    protocol::init_saved_handles();

    // Tracing is opt-in. If TRACE_FLASH is unset, no subscriber is installed,
    // no log file is created, and tracing events are dropped.
    let _trace_guard = init_tracing_if_enabled();

    // Redirect stdout and stderr to /dev/null (Unix) or NUL (Windows) so
    // that libpepflashplayer (and any other native code) cannot write to
    // them and corrupt the native messaging channel.  Our
    // protocol::send_host_message() uses the saved handle from above.
    #[cfg(unix)]
    unsafe {
        let devnull = libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_WRONLY);
        if devnull >= 0 {
            libc::dup2(devnull, 1); // stdout → /dev/null
            libc::dup2(devnull, 2); // stderr → /dev/null
            libc::close(devnull);
        }
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{
            SetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
        };

        extern "C" {
            fn _open(filename: *const i8, oflag: i32, ...) -> i32;
            fn _dup2(fd1: i32, fd2: i32) -> i32;
            fn _get_osfhandle(fd: i32) -> isize;
        }

        unsafe {
            // Open NUL via the C runtime so we get a proper CRT file descriptor.
            let nul_fd = _open(b"NUL\0".as_ptr().cast::<i8>(), 1 /*_O_WRONLY*/);
            assert!(nul_fd >= 0, "failed to open NUL device");

            // Redirect CRT file descriptors 1 (stdout) and 2 (stderr) to NUL.
            //
            // The previous approach used only SetStdHandle(), which updates
            // the Win32-level handle returned by GetStdHandle().  However the
            // MSVC/UCRT runtime keeps its own fd→handle table that is NOT
            // touched by SetStdHandle.  Native code loaded into the process
            // (libpepflashplayer) that calls printf() or _write(1, …) goes
            // through the CRT layer, so without _dup2 those writes still
            // reached Chrome's original stdout pipe and corrupted the
            // native-messaging channel - causing Chrome to disconnect and the
            // host to see an unexpected EOF on stdin.
            //
            // On Unix, dup2() covers both layers because the CRT fd IS the
            // kernel fd.  On Windows they are separate, so we must update both.
            _dup2(nul_fd, 1); // CRT stdout → NUL
            _dup2(nul_fd, 2); // CRT stderr → NUL

            // Also update the Win32-level standard handles so that Rust's
            // io::stdout()/io::stderr() (which call GetStdHandle) see NUL.
            // MSVC's _dup2 already does this for fds 0–2, but we call
            // SetStdHandle explicitly to be safe across CRT versions.
            SetStdHandle(STD_OUTPUT_HANDLE, _get_osfhandle(1) as *mut std::ffi::c_void);
            SetStdHandle(STD_ERROR_HANDLE, _get_osfhandle(2) as *mut std::ffi::c_void);
        }
    }

    tracing::info!("Flash Player Native Messaging Host starting");

    let mut player = FlashPlayer::new();
    let frame_handle = player.latest_frame();
    let cursor_type_handle = player.cursor_type();
    let _state_handle = player.state();

    // Signal that a new frame is available so the main loop sends it.
    let frame_ready = Arc::new(Mutex::new(false));
    let frame_ready_cb = frame_ready.clone();
    player.set_repaint_callback(move || {
        *frame_ready_cb.lock() = true;
    });

    // When Flash requests navigation, send a Navigate message to the extension.
    player.set_navigate_callback(|url, target| {
        let _ = protocol::send_host_message(&protocol::HostMessage::Navigate {
            url,
            target,
        });
    });

    // Resolve the plugin path (env var → default name → CWD scan).
    player.apply_default_plugin_path();

    if let Err(e) = player.init_host() {
        let _ = protocol::send_host_message(&protocol::HostMessage::Error(
            &format!("Failed to initialize host: {}", e),
        ));
        std::process::exit(1);
    }

    // Set up the JavaScript scripting bridge so that the PPAPI host can
    // proxy GetWindowObject / ExecuteScript / property access / method
    // calls through the real browser DOM via the Chrome Extension.
    let script_bridge = Arc::new(script_bridge::ScriptBridge::new());
    SCRIPT_BRIDGE
        .set(script_bridge.clone())
        .expect("SCRIPT_BRIDGE already initialised");

    {
        let host = ppapi_host::HOST.get().expect("HOST not initialised");
        host.set_script_provider(Box::new(
            script_bridge::WebScriptProvider::new(script_bridge.clone()),
        ));
        host.set_url_provider(Box::new(WebUrlProvider::new(script_bridge.clone())));

        // Set up the settings provider early so backend selection can read it.
        let settings = Arc::new(WebSettingsProvider::new());
        let settings_bridge = SCRIPT_BRIDGE.get().expect("SCRIPT_BRIDGE not initialised").clone();
        let _ = settings.bridge.set(settings_bridge);
        WEB_SETTINGS
            .set(settings.clone())
            .ok()
            .expect("WEB_SETTINGS already initialised");
        host.set_settings_provider(Box::new(WebSettingsProviderWrapper(settings.clone())));

        // Set up the URL rewrite provider backed by extension settings rules
        // (and optional JS callback for custom rewriting).
        let url_rewrite = Arc::new(WebUrlRewriteProvider::new(
            SCRIPT_BRIDGE.get().expect("SCRIPT_BRIDGE not initialised").clone(),
        ));
        WEB_URL_REWRITE
            .set(url_rewrite.clone())
            .ok()
            .expect("WEB_URL_REWRITE already initialised");
        host.set_url_rewrite_provider(Box::new(WebUrlRewriteProviderWrapper(url_rewrite)));

        // Set up the audio input provider so that Flash can capture from
        // the browser's microphone via MediaStream / getUserMedia.
        let audio_input = Arc::new(WebAudioInputProvider::new());
        WEB_AUDIO_INPUT
            .set(audio_input.clone())
            .ok()
            .expect("WEB_AUDIO_INPUT already initialised");
        host.set_audio_input_provider(Box::new(WebAudioInputProviderWrapper(audio_input)));

        // Set up the clipboard provider so that Flash clipboard operations
        // are forwarded to the browser via the script bridge.
        let clip_bridge = SCRIPT_BRIDGE.get().expect("SCRIPT_BRIDGE not initialised").clone();
        host.set_clipboard_provider(Box::new(WebClipboardProvider::new(clip_bridge)));

        // Set up the fullscreen provider so that Flash fullscreen requests
        // are forwarded to the browser's Fullscreen API.
        let fs_bridge = SCRIPT_BRIDGE.get().expect("SCRIPT_BRIDGE not initialised").clone();
        host.set_fullscreen_provider(Box::new(WebFullscreenProvider::new(fs_bridge)));

        // Set up the cursor lock provider so that Flash cursor lock requests
        // are forwarded to the browser's Pointer Lock API.
        let cl_bridge = SCRIPT_BRIDGE.get().expect("SCRIPT_BRIDGE not initialised").clone();
        host.set_cursor_lock_provider(Box::new(WebCursorLockProvider::new(cl_bridge)));

        // Set up the context menu provider so that Flash right-click menus
        // are forwarded to the browser extension for display.
        let ctx_menu = Arc::new(WebContextMenuProvider::new());
        WEB_CONTEXT_MENU
            .set(ctx_menu.clone())
            .ok()
            .expect("WEB_CONTEXT_MENU already initialised");
        host.set_context_menu_provider(Box::new(WebContextMenuProviderWrapper(ctx_menu)));

        // Set up the print provider so that Flash print requests are
        // forwarded to the browser (window.print()).
        host.set_print_provider(Box::new(WebPrintProvider));

        // Set up the video capture provider so that Flash can capture from
        // the browser's webcam via MediaStream / getUserMedia({ video }).
        let video_capture = Arc::new(WebVideoCaptureProvider::new());
        WEB_VIDEO_CAPTURE
            .set(video_capture.clone())
            .ok()
            .expect("WEB_VIDEO_CAPTURE already initialised");
        host.set_video_capture_provider(Box::new(WebVideoCaptureProviderWrapper(video_capture)));

        // Set up the file chooser provider using native OS dialogs (rfd).
        // This spawns a worker thread that must exist BEFORE load_plugin()
        // activates the seccomp sandbox.
        host.set_file_chooser_provider(Box::new(
            player_ui_traits::RfdFileChooserProvider::new(),
        ));

        // Set up the cookie provider so that HTTP cookies from the browser
        // are attached to URLLoader requests and Set-Cookie responses are
        // stored back in the browser cookie jar.
        let cookie_bridge = SCRIPT_BRIDGE.get().expect("SCRIPT_BRIDGE not initialised").clone();
        host.set_cookie_provider(Box::new(WebCookieProvider::new(cookie_bridge)));

        // Set up the HTTP request provider.  Uses a dispatching provider
        // that checks the preferNetworkBrowser setting at request time:
        // - When true (default): routes through the browser's fetch() API
        //   via the script bridge, with optional reqwest fallback on CORS.
        // - When false: uses reqwest (direct HTTP) exclusively.
        let fetch_bridge = SCRIPT_BRIDGE.get().expect("SCRIPT_BRIDGE not initialised").clone();
        let reqwest_provider = std::sync::Arc::new(
            ppapi_host::http_reqwest::ReqwestHttpRequestProvider::new(),
        );
        let fetch_provider = http_fetch::FetchHttpRequestProvider::new(fetch_bridge)
            .with_fallback(reqwest_provider.clone());
        host.set_http_request_provider(Box::new(
            http_fetch::DispatchingHttpRequestProvider::new(fetch_provider, reqwest_provider),
        ));

    }

    // Plugin loading is deferred until the first "open" message so that
    // browser-supplied settings (GL backend, audio backend, …) are
    // available before pre-sandbox initialization runs.

    tracing::info!("Entering main loop");

    // Send initial state.
    let _ = protocol::send_host_message(&protocol::HostMessage::State {
        code: 0, // idle
        width: 0,
        height: 0,
    });

    let mut last_cursor: i32 = -1;

    // Main loop: poll stdin for commands, send frames when ready.
    loop {
        // ---- Poll the PPAPI main-thread message loop ----
        player.poll_main_loop();

        // ---- Check for pending frame updates ----
        {
            let ready = {
                let mut guard = frame_ready.lock();
                let r = *guard;
                *guard = false;
                r
            };

            if ready {
                send_dirty_frame(&frame_handle);
            }
        }

        // ---- Check cursor changes ----
        {
            let cur = cursor_type_handle.load(Ordering::Relaxed);
            if cur != last_cursor {
                last_cursor = cur;
                let _ = protocol::send_host_message(&protocol::HostMessage::Cursor(cur));
            }
        }

        // ---- Read next command (non-blocking via timeout) ----
        // We use a short sleep + non-blocking check pattern.
        // Native messaging stdin blocks, so we read on a helper thread.
        match try_read_command() {
            Some(cmd) => {
                if !handle_command(&mut player, &cmd, &frame_ready) {
                    break; // Extension disconnected or "close" received.
                }
            }
            None => {
                // No message available yet; yield briefly.
                std::thread::sleep(std::time::Duration::from_millis(4));
            }
        }
    }

    player.shutdown();
    tracing::info!("Flash Player Native Messaging Host exiting");
}

fn init_tracing_if_enabled() -> Option<WorkerGuard> {
    //if std::env::var_os("TRACE_FLASH").is_none() {
    //    return None;
    //}

    let filter = EnvFilter::new("trace");
    let log_dir = std::env::var("FLASH_PLAYER_LOG_DIR")
        .unwrap_or_else(|_| "/home/user/flash-player-host".into());
    let _ = std::fs::create_dir_all(&log_dir);
    let timestamp = chrono_timestamp();
    let log_filename = format!("flash-player-host-{}.log", timestamp);
    let log_path = std::path::Path::new(&log_dir).join(&log_filename);
    let log_file = std::fs::File::create(&log_path).expect("failed to create log file");

    use std::io::Write;
    let mut header_file = log_file.try_clone().expect("failed to clone log file handle");
    let _ = writeln!(header_file, "=== Flash Player Native Messaging Host ===");
    let _ = writeln!(header_file, "=== Started: {} ===", timestamp);
    let _ = writeln!(header_file);

    let (file_writer, guard) = tracing_appender::non_blocking(log_file);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false);

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .init();

    tracing::info!("Log file: {}", log_path.display());
    Some(guard)
}

// ===========================================================================
// Threaded stdin reader - makes stdin non-blocking for the main loop
// ===========================================================================

use std::sync::mpsc;
use std::sync::OnceLock;

/// Global script bridge for routing `jsResponse` messages from the stdin
/// reader thread to the blocking `ScriptProvider` calls.
static SCRIPT_BRIDGE: OnceLock<Arc<script_bridge::ScriptBridge>> = OnceLock::new();

/// Global context menu provider for routing `menuResponse` messages.
static WEB_CONTEXT_MENU: OnceLock<Arc<WebContextMenuProvider>> = OnceLock::new();

/// Global settings provider for live settings updates.
pub(crate) static WEB_SETTINGS: OnceLock<Arc<WebSettingsProvider>> = OnceLock::new();

/// Global URL rewrite provider for resolving the JS callback.
static WEB_URL_REWRITE: OnceLock<Arc<WebUrlRewriteProvider>> = OnceLock::new();

fn apply_audio_provider_from_settings(host: &ppapi_host::HostState) {
    let use_native_audio = WEB_SETTINGS
        .get()
        .map(|ws| *ws.audio_backend_native.lock())
        .unwrap_or(false);

    if use_native_audio {
        tracing::info!("Selecting native (cpal) audio output for Flash");
        host.switch_audio_provider(Box::new(
            ppapi_host::audio_cpal::CpalAudioProvider::new(),
        ));
    } else {
        tracing::info!("Selecting Web Audio API for Flash audio output");
        host.switch_audio_provider(Box::new(WebAudioProvider::new()));
    }
}

/// Lazily-initialized channel that receives messages from a background reader.
fn try_read_command() -> Option<serde_json::Value> {
    static RX: OnceLock<Mutex<mpsc::Receiver<Option<serde_json::Value>>>> = OnceLock::new();

    let rx = RX.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        let bridge = SCRIPT_BRIDGE.get().cloned();
        let audio_input = WEB_AUDIO_INPUT.get().cloned();
        let context_menu = WEB_CONTEXT_MENU.get().cloned();
        let video_capture = WEB_VIDEO_CAPTURE.get().cloned();
        std::thread::Builder::new()
            .name("stdin-reader".into())
            .spawn(move || {
                loop {
                    match protocol::read_message() {
                        Ok(Some(msg)) => {
                            // Route jsResponse messages to the script bridge
                            // instead of the normal command channel.
                            if msg.get("type").and_then(|v| v.as_str()) == Some("jsResponse") {
                                if let Some(ref b) = bridge {
                                    b.handle_response(&msg);
                                }
                                continue;
                            }
                            // Route audioInputData messages to the audio
                            // input provider's ring buffer.
                            if msg.get("type").and_then(|v| v.as_str()) == Some("audioInputData") {
                                if let Some(ref ai) = audio_input {
                                    ai.handle_audio_data(&msg);
                                }
                                continue;
                            }
                            // Route videoCaptureData messages to the video
                            // capture provider's frame buffer.
                            if msg.get("type").and_then(|v| v.as_str()) == Some("videoCaptureData") {
                                if let Some(ref vc) = video_capture {
                                    vc.handle_video_data(&msg);
                                }
                                continue;
                            }
                            // Route menuResponse messages to the context
                            // menu provider's pending channel.
                            if msg.get("type").and_then(|v| v.as_str()) == Some("menuResponse") {
                                if let Some(ref cm) = context_menu {
                                    cm.handle_response(&msg);
                                }
                                continue;
                            }
                            if tx.send(Some(msg)).is_err() {
                                break;
                            }
                        }
                        Ok(None) => {
                            // EOF - extension closed.
                            let _ = tx.send(None);
                            break;
                        }
                        Err(e) => {
                            tracing::error!("stdin read error: {}", e);
                            let _ = tx.send(None);
                            break;
                        }
                    }
                }
            })
            .expect("failed to spawn stdin reader thread");
        Mutex::new(rx)
    });

    let guard = rx.lock();
    match guard.try_recv() {
        Ok(Some(msg)) => Some(msg),
        Ok(None) => {
            // EOF sentinel - signal shutdown.
            Some(json!({"type": "eof"}))
        }
        Err(mpsc::TryRecvError::Empty) => None,
        Err(mpsc::TryRecvError::Disconnected) => Some(json!({"type": "eof"})),
    }
}

// ===========================================================================
// Command handling
// ===========================================================================

fn parse_open_embed_args(cmd: &serde_json::Value) -> Vec<EmbedArg> {
    let Some(raw_args) = cmd.get("args").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    raw_args
        .iter()
        .filter_map(|raw| {
            let name = raw.get("name").and_then(|v| v.as_str())?.trim();
            if name.is_empty() {
                return None;
            }
            let value = raw
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Some(EmbedArg {
                name: name.to_string(),
                value,
            })
        })
        .collect()
}

/// Extract browser-sourced view metadata from a JSON command.
///
/// Used by `resize` and `viewUpdate` commands. Missing fields fall back
/// to sensible defaults.
fn parse_view_info(cmd: &serde_json::Value) -> ViewInfo {
    ViewInfo {
        device_scale: cmd["deviceScale"].as_f64().unwrap_or(1.0) as f32,
        css_scale: cmd["cssScale"].as_f64().unwrap_or(1.0) as f32,
        scroll_offset_x: cmd["scrollX"].as_i64().unwrap_or(0) as i32,
        scroll_offset_y: cmd["scrollY"].as_i64().unwrap_or(0) as i32,
        is_fullscreen: cmd["isFullscreen"].as_bool().unwrap_or(false),
        is_visible: cmd["isVisible"].as_bool().unwrap_or(true),
        is_page_visible: cmd["isPageVisible"].as_bool().unwrap_or(true),
    }
}

/// Handle one incoming JSON command. Returns `false` to signal shutdown.
fn handle_command(
    player: &mut FlashPlayer,
    cmd: &serde_json::Value,
    frame_ready: &Arc<Mutex<bool>>,
) -> bool {
    let msg_type = cmd["type"].as_str().unwrap_or("");

    match msg_type {
        "open" => {
            let url = cmd["url"].as_str().unwrap_or("");
            if url.is_empty() {
                let _ = protocol::send_host_message(&protocol::HostMessage::Error(
                    "missing 'url' in open command",
                ));
                return true;
            }
            let embed_args = parse_open_embed_args(cmd);

            // Store browser-sourced flash settings (incognito, language).
            if let Some(host) = ppapi_host::HOST.get() {
                if let Some(incognito) = cmd["incognito"].as_bool() {
                    host.set_flash_incognito(incognito);
                    tracing::info!("Browser incognito mode: {}", incognito);
                }
                if let Some(lang) = cmd["language"].as_str() {
                    if !lang.is_empty() {
                        host.set_flash_language(lang);
                        tracing::info!("Browser language: {}", lang);
                    }
                }
            }

            // Apply extension settings from the open command.
            if let Some(settings_obj) = cmd.get("settings") {
                tracing::info!("Applying initial settings from open command");
                if let Some(ws) = WEB_SETTINGS.get() {
                    ws.update_from_json(settings_obj);
                }
                if let Some(host) = ppapi_host::HOST.get() {
                    apply_audio_provider_from_settings(host);
                }
            }

            // Resolve the JS URL rewrite callback if provided.
            // The open command may include a `urlRewriterCallbackId` field
            // (a JS object handle for a `(url: string) => string` function).
            if let Some(cb_id) = cmd.get("urlRewriterCallbackId").and_then(|v| v.as_u64()) {
                if let Some(rewriter) = WEB_URL_REWRITE.get() {
                    tracing::info!("URL rewriter JS callback set (object_id={})", cb_id);
                    rewriter.set_js_callback(cb_id);
                }
            }

            // On the first open, load the plugin now that settings are
            // available.  pre_sandbox_init (EGL, audio thread) runs
            // inside load_plugin, and the seccomp sandbox is activated
            // immediately after.
            if !player.is_plugin_loaded() {
                if let Err(e) = player.load_plugin() {
                    let _ = protocol::send_host_message(&protocol::HostMessage::Error(
                        &format!("Failed to load plugin: {}", e),
                    ));
                    return true;
                }
            }

            tracing::info!("Opening SWF: {}", url);
            tracing::info!("Open command includes {} DidCreate args", embed_args.len());
            match player.open_swf_with_args(url, &embed_args) {
                Ok(()) => {
                    // Apply initial view metadata sent in the start/open message
                    // so PPB_View reflects browser state from the very beginning.
                    let w = cmd["width"].as_i64().unwrap_or(0) as i32;
                    let h = cmd["height"].as_i64().unwrap_or(0) as i32;
                    if w > 0 && h > 0 {
                        let view_info = parse_view_info(cmd);
                        player.notify_view_change(w, h, Some(&view_info));
                    }

                    // Force a frame update immediately after boot so the
                    // browser receives the initial view without waiting for
                    // the first repaint callback.
                    *frame_ready.lock() = true;

                    let _ = protocol::send_host_message(&protocol::HostMessage::State {
                        code: 2, // running
                        width: 0,
                        height: 0,
                    });
                }
                Err(e) => {
                    let _ = protocol::send_host_message(&protocol::HostMessage::Error(
                        &format!("Failed to open SWF: {}", e),
                    ));
                }
            }
        }

        "resize" => {
            let w = cmd["width"].as_i64().unwrap_or(0) as i32;
            let h = cmd["height"].as_i64().unwrap_or(0) as i32;
            if w > 0 && h > 0 {
                let view_info = parse_view_info(cmd);
                player.notify_view_change(w, h, Some(&view_info));
            }
        }

        "viewUpdate" => {
            // View metadata changed (visibility, scroll, fullscreen) without resize.
            // Re-send DidChangeView with current dimensions + updated view info.
            if let Some(host) = ppapi_host::HOST.get() {
                if let Some(instance_id) = player.instance_id() {
                    let rect = host.instances.with_instance(instance_id, |inst| inst.view_rect);
                    if let Some(rect) = rect {
                        let w = rect.size.width;
                        let h = rect.size.height;
                        if w > 0 && h > 0 {
                            let view_info = parse_view_info(cmd);
                            player.notify_view_change(w, h, Some(&view_info));
                        }
                    }
                }
            }
        }

        "mousedown" | "mouseup" | "mousemove" | "mouseenter" | "mouseleave" | "contextmenu" => {
            let x = cmd["x"].as_f64().unwrap_or(0.0) as i32;
            let y = cmd["y"].as_f64().unwrap_or(0.0) as i32;
            let btn_idx = cmd["button"].as_i64().unwrap_or(-1);
            let modifiers = cmd["modifiers"].as_u64().unwrap_or(0) as u32;

            let event_type = match msg_type {
                "mousedown" => PP_INPUTEVENT_TYPE_MOUSEDOWN,
                "mouseup" => PP_INPUTEVENT_TYPE_MOUSEUP,
                "mousemove" => PP_INPUTEVENT_TYPE_MOUSEMOVE,
                "mouseenter" => PP_INPUTEVENT_TYPE_MOUSEENTER,
                "mouseleave" => PP_INPUTEVENT_TYPE_MOUSELEAVE,
                "contextmenu" => PP_INPUTEVENT_TYPE_CONTEXTMENU,
                _ => unreachable!(),
            };

            let button = match btn_idx {
                0 => PP_INPUTEVENT_MOUSEBUTTON_LEFT,
                1 => PP_INPUTEVENT_MOUSEBUTTON_MIDDLE,
                2 => PP_INPUTEVENT_MOUSEBUTTON_RIGHT,
                _ => PP_INPUTEVENT_MOUSEBUTTON_NONE,
            };

            let click_count = if msg_type == "mousedown" || msg_type == "mouseup" {
                1
            } else {
                0
            };

            player.send_mouse_event(
                event_type,
                button,
                PP_Point { x, y },
                click_count,
                modifiers,
            );
        }

        "keydown" | "rawkeydown" | "keyup" | "char" => {
            let key_code = cmd["keyCode"].as_u64().unwrap_or(0) as u32;
            let modifiers = cmd["modifiers"].as_u64().unwrap_or(0) as u32;
            let text = cmd["text"].as_str().unwrap_or("");
            let code = cmd["code"].as_str().unwrap_or("");

            let event_type = match msg_type {
                // Chrome PPAPI sends RAWKEYDOWN for physical key presses;
                // PepperFlash expects this type for text field handling.
                "keydown" | "rawkeydown" => PP_INPUTEVENT_TYPE_RAWKEYDOWN,
                "keyup" => PP_INPUTEVENT_TYPE_KEYUP,
                "char" => PP_INPUTEVENT_TYPE_CHAR,
                _ => unreachable!(),
            };

            player.send_keyboard_event(event_type, key_code, text, code, modifiers);
        }

        "compositionstart" => {
            player.send_ime_event(
                PP_INPUTEVENT_TYPE_IME_COMPOSITION_START,
                "",
                &[],
                -1,
                0,
                0,
            );
        }

        "compositionupdate" => {
            let text = cmd["text"].as_str().unwrap_or("");
            let text_len = text.len() as u32;
            // Single segment spanning the whole composition string.
            let segment_offsets = [0u32, text_len];
            player.send_ime_event(
                PP_INPUTEVENT_TYPE_IME_COMPOSITION_UPDATE,
                text,
                &segment_offsets,
                0,
                0,
                text_len,
            );
        }

        "compositionend" => {
            let text = cmd["text"].as_str().unwrap_or("");
            let text_len = text.len() as u32;
            let segment_offsets = [0u32, text_len];
            player.send_ime_event(
                PP_INPUTEVENT_TYPE_IME_COMPOSITION_END,
                text,
                &segment_offsets,
                -1,
                0,
                text_len,
            );
            // Also send IME_TEXT so Flash commits the composed text.
            player.send_ime_event(
                PP_INPUTEVENT_TYPE_IME_TEXT,
                text,
                &segment_offsets,
                -1,
                0,
                text_len,
            );
        }

        "wheel" => {
            let dx = cmd["deltaX"].as_f64().unwrap_or(0.0) as f32;
            let dy = cmd["deltaY"].as_f64().unwrap_or(0.0) as f32;
            let modifiers = cmd["modifiers"].as_u64().unwrap_or(0) as u32;

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

        "focus" => {
            let has_focus = cmd["hasFocus"].as_bool().unwrap_or(true);
            player.notify_focus_change(has_focus);
        }

        "callFunction" => {
            // ExternalInterface: JS → AS.  The invoke XML was sent by
            // page-script.js when JavaScript called a registered callback
            // (e.g. game.startup(…)).  Route it to PepperFlash's scriptable
            // object.
            let xml = cmd["xml"].as_str().unwrap_or("");
            if xml.is_empty() {
                tracing::warn!("callFunction: empty XML");
            } else {
                tracing::info!("callFunction: routing invoke XML to PepperFlash");
                tracing::debug!("callFunction XML: {}", xml);
                let host = ppapi_host::HOST.get().expect("HOST not initialised");
                let result = unsafe { host.handle_external_call(xml) };
                if let Some(ref js) = result {
                    tracing::debug!("callFunction result: {}", js);
                }
                // For now the result is discarded - the JS caller already
                // returned undefined (fire-and-forget).  If synchronous
                // return values are needed in the future, send the result
                // back to the extension here.
            }
        }

        "cursorLockChanged" => {
            // Browser reports that pointer lock state changed (e.g. user
            // pressed Escape, or requestPointerLock succeeded/failed).
            let locked = cmd["locked"].as_bool().unwrap_or(false);
            tracing::debug!("cursorLockChanged: locked={}", locked);
            if let Some(host) = ppapi_host::HOST.get() {
                if let Some(instance_id) = player.instance_id() {
                    host.set_cursor_lock_state(instance_id, locked);
                }
            }
        }

        "settingsUpdate" => {
            // Live settings update from the browser extension settings popup.
            if let Some(settings_obj) = cmd.get("settings") {
                tracing::info!("Settings updated live");
                if let Some(ws) = WEB_SETTINGS.get() {
                    ws.update_from_json(settings_obj);
                }
                if let Some(host) = ppapi_host::HOST.get() {
                    apply_audio_provider_from_settings(host);
                }
            }
        }

        "close" | "eof" => {
            tracing::info!("Received {} - shutting down", msg_type);
            return false;
        }

        other => {
            tracing::warn!("Unknown message type: {}", other);
        }
    }

    true
}

// ===========================================================================
// Frame sending - only dirty region
// ===========================================================================

/// Extract the pending dirty region from the shared frame buffer and send
/// it to the extension as a chunked binary message.
fn send_dirty_frame(frame_handle: &Arc<Mutex<Option<player_core::SharedFrameBuffer>>>) {
    let mut guard = frame_handle.lock();
    let Some(buf) = guard.as_mut() else { return };
    let Some((dx, dy, dw, dh)) = buf.pending_dirty.take() else {
        return;
    };

    if dw == 0 || dh == 0 {
        return;
    }

    let stride = buf.stride;
    let frame_w = buf.width;
    let frame_h = buf.height;

    // Extract dirty sub-rectangle pixels.
    let row_bytes = (dw * 4) as usize;
    let mut region = Vec::with_capacity(row_bytes * dh as usize);
    for row in 0..dh {
        let y = dy + row;
        let offset = (y * stride + dx * 4) as usize;
        let end = offset + row_bytes;
        if end <= buf.pixels.len() {
            region.extend_from_slice(&buf.pixels[offset..end]);
        }
    }

    // Release the lock before the potentially slow encode + I/O.
    drop(guard);

    // QOI-encode the dirty region (converts BGRA → RGBA on the fly).
    let qoi_data = qoi_encode::qoi_encode_bgra(&region, dw, dh);

    let _ = protocol::send_host_message(&protocol::HostMessage::Frame {
        x: dx,
        y: dy,
        width: dw,
        height: dh,
        frame_width: frame_w,
        frame_height: frame_h,
        stride,
        pixels: &qoi_data,
    });
}

// ===========================================================================
// Context menu provider - sends Flash menus to the browser extension
// ===========================================================================

/// Context menu provider that sends menu items to the Chrome Extension
/// for display as a DOM-based context menu and blocks until the user
/// selects an item or dismisses the menu.
struct WebContextMenuProvider {
    /// Channel for receiving the user's menu response.
    pending: Mutex<Option<mpsc::Sender<Option<i32>>>>,
}

impl WebContextMenuProvider {
    fn new() -> Self {
        Self {
            pending: Mutex::new(None),
        }
    }

    /// Called by the stdin reader thread when a `menuResponse` message
    /// arrives from the browser extension.
    fn handle_response(&self, msg: &serde_json::Value) {
        let selected_id = msg.get("selectedId").and_then(|v| v.as_i64()).map(|v| v as i32);

        let mut guard = self.pending.lock();
        if let Some(tx) = guard.take() {
            let _ = tx.send(selected_id);
        } else {
            tracing::warn!("menuResponse received but no pending request");
        }
    }
}

impl player_ui_traits::ContextMenuProvider for WebContextMenuProvider {
    fn show_context_menu(
        &self,
        items: &[player_ui_traits::ContextMenuItem],
        x: i32,
        y: i32,
    ) -> Option<i32> {
        // Serialize items to JSON.
        let json_items = serialize_menu_items(items);
        let payload = serde_json::json!({
            "items": json_items,
            "x": x,
            "y": y,
        });
        let json_str = payload.to_string();

        // Register the pending waiter *before* sending.
        let (tx, rx) = mpsc::channel();
        *self.pending.lock() = Some(tx);

        // Send via the binary protocol.
        if let Err(e) = protocol::send_host_message(&protocol::HostMessage::ContextMenu(&json_str))
        {
            tracing::error!("failed to send ContextMenu message: {}", e);
            *self.pending.lock() = None;
            return None;
        }

        // Block until the response arrives (generous timeout: menus can
        // stay open for a long time).
        match rx.recv_timeout(std::time::Duration::from_secs(120)) {
            Ok(selected) => selected,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                tracing::warn!("ContextMenu response timed out");
                *self.pending.lock() = None;
                None
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                tracing::warn!("ContextMenu response channel disconnected");
                None
            }
        }
    }
}

/// Thin wrapper so we can pass `Arc<WebContextMenuProvider>` as a
/// `Box<dyn ContextMenuProvider>` to the host.
struct WebContextMenuProviderWrapper(Arc<WebContextMenuProvider>);

impl player_ui_traits::ContextMenuProvider for WebContextMenuProviderWrapper {
    fn show_context_menu(
        &self,
        items: &[player_ui_traits::ContextMenuItem],
        x: i32,
        y: i32,
    ) -> Option<i32> {
        self.0.show_context_menu(items, x, y)
    }
}

fn serialize_menu_items(items: &[player_ui_traits::ContextMenuItem]) -> serde_json::Value {
    serde_json::Value::Array(
        items
            .iter()
            .map(|item| {
                let type_str = match item.item_type {
                    player_ui_traits::ContextMenuItemType::Normal => "normal",
                    player_ui_traits::ContextMenuItemType::Checkbox => "checkbox",
                    player_ui_traits::ContextMenuItemType::Separator => "separator",
                    player_ui_traits::ContextMenuItemType::Submenu => "submenu",
                };
                serde_json::json!({
                    "type": type_str,
                    "name": item.name,
                    "id": item.id,
                    "enabled": item.enabled,
                    "checked": item.checked,
                    "submenu": serialize_menu_items(&item.submenu),
                })
            })
            .collect(),
    )
}

// ===========================================================================
// Audio provider - sends PCM audio to the browser via native messaging
// ===========================================================================

/// Audio provider that forwards PCM samples to the Chrome Extension's
/// Web Audio API via native messaging binary messages.
struct WebAudioProvider {
    next_stream_id: AtomicU32,
}

impl WebAudioProvider {
    fn new() -> Self {
        Self {
            next_stream_id: AtomicU32::new(1),
        }
    }
}

impl player_ui_traits::AudioProvider for WebAudioProvider {
    fn provider_name(&self) -> &'static str {
        "web-audio"
    }

    fn create_stream(&self, sample_rate: u32, sample_frame_count: u32) -> u32 {
        let id = self.next_stream_id.fetch_add(1, Ordering::Relaxed);
        tracing::info!(
            "WebAudioProvider: create_stream id={}, rate={}, frames={}",
            id, sample_rate, sample_frame_count,
        );
        let _ = protocol::send_host_message(&protocol::HostMessage::AudioInit {
            stream_id: id,
            sample_rate,
            sample_frame_count,
        });
        id
    }

    fn write_samples(&self, stream_id: u32, samples: &[u8]) {
        let _ = protocol::send_host_message(&protocol::HostMessage::AudioSamples {
            stream_id,
            samples,
        });
    }

    fn start_stream(&self, stream_id: u32) -> bool {
        tracing::debug!("WebAudioProvider: start_stream {}", stream_id);
        let _ = protocol::send_host_message(&protocol::HostMessage::AudioStart {
            stream_id,
        });
        true
    }

    fn stop_stream(&self, stream_id: u32) {
        tracing::debug!("WebAudioProvider: stop_stream {}", stream_id);
        let _ = protocol::send_host_message(&protocol::HostMessage::AudioStop {
            stream_id,
        });
    }

    fn close_stream(&self, stream_id: u32) {
        tracing::debug!("WebAudioProvider: close_stream {}", stream_id);
        let _ = protocol::send_host_message(&protocol::HostMessage::AudioClose {
            stream_id,
        });
    }
}

// ===========================================================================
// Audio input provider - captures audio from the browser's microphone
// via native messaging.
//
// The host sends AudioInputOpen/Start/Stop/Close commands to the browser.
// The browser calls navigator.mediaDevices.getUserMedia() and sends
// captured PCM data back as JSON messages of type "audioInputData".
// ===========================================================================

/// Global audio input provider for routing captured samples from the
/// stdin reader thread.
static WEB_AUDIO_INPUT: OnceLock<Arc<WebAudioInputProvider>> = OnceLock::new();

/// Audio input provider that requests microphone capture from the browser
/// and receives PCM samples over the native messaging channel.
struct WebAudioInputProvider {
    next_stream_id: AtomicU32,
    /// Per-stream ring buffers for captured samples.
    streams: parking_lot::Mutex<std::collections::HashMap<u32, WebAudioInputStreamState>>,
}

struct WebAudioInputStreamState {
    /// Ring buffer of captured bytes (mono i16 LE PCM).
    ring: RingBuf,
    #[allow(dead_code)]
    sample_rate: u32,
    #[allow(dead_code)]
    sample_frame_count: u32,
}

/// Simple ring buffer for captured audio bytes.
struct RingBuf {
    data: Vec<u8>,
    write_pos: usize,
    read_pos: usize,
    capacity: usize,
}

impl RingBuf {
    fn new(capacity: usize) -> Self {
        Self {
            data: vec![0u8; capacity],
            write_pos: 0,
            read_pos: 0,
            capacity,
        }
    }

    fn available(&self) -> usize {
        self.write_pos.wrapping_sub(self.read_pos)
    }

    fn write(&mut self, src: &[u8]) {
        for &b in src {
            let idx = self.write_pos % self.capacity;
            self.data[idx] = b;
            self.write_pos = self.write_pos.wrapping_add(1);
        }
        if self.available() > self.capacity {
            self.read_pos = self.write_pos.wrapping_sub(self.capacity);
        }
    }

    fn read(&mut self, dst: &mut [u8]) -> usize {
        let avail = self.available();
        let to_read = dst.len().min(avail);
        for i in 0..to_read {
            let idx = self.read_pos % self.capacity;
            dst[i] = self.data[idx];
            self.read_pos = self.read_pos.wrapping_add(1);
        }
        to_read
    }
}

impl WebAudioInputProvider {
    fn new() -> Self {
        Self {
            next_stream_id: AtomicU32::new(1),
            streams: parking_lot::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Called by the stdin reader thread when an `audioInputData` message
    /// arrives from the browser extension.
    fn handle_audio_data(&self, msg: &serde_json::Value) {
        let stream_id = msg["streamId"].as_u64().unwrap_or(0) as u32;
        let data_b64 = msg["data"].as_str().unwrap_or("");

        if stream_id == 0 || data_b64.is_empty() {
            return;
        }

        // Decode base64 PCM data.
        use base64::Engine;
        let pcm = match base64::engine::general_purpose::STANDARD.decode(data_b64) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(
                    "WebAudioInputProvider: bad base64 in audioInputData: {}",
                    e
                );
                return;
            }
        };

        let mut streams = self.streams.lock();
        if let Some(state) = streams.get_mut(&stream_id) {
            state.ring.write(&pcm);
        }
    }
}

impl player_ui_traits::AudioInputProvider for WebAudioInputProvider {
    fn enumerate_devices(&self) -> Vec<(String, String)> {
        vec![("browser:default".into(), "Microphone".into())]
    }

    fn open_stream(
        &self,
        _device_id: Option<&str>,
        sample_rate: u32,
        sample_frame_count: u32,
    ) -> u32 {
        let id = self.next_stream_id.fetch_add(1, Ordering::Relaxed);
        tracing::info!(
            "WebAudioInputProvider: open_stream id={}, rate={}, frames={}",
            id, sample_rate, sample_frame_count,
        );

        // Ring buffer: hold ~4 buffers worth of audio data.
        let ring_capacity = (sample_frame_count as usize) * 2 * 4;

        self.streams.lock().insert(id, WebAudioInputStreamState {
            ring: RingBuf::new(ring_capacity),
            sample_rate,
            sample_frame_count,
        });

        let _ = protocol::send_host_message(&protocol::HostMessage::AudioInputOpen {
            stream_id: id,
            sample_rate,
            sample_frame_count,
        });

        id
    }

    fn start_capture(&self, stream_id: u32) -> bool {
        tracing::debug!("WebAudioInputProvider: start_capture {}", stream_id);
        let _ = protocol::send_host_message(&protocol::HostMessage::AudioInputStart {
            stream_id,
        });
        true
    }

    fn stop_capture(&self, stream_id: u32) {
        tracing::debug!("WebAudioInputProvider: stop_capture {}", stream_id);
        let _ = protocol::send_host_message(&protocol::HostMessage::AudioInputStop {
            stream_id,
        });
    }

    fn read_samples(&self, stream_id: u32, buffer: &mut [u8]) -> usize {
        let mut streams = self.streams.lock();
        if let Some(state) = streams.get_mut(&stream_id) {
            state.ring.read(buffer)
        } else {
            0
        }
    }

    fn close_stream(&self, stream_id: u32) {
        tracing::debug!("WebAudioInputProvider: close_stream {}", stream_id);
        self.streams.lock().remove(&stream_id);
        let _ = protocol::send_host_message(&protocol::HostMessage::AudioInputClose {
            stream_id,
        });
    }
}

/// Thin wrapper so we can pass `Arc<WebAudioInputProvider>` as a
/// `Box<dyn AudioInputProvider>` to the host (delegates all calls).
struct WebAudioInputProviderWrapper(Arc<WebAudioInputProvider>);

impl player_ui_traits::AudioInputProvider for WebAudioInputProviderWrapper {
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

// ===========================================================================
// Video capture provider - captures video from the browser's webcam
// via native messaging.
//
// The host sends VideoCaptureOpen/Start/Stop/Close commands to the browser.
// The browser calls navigator.mediaDevices.getUserMedia({ video }) and sends
// captured I420 frame data back as JSON messages of type "videoCaptureData".
// ===========================================================================

/// Global video capture provider for routing captured frames from the
/// stdin reader thread.
static WEB_VIDEO_CAPTURE: OnceLock<Arc<WebVideoCaptureProvider>> = OnceLock::new();

/// Video capture provider that requests webcam capture from the browser
/// and receives I420 frame data over the native messaging channel.
struct WebVideoCaptureProvider {
    next_stream_id: AtomicU32,
    /// Per-stream latest frame (overwritten on each incoming frame).
    streams: parking_lot::Mutex<std::collections::HashMap<u32, WebVideoCaptureStreamState>>,
}

struct WebVideoCaptureStreamState {
    /// Latest captured frame (I420 data), replaced on each new frame.
    latest_frame: Option<player_ui_traits::VideoCaptureFrame>,
    #[allow(dead_code)]
    width: u32,
    #[allow(dead_code)]
    height: u32,
    #[allow(dead_code)]
    fps: u32,
}

impl WebVideoCaptureProvider {
    fn new() -> Self {
        Self {
            next_stream_id: AtomicU32::new(1),
            streams: parking_lot::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Called by the stdin reader thread when a `videoCaptureData` message
    /// arrives from the browser extension.
    fn handle_video_data(&self, msg: &serde_json::Value) {
        let stream_id = msg["streamId"].as_u64().unwrap_or(0) as u32;
        let data_b64 = msg["data"].as_str().unwrap_or("");
        let width = msg["width"].as_u64().unwrap_or(0) as u32;
        let height = msg["height"].as_u64().unwrap_or(0) as u32;

        if stream_id == 0 || data_b64.is_empty() || width == 0 || height == 0 {
            return;
        }

        use base64::Engine;
        let data = match base64::engine::general_purpose::STANDARD.decode(data_b64) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(
                    "WebVideoCaptureProvider: bad base64 in videoCaptureData: {}",
                    e
                );
                return;
            }
        };

        let frame = player_ui_traits::VideoCaptureFrame { data, width, height };

        let mut streams = self.streams.lock();
        if let Some(state) = streams.get_mut(&stream_id) {
            state.latest_frame = Some(frame);
        }
    }
}

impl player_ui_traits::VideoCaptureProvider for WebVideoCaptureProvider {
    fn enumerate_devices(&self) -> Vec<(String, String)> {
        vec![("browser:default".into(), "Camera".into())]
    }

    fn open_stream(
        &self,
        _device_id: Option<&str>,
        width: u32,
        height: u32,
        frames_per_second: u32,
    ) -> u32 {
        let id = self.next_stream_id.fetch_add(1, Ordering::Relaxed);
        tracing::info!(
            "WebVideoCaptureProvider: open_stream id={}, {}x{} @ {} fps",
            id, width, height, frames_per_second,
        );

        self.streams.lock().insert(id, WebVideoCaptureStreamState {
            latest_frame: None,
            width,
            height,
            fps: frames_per_second,
        });

        let _ = protocol::send_host_message(&protocol::HostMessage::VideoCaptureOpen {
            stream_id: id,
            width,
            height,
            frames_per_second,
        });

        id
    }

    fn start_capture(&self, stream_id: u32) -> bool {
        tracing::debug!("WebVideoCaptureProvider: start_capture {}", stream_id);
        let _ = protocol::send_host_message(&protocol::HostMessage::VideoCaptureStart {
            stream_id,
        });
        true
    }

    fn stop_capture(&self, stream_id: u32) {
        tracing::debug!("WebVideoCaptureProvider: stop_capture {}", stream_id);
        let _ = protocol::send_host_message(&protocol::HostMessage::VideoCaptureStop {
            stream_id,
        });
    }

    fn read_frame(&self, stream_id: u32) -> Option<player_ui_traits::VideoCaptureFrame> {
        let mut streams = self.streams.lock();
        if let Some(state) = streams.get_mut(&stream_id) {
            state.latest_frame.take()
        } else {
            None
        }
    }

    fn close_stream(&self, stream_id: u32) {
        tracing::debug!("WebVideoCaptureProvider: close_stream {}", stream_id);
        self.streams.lock().remove(&stream_id);
        let _ = protocol::send_host_message(&protocol::HostMessage::VideoCaptureClose {
            stream_id,
        });
    }
}

/// Thin wrapper so we can pass `Arc<WebVideoCaptureProvider>` as a
/// `Box<dyn VideoCaptureProvider>` to the host.
struct WebVideoCaptureProviderWrapper(Arc<WebVideoCaptureProvider>);

impl player_ui_traits::VideoCaptureProvider for WebVideoCaptureProviderWrapper {
    fn enumerate_devices(&self) -> Vec<(String, String)> {
        self.0.enumerate_devices()
    }
    fn open_stream(&self, device_id: Option<&str>, width: u32, height: u32, frames_per_second: u32) -> u32 {
        self.0.open_stream(device_id, width, height, frames_per_second)
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

// ===========================================================================
// Clipboard provider - routes through ScriptBridge to browser clipboard APIs
// ===========================================================================

/// Clipboard provider for the web player.
///
/// Uses the ScriptBridge (TAG_SCRIPT → page-script.js) to call browser
/// clipboard APIs.  Because the async Clipboard API requires a user gesture
/// and is not reliably available, the page-script.js implementation uses a
/// synchronous `execCommand` approach with a hidden textarea fallback, plus
/// an internal buffer for formats that the browser doesn't natively support.
struct WebClipboardProvider {
    bridge: Arc<script_bridge::ScriptBridge>,
}

impl WebClipboardProvider {
    fn new(bridge: Arc<script_bridge::ScriptBridge>) -> Self {
        Self { bridge }
    }
}

impl player_ui_traits::ClipboardProvider for WebClipboardProvider {
    fn is_format_available(&self, format: player_ui_traits::ClipboardFormat) -> bool {
        let fmt_str = match format {
            player_ui_traits::ClipboardFormat::PlainText => "plaintext",
            player_ui_traits::ClipboardFormat::Html => "html",
            player_ui_traits::ClipboardFormat::Rtf => "rtf",
        };
        let resp = self.bridge.request(serde_json::json!({
            "op": "clipboardIsAvailable",
            "format": fmt_str,
        }));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("v"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn read_text(&self, format: player_ui_traits::ClipboardFormat) -> Option<String> {
        let fmt_str = match format {
            player_ui_traits::ClipboardFormat::PlainText => "plaintext",
            player_ui_traits::ClipboardFormat::Html => "html",
            player_ui_traits::ClipboardFormat::Rtf => return None,
        };
        let resp = self.bridge.request(serde_json::json!({
            "op": "clipboardRead",
            "format": fmt_str,
        }));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("v"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned())
    }

    fn read_rtf(&self) -> Option<Vec<u8>> {
        // RTF is not supported in web browsers
        None
    }

    fn write(&self, items: &[(player_ui_traits::ClipboardFormat, Vec<u8>)]) -> bool {
        let mut plain: Option<String> = None;
        let mut html: Option<String> = None;

        for (fmt, data) in items {
            match fmt {
                player_ui_traits::ClipboardFormat::PlainText => {
                    plain = Some(String::from_utf8_lossy(data).into_owned());
                }
                player_ui_traits::ClipboardFormat::Html => {
                    html = Some(String::from_utf8_lossy(data).into_owned());
                }
                player_ui_traits::ClipboardFormat::Rtf => {} // not supported in web
            }
        }

        let resp = self.bridge.request(serde_json::json!({
            "op": "clipboardWrite",
            "plaintext": plain,
            "html": html,
        }));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("v"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}

// ===========================================================================
// Timestamp helper (no external chrono dependency)
// ===========================================================================

/// Generate a UTC timestamp string like `2026-03-06_14-30-05`.
fn chrono_timestamp() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Manual UTC breakdown (no leap-second handling needed for filenames).
    let secs_per_day: u64 = 86400;
    let days = now / secs_per_day;
    let day_secs = now % secs_per_day;
    let hours = day_secs / 3600;
    let minutes = (day_secs % 3600) / 60;
    let seconds = day_secs % 60;

    // Days since 1970-01-01 → (year, month, day) via civil_from_days.
    let (year, month, day) = civil_from_days(days as i64);

    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        year, month, day, hours, minutes, seconds
    )
}

/// Convert days since Unix epoch to (year, month, day).
/// Algorithm from Howard Hinnant's `chrono`-compatible date library.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ===========================================================================
// Settings provider - stores user-configurable settings with live updates
// ===========================================================================

/// Parse a JSON settings object (from content.js / settingsUpdate) into
/// a [`PlayerSettings`] value.  Missing keys fall back to defaults.
/// Parse a JSON settings object into a [`PlayerSettings`] value (only the
/// fields that matter to the native PPAPI host). Browser-only fields
/// (ruffleCompat, networkMode) are handled separately from this struct.
fn parse_settings(val: &serde_json::Value) -> PlayerSettings {
    let mut s = PlayerSettings::default();

    if let Some(v) = val.get("disableCrossdomainHttp").and_then(|v| v.as_bool()) {
        s.disable_crossdomain_http = v;
    }
    if let Some(v) = val.get("disableCrossdomainSockets").and_then(|v| v.as_bool()) {
        s.disable_crossdomain_sockets = v;
    }
    if let Some(v) = val.get("hardwareAcceleration").and_then(|v| v.as_bool()) {
        s.hardware_acceleration = v;
    }
    if let Some(v) = val.get("disableGeolocation").and_then(|v| v.as_bool()) {
        s.disable_geolocation = v;
    }
    if let Some(v) = val.get("spoofHardwareId").and_then(|v| v.as_bool()) {
        s.spoof_hardware_id = v;
    }
    if let Some(v) = val.get("disableMicrophone").and_then(|v| v.as_bool()) {
        s.disable_microphone = v;
    }
    if let Some(v) = val.get("disableWebcam").and_then(|v| v.as_bool()) {
        s.disable_webcam = v;
    }

    // Sandboxing: HTTP(s)
    if let Some(v) = val.get("httpSandboxMode").and_then(|v| v.as_str()) {
        s.http_sandbox_mode = player_ui_traits::SandboxMode::from_str(v);
    }
    if let Some(arr) = val.get("httpBlacklist").and_then(|v| v.as_array()) {
        s.http_blacklist = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }
    if let Some(arr) = val.get("httpWhitelist").and_then(|v| v.as_array()) {
        s.http_whitelist = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }

    // Sandboxing: TCP/UDP
    if let Some(v) = val.get("tcpUdpSandboxMode").and_then(|v| v.as_str()) {
        s.tcp_udp_sandbox_mode = player_ui_traits::SandboxMode::from_str(v);
    }
    if let Some(arr) = val.get("tcpUdpBlacklist").and_then(|v| v.as_array()) {
        s.tcp_udp_blacklist = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }
    if let Some(arr) = val.get("tcpUdpWhitelist").and_then(|v| v.as_array()) {
        s.tcp_udp_whitelist = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }

    // Sandboxing: File system
    if let Some(v) = val.get("fileWhitelistEnabled").and_then(|v| v.as_bool()) {
        s.file_whitelist_enabled = v;
    }
    if let Some(arr) = val.get("whitelistedFiles").and_then(|v| v.as_array()) {
        s.whitelisted_files = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }
    if let Some(arr) = val.get("whitelistedFolders").and_then(|v| v.as_array()) {
        s.whitelisted_folders = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }

    s
}

/// Settings provider backed by an `Arc<Mutex<PlayerSettings>>` that can
/// be updated live from the browser extension settings popup.
///
/// Also stores browser-only settings (network fallback) that don't belong
/// in the shared `PlayerSettings` struct.
pub(crate) struct WebSettingsProvider {
    inner: Mutex<PlayerSettings>,
    /// Script bridge used to push settings edits back to the browser.
    bridge: OnceLock<Arc<script_bridge::ScriptBridge>>,
    /// Whether native (cpal) audio should be used instead of browser audio.
    pub audio_backend_native: Mutex<bool>,
    /// Whether to always prefer the browser's fetch() for HTTP requests.
    /// When false, use direct reqwest HTTP instead of fetch.
    pub prefer_network_browser: Mutex<bool>,
    /// Whether to allow native HTTP fallback when fetch() hits CORS.
    pub network_fallback_native: Mutex<bool>,
}

impl WebSettingsProvider {
    fn new() -> Self {
        Self {
            inner: Mutex::new(PlayerSettings::default()),
            bridge: OnceLock::new(),
            audio_backend_native: Mutex::new(false),
            prefer_network_browser: Mutex::new(true),
            network_fallback_native: Mutex::new(false),
        }
    }

    /// Update all settings from a JSON object (from content.js / settingsUpdate).
    pub(crate) fn update_from_json(&self, val: &serde_json::Value) {
        *self.inner.lock() = parse_settings(val);

        if let Some(v) = val.get("audioBackend").and_then(|v| v.as_u64()) {
            *self.audio_backend_native.lock() = v == 1;
        }
        if let Some(v) = val.get("preferNetworkBrowser").and_then(|v| v.as_bool()) {
            *self.prefer_network_browser.lock() = v;
        }
        if let Some(v) = val.get("networkFallbackNative").and_then(|v| v.as_bool()) {
            *self.network_fallback_native.lock() = v;
        }
    }
}

impl player_ui_traits::SettingsProvider for WebSettingsProvider {
    fn get_settings(&self) -> PlayerSettings {
        self.inner.lock().clone()
    }

    fn edit_settings(&self, edits: serde_json::Value) {
        if let Some(bridge) = self.bridge.get() {
            let _ = bridge.request(serde_json::json!({
                "op": "editSettings",
                "edits": edits,
            }));
        }
    }
}

/// Thin wrapper so we can pass `Arc<WebSettingsProvider>` as a
/// `Box<dyn SettingsProvider>` to the host.
struct WebSettingsProviderWrapper(Arc<WebSettingsProvider>);

impl player_ui_traits::SettingsProvider for WebSettingsProviderWrapper {
    fn get_settings(&self) -> PlayerSettings {
        self.0.get_settings()
    }

    fn edit_settings(&self, edits: serde_json::Value) {
        self.0.edit_settings(edits);
    }
}

// ===========================================================================
// Web URL rewrite provider - regex rules from settings + optional JS callback
// ===========================================================================

/// URL rewrite provider that delegates regex-based rewrite rules to the
/// browser extension (content.js) and, optionally, invokes a JavaScript
/// callback function set on the `<embed>` / `<object>` element.
///
/// The regex rules live in the extension's settings storage and are
/// applied in cascade by content.js.  After the regex pass, the JS
/// callback (if set) gets a final chance to rewrite.
struct WebUrlRewriteProvider {
    bridge: Arc<script_bridge::ScriptBridge>,
    /// Object ID of the JS rewrite callback, if set via embed/object attribute.
    js_callback_id: Mutex<Option<u64>>,
}

impl WebUrlRewriteProvider {
    fn new(bridge: Arc<script_bridge::ScriptBridge>) -> Self {
        Self {
            bridge,
            js_callback_id: Mutex::new(None),
        }
    }

    /// Set the JS callback function object ID (resolved from embed/object
    /// attribute `data-url-rewriter`).
    fn set_js_callback(&self, object_id: u64) {
        *self.js_callback_id.lock() = Some(object_id);
    }
}

impl player_ui_traits::UrlRewriteProvider for WebUrlRewriteProvider {
    fn rewrite_url(&self, url: &str) -> Option<String> {
        let mut current = url.to_string();
        let mut changed = false;

        // Delegate regex rewrite rules to the browser extension.
        match self.bridge.request(serde_json::json!({
            "op": "rewriteUrl",
            "url": url,
        })) {
            Some(resp) => {
                if resp.get("error").is_none() {
                    if let Some(val) = resp.get("value") {
                        let js_val = script_bridge::json_to_js_value(val);
                        if let player_ui_traits::JsValue::String(new_url) = js_val {
                            if !new_url.is_empty() {
                                current = new_url;
                                changed = true;
                            }
                        }
                    }
                }
            }
            None => {}
        }

        // Apply the JS callback (if set) as a final rewrite step.
        if let Some(cb_id) = *self.js_callback_id.lock() {
            let arg = player_ui_traits::JsValue::String(current.clone());
            match self.bridge.request(serde_json::json!({
                "op": "call",
                "obj": cb_id,
                "args": [script_bridge::js_value_to_json(&arg)],
            })) {
                Some(resp) => {
                    if resp.get("error").is_none() {
                        if let Some(val) = resp.get("value") {
                            let js_val = script_bridge::json_to_js_value(val);
                            if let player_ui_traits::JsValue::String(new_url) = js_val {
                                if !new_url.is_empty() && new_url != current {
                                    current = new_url;
                                    changed = true;
                                }
                            }
                        }
                    }
                }
                None => {}
            }
        }

        if changed { Some(current) } else { None }
    }
}

/// Thin wrapper so we can pass `Arc<WebUrlRewriteProvider>` as a
/// `Box<dyn UrlRewriteProvider>` to the host.
struct WebUrlRewriteProviderWrapper(Arc<WebUrlRewriteProvider>);

impl player_ui_traits::UrlRewriteProvider for WebUrlRewriteProviderWrapper {
    fn rewrite_url(&self, url: &str) -> Option<String> {
        self.0.rewrite_url(url)
    }
}

// ===========================================================================
// Web fullscreen provider - uses the script bridge to toggle fullscreen
// ===========================================================================

struct WebFullscreenProvider {
    bridge: Arc<script_bridge::ScriptBridge>,
}

impl WebFullscreenProvider {
    fn new(bridge: Arc<script_bridge::ScriptBridge>) -> Self {
        Self { bridge }
    }
}

impl player_ui_traits::FullscreenProvider for WebFullscreenProvider {
    fn is_fullscreen(&self) -> bool {
        let resp = self.bridge.request(serde_json::json!({
            "op": "fullscreenIsActive",
        }));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("v"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn set_fullscreen(&self, fullscreen: bool) -> bool {
        let resp = self.bridge.request(serde_json::json!({
            "op": "fullscreenSet",
            "fullscreen": fullscreen,
        }));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("v"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn get_screen_size(&self) -> Option<(i32, i32)> {
        let resp = self.bridge.request(serde_json::json!({
            "op": "fullscreenGetScreenSize",
        }));
        let obj = resp.as_ref()
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("v"))?;
        let w = obj.get("w")?.as_i64()? as i32;
        let h = obj.get("h")?.as_i64()? as i32;
        Some((w, h))
    }
}

// ===========================================================================
// Web cursor lock provider - uses the script bridge for Pointer Lock API
// ===========================================================================

struct WebCursorLockProvider {
    bridge: Arc<script_bridge::ScriptBridge>,
}

impl WebCursorLockProvider {
    fn new(bridge: Arc<script_bridge::ScriptBridge>) -> Self {
        Self { bridge }
    }
}

impl player_ui_traits::CursorLockProvider for WebCursorLockProvider {
    fn lock_cursor(&self) -> bool {
        let resp = self.bridge.request(serde_json::json!({
            "op": "cursorLock",
        }));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("v"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn unlock_cursor(&self) -> bool {
        let resp = self.bridge.request(serde_json::json!({
            "op": "cursorUnlock",
        }));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("v"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn has_cursor_lock(&self) -> bool {
        let resp = self.bridge.request(serde_json::json!({
            "op": "hasCursorLock",
        }));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("v"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn can_lock_cursor(&self) -> bool {
        let resp = self.bridge.request(serde_json::json!({
            "op": "canLockCursor",
        }));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("v"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}

// ===========================================================================
// Web URL provider - browser-backed document/plugin URL lookups
// ===========================================================================

struct WebUrlProvider {
    bridge: Arc<script_bridge::ScriptBridge>,
}

impl WebUrlProvider {
    fn new(bridge: Arc<script_bridge::ScriptBridge>) -> Self {
        Self { bridge }
    }

    fn decode_string_value(resp: Option<serde_json::Value>) -> Option<String> {
        let value = resp.as_ref()?.get("value")?;
        let ty = value.get("type")?.as_str()?;
        if ty != "string" {
            return None;
        }
        value.get("v")?.as_str().map(|s| s.to_string())
    }
}

impl player_ui_traits::UrlProvider for WebUrlProvider {
    fn get_document_url(&self, _instance: i32) -> Option<String> {
        Self::decode_string_value(self.bridge.request(serde_json::json!({
            "op": "getDocumentUrl",
        })))
    }

    fn get_document_base_url(&self, _instance: i32) -> Option<String> {
        Self::decode_string_value(self.bridge.request(serde_json::json!({
            "op": "getDocumentBaseUrl",
        })))
    }

    fn get_plugin_instance_url(&self, _instance: i32) -> Option<String> {
        Self::decode_string_value(self.bridge.request(serde_json::json!({
            "op": "getPluginUrl",
        })))
    }
}

// ===========================================================================
// Print provider - delegates printing to the browser via native messaging
// ===========================================================================

struct WebPrintProvider;

impl player_ui_traits::PrintProvider for WebPrintProvider {
    fn print(&self) -> bool {
        tracing::debug!("WebPrintProvider::print - sending Print message to browser");
        protocol::send_host_message(&protocol::HostMessage::Print).is_ok()
    }
}

// ===========================================================================
// Cookie provider — uses the script bridge to get/set HTTP cookies
// ===========================================================================

struct WebCookieProvider {
    bridge: Arc<script_bridge::ScriptBridge>,
}

impl WebCookieProvider {
    fn new(bridge: Arc<script_bridge::ScriptBridge>) -> Self {
        Self { bridge }
    }
}

impl player_ui_traits::CookieProvider for WebCookieProvider {
    fn get_cookies_for_url(&self, url: &str) -> Option<String> {
        let resp = self.bridge.request(serde_json::json!({
            "op": "getCookiesForUrl",
            "url": url,
        }));
        resp.as_ref()
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("v"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    }

    fn set_cookies_from_response(&self, url: &str, set_cookie_headers: &[String]) {
        let _ = self.bridge.request(serde_json::json!({
            "op": "setCookiesFromResponse",
            "url": url,
            "cookies": set_cookie_headers,
        }));
    }
}
