//! Flash Player Native Messaging Host
//!
//! This binary implements the Chrome/Firefox Native Messaging protocol.
//! It receives JSON commands over stdin (input events, open requests)
//! and sends frame updates over stdout (dirty regions as base64 BGRA data).
//!
//! # Protocol (extension → host)
//!
//! ```json
//! {"type": "open", "url": "https://example.com/game.swf"}
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

mod protocol;

use parking_lot::Mutex;
use player_core::FlashPlayer;
use ppapi_sys::*;
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

fn main() {
    // Save the real stdout fd before we redirect it to /dev/null.
    protocol::init_saved_stdout();

    // Set up logging: write to log file ONLY.
    // stdout is reserved for native messaging; stderr is silenced so that
    // libpepflashplayer.so cannot corrupt the native messaging channel.
    let filter = 
                EnvFilter::new("trace");
                //EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Create a new log file for each execution, named with a timestamp.
    let log_dir = std::env::var("FLASH_PLAYER_LOG_DIR")
        .unwrap_or_else(|_| "/home/user/flash-player-host".into());
    let _ = std::fs::create_dir_all(&log_dir);
    let timestamp = chrono_timestamp();
    let log_filename = format!("flash-player-host-{}.log", timestamp);
    let log_path = std::path::Path::new(&log_dir).join(&log_filename);
    let log_file = std::fs::File::create(&log_path).expect("failed to create log file");

    // Write startup header with current date to the log file.
    use std::io::Write;
    let mut header_file = log_file.try_clone().expect("failed to clone log file handle");
    let _ = writeln!(header_file, "=== Flash Player Native Messaging Host ===");
    let _ = writeln!(header_file, "=== Started: {} ===", timestamp);
    let _ = writeln!(header_file);

    // Build subscriber: log file only (no stderr, no stdout).
    let (file_writer, _guard) = tracing_appender::non_blocking(log_file);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false);

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .init();

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
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Console::{
            SetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
        };

        // Open the NUL device (Windows equivalent of /dev/null).
        let nul = std::fs::OpenOptions::new()
            .write(true)
            .open("NUL")
            .expect("failed to open NUL device");
        let nul_handle = nul.as_raw_handle();

        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, nul_handle); // stdout → NUL
            SetStdHandle(STD_ERROR_HANDLE, nul_handle);  // stderr → NUL
        }

        // Leak so the handle stays valid for the entire process lifetime.
        std::mem::forget(nul);
    }

    tracing::info!("Flash Player Native Messaging Host starting");
    tracing::info!("Log file: {}", log_path.display());

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

    // Plugin path from environment.
    #[cfg(windows)]
    let plugin_path =
        std::env::var("FLASH_PLUGIN_PATH").unwrap_or_else(|_| "pepflashplayer.dll".into());
    #[cfg(not(windows))]
    let plugin_path =
        std::env::var("FLASH_PLUGIN_PATH").unwrap_or_else(|_| "libpepflashplayer.so".into());

    if let Ok(resolved) = std::fs::canonicalize(&plugin_path) {
        player.set_plugin_path(resolved.to_string_lossy().as_ref());
    } else {
        player.set_plugin_path(&plugin_path);
    }

    if let Err(e) = player.init_host() {
        let _ = protocol::send_host_message(&protocol::HostMessage::Error(
            &format!("Failed to initialize host: {}", e),
        ));
        std::process::exit(1);
    }

    tracing::info!("Host initialized successfully, entering main loop");

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
                if !handle_command(&mut player, &cmd) {
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

// ===========================================================================
// Threaded stdin reader — makes stdin non-blocking for the main loop
// ===========================================================================

use std::sync::mpsc;

/// Lazily-initialized channel that receives messages from a background reader.
fn try_read_command() -> Option<serde_json::Value> {
    use std::sync::OnceLock;

    static RX: OnceLock<Mutex<mpsc::Receiver<Option<serde_json::Value>>>> = OnceLock::new();

    let rx = RX.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("stdin-reader".into())
            .spawn(move || {
                loop {
                    match protocol::read_message() {
                        Ok(Some(msg)) => {
                            if tx.send(Some(msg)).is_err() {
                                break;
                            }
                        }
                        Ok(None) => {
                            // EOF — extension closed.
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
            // EOF sentinel — signal shutdown.
            Some(json!({"type": "eof"}))
        }
        Err(mpsc::TryRecvError::Empty) => None,
        Err(mpsc::TryRecvError::Disconnected) => Some(json!({"type": "eof"})),
    }
}

// ===========================================================================
// Command handling
// ===========================================================================

/// Handle one incoming JSON command. Returns `false` to signal shutdown.
fn handle_command(player: &mut FlashPlayer, cmd: &serde_json::Value) -> bool {
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
            tracing::info!("Opening SWF: {}", url);
            match player.open_swf(url) {
                Ok(()) => {
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
                player.notify_view_change(w, h);
            }
        }

        "mousedown" | "mouseup" | "mousemove" | "mouseenter" | "mouseleave" => {
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

        "keydown" | "keyup" | "char" => {
            let key_code = cmd["keyCode"].as_u64().unwrap_or(0) as u32;
            let modifiers = cmd["modifiers"].as_u64().unwrap_or(0) as u32;
            let text = cmd["text"].as_str().unwrap_or("");
            let code = cmd["code"].as_str().unwrap_or("");

            let event_type = match msg_type {
                "keydown" => PP_INPUTEVENT_TYPE_KEYDOWN,
                "keyup" => PP_INPUTEVENT_TYPE_KEYUP,
                "char" => PP_INPUTEVENT_TYPE_CHAR,
                _ => unreachable!(),
            };

            player.send_keyboard_event(event_type, key_code, text, code, modifiers);
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

        "close" | "eof" => {
            tracing::info!("Received {} — shutting down", msg_type);
            return false;
        }

        other => {
            tracing::warn!("Unknown message type: {}", other);
        }
    }

    true
}

// ===========================================================================
// Frame sending — only dirty region
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

    let _ = protocol::send_host_message(&protocol::HostMessage::Frame {
        x: dx,
        y: dy,
        width: dw,
        height: dh,
        frame_width: frame_w,
        frame_height: frame_h,
        stride,
        pixels: &region,
    });
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
