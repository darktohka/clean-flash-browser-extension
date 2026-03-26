//! Native Messaging protocol -- chunked binary messages over a saved stdout fd.
//!
//! ## Host → Browser message flow
//!
//! 1. Build a binary message ([`HostMessage::to_bytes`]).
//! 2. Base64-encode the entire binary blob.
//! 3. Split the base64 string into chunks that each fit within the
//!    native-messaging 1 MB limit.
//! 4. Send each chunk as a length-prefixed JSON native-messaging frame:
//!    `{"s": <seq>, "c": <index>, "t": <total>, "d": "<base64>"}`
//!
//! ## Binary message format (little-endian)
//!
//! | Tag   | Type            | Payload                                              |
//! |-------|-----------------|------------------------------------------------------|
//! | 0x01  | Frame           | 7×u32 (x y w h frameW frameH stride) + QOI data      |
//! | 0x02  | State           | u8 state_code + u32 width + u32 height               |
//! | 0x03  | Cursor          | i32 cursor_type                                      |
//! | 0x04  | Error           | u32 msg_len + UTF-8 bytes                            |
//! | 0x05  | Navigate        | u32 url_len + URL + u32 target_len + target           |
//! | 0x10  | Script          | u32 json_len + UTF-8 JSON                            |
//! | 0x20  | AudioInit       | u32 stream_id + u32 rate + u32 frames                |
//! | 0x21  | AudioSamples    | u32 stream_id + PCM bytes                            |
//! | 0x22  | AudioStart      | u32 stream_id                                        |
//! | 0x23  | AudioStop       | u32 stream_id                                        |
//! | 0x24  | AudioClose      | u32 stream_id                                        |
//! | 0x30  | AudioInputOpen  | u32 stream_id + u32 rate + u32 frames                |
//! | 0x31  | AudioInputStart | u32 stream_id                                        |
//! | 0x32  | AudioInputStop  | u32 stream_id                                        |
//! | 0x33  | AudioInputClose | u32 stream_id                                        |
//! | 0x40  | ContextMenu     | u32 json_len + UTF-8 JSON                            |
//! | 0x50  | Print           | (no payload)                                         |
//! | 0x60  | VidCapOpen      | u32 stream_id + u32 width + u32 height + u32 fps     |
//! | 0x61  | VidCapStart     | u32 stream_id                                        |
//! | 0x62  | VidCapStop      | u32 stream_id                                        |
//! | 0x63  | VidCapClose     | u32 stream_id                                        |
//! | 0x70  | Version         | u32 version_len + UTF-8 version string               |//!
//! Because stdout/stderr are redirected to `/dev/null` (Unix) or `NUL`
//! (Windows) to prevent the Flash plugin from corrupting the native
//! messaging channel, we write to a duplicated fd/handle that was saved
//! before the redirect.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

use base64::Engine;

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, RawFd};

#[cfg(windows)]
use std::os::windows::io::{FromRawHandle, RawHandle};

/// Maximum base64 characters per chunk.  The JSON envelope around each
/// chunk is at most ~64 bytes (`{"s":4294967295,"c":999,"t":999,"d":""}`),
/// so 1 000 000 + 64 ≈ 1 000 064 which is well under the 1 048 576 byte
/// native-messaging limit.  1 000 000 is also a multiple of 4 (required
/// for clean base64 slicing).
const MAX_B64_PER_CHUNK: usize = 1_000_000;

/// Monotonically increasing message sequence number.
static SEQ: AtomicU32 = AtomicU32::new(0);

// -----------------------------------------------------------------------
// Saved stdout
// -----------------------------------------------------------------------

/// The saved stdout file, initialised by [`init_saved_handles`] before
/// the real stdout is redirected to `/dev/null`.
static SAVED_STDOUT: OnceLock<parking_lot::Mutex<std::fs::File>> = OnceLock::new();

/// Saved stdin file, initialised by [`init_saved_handles`] before
/// the native plugin is loaded (which could interfere with standard
/// handles via SetStdHandle or CRT `_dup2`).
static SAVED_STDIN: OnceLock<parking_lot::Mutex<std::fs::File>> = OnceLock::new();

/// Duplicate the current stdout **and stdin** fds and store them.
/// Must be called **before** stdout is redirected.
#[cfg(unix)]
pub fn init_saved_handles() {
    let raw_out: RawFd = unsafe { libc::dup(1) };
    assert!(raw_out >= 0, "failed to dup stdout");
    let file_out = unsafe { std::fs::File::from_raw_fd(raw_out) };
    SAVED_STDOUT
        .set(parking_lot::Mutex::new(file_out))
        .ok()
        .expect("init_saved_handles called twice");

    let raw_in: RawFd = unsafe { libc::dup(0) };
    assert!(raw_in >= 0, "failed to dup stdin");
    let file_in = unsafe { std::fs::File::from_raw_fd(raw_in) };
    SAVED_STDIN
        .set(parking_lot::Mutex::new(file_in))
        .ok()
        .expect("init_saved_handles called twice (stdin)");
}

/// Duplicate the current stdout **and stdin** handles and store them.
/// Must be called **before** stdout is redirected.
#[cfg(windows)]
pub fn init_saved_handles() {
    use windows_sys::Win32::Foundation::{
        DuplicateHandle, DUPLICATE_SAME_ACCESS, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    unsafe {
        let process = GetCurrentProcess();

        // ---- stdout ----
        let stdout_handle = GetStdHandle(STD_OUTPUT_HANDLE);
        assert!(
            stdout_handle != INVALID_HANDLE_VALUE && !stdout_handle.is_null(),
            "failed to get stdout handle",
        );
        let mut dup_out: *mut std::ffi::c_void = std::ptr::null_mut();
        let ok = DuplicateHandle(
            process, stdout_handle, process, &mut dup_out,
            0, 0, DUPLICATE_SAME_ACCESS,
        );
        assert!(ok != 0, "failed to duplicate stdout handle");
        let file_out = std::fs::File::from_raw_handle(dup_out as RawHandle);
        SAVED_STDOUT
            .set(parking_lot::Mutex::new(file_out))
            .ok()
            .expect("init_saved_handles called twice");

        // ---- stdin ----
        let stdin_handle = GetStdHandle(STD_INPUT_HANDLE);
        assert!(
            stdin_handle != INVALID_HANDLE_VALUE && !stdin_handle.is_null(),
            "failed to get stdin handle",
        );
        let mut dup_in: *mut std::ffi::c_void = std::ptr::null_mut();
        let ok = DuplicateHandle(
            process, stdin_handle, process, &mut dup_in,
            0, 0, DUPLICATE_SAME_ACCESS,
        );
        assert!(ok != 0, "failed to duplicate stdin handle");
        let file_in = std::fs::File::from_raw_handle(dup_in as RawHandle);
        SAVED_STDIN
            .set(parking_lot::Mutex::new(file_in))
            .ok()
            .expect("init_saved_handles called twice (stdin)");
    }
}

// -----------------------------------------------------------------------
// Read (extension → host) - still plain JSON
// -----------------------------------------------------------------------

/// Read one native messaging frame from the saved stdin handle.
///
/// Uses the handle duplicated at startup by [`init_saved_handles`] so
/// that the read is immune to later `SetStdHandle` / CRT `_dup2` calls
/// by the Flash plugin or other native code.
///
/// Returns `None` on EOF (extension disconnected).
pub fn read_message() -> io::Result<Option<serde_json::Value>> {
    let saved = SAVED_STDIN
        .get()
        .expect("init_saved_handles was not called");
    let mut handle = saved.lock();

    let mut len_buf = [0u8; 4];
    match handle.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
            tracing::warn!("stdin read_exact returned UnexpectedEof (pipe closed by browser)");
            return Ok(None);
        }
        Err(e) => {
            tracing::warn!(
                "stdin read_exact error: kind={:?} os_error={:?} - {}",
                e.kind(), e.raw_os_error(), e,
            );
            return Err(e);
        }
    }

    let msg_len = u32::from_ne_bytes(len_buf) as usize;
    if msg_len > 64 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message too large: {} bytes", msg_len),
        ));
    }

    let mut buf = vec![0u8; msg_len];
    handle.read_exact(&mut buf)?;

    let value: serde_json::Value = serde_json::from_slice(&buf).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("invalid JSON: {}", e))
    })?;

    Ok(Some(value))
}

// -----------------------------------------------------------------------
// Host messages (host → extension)
// -----------------------------------------------------------------------

/// A message from the host to the browser extension.
pub enum HostMessage<'a> {
    /// Dirty frame region.
    Frame {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        frame_width: u32,
        frame_height: u32,
        stride: u32,
        /// QOI-encoded RGBA pixels for the dirty sub-rect.
        /// Produced by `qoi_encode::qoi_encode_bgra` (BGRA→RGBA on the fly).
        pixels: &'a [u8],
    },
    /// Player state change.
    State {
        /// 0 = idle, 1 = loading, 2 = running, 3 = error.
        code: u8,
        width: u32,
        height: u32,
    },
    /// Cursor type changed (`PP_CursorType_Dev`).
    Cursor(i32),
    /// Error message.
    Error(&'a str),
    /// JavaScript scripting request (host → browser → content script).
    /// The payload is a JSON string that the content script interprets.
    ScriptRequest(&'a str),
    /// Navigation request - Flash wants to open a URL.
    Navigate {
        url: &'a str,
        target: &'a str,
    },
    /// Audio: initialise a new stream.
    AudioInit {
        stream_id: u32,
        sample_rate: u32,
        sample_frame_count: u32,
    },
    /// Audio: PCM sample data for a stream.
    AudioSamples {
        stream_id: u32,
        samples: &'a [u8],
    },
    /// Audio: start playback on a stream.
    AudioStart {
        stream_id: u32,
    },
    /// Audio: stop (pause) playback on a stream.
    AudioStop {
        stream_id: u32,
    },
    /// Audio: close and release a stream.
    AudioClose {
        stream_id: u32,
    },
    /// Audio input: open a capture stream.
    AudioInputOpen {
        stream_id: u32,
        sample_rate: u32,
        sample_frame_count: u32,
    },
    /// Audio input: start capturing.
    AudioInputStart {
        stream_id: u32,
    },
    /// Audio input: stop capturing.
    AudioInputStop {
        stream_id: u32,
    },
    /// Audio input: close and release a capture stream.
    AudioInputClose {
        stream_id: u32,
    },
    /// Context menu: send menu items to the UI for display.
    /// Payload is a JSON string describing the menu tree.
    ContextMenu(&'a str),
    /// Print: request the browser to print the current content.
    Print,
    /// Video capture: open a capture stream.
    VideoCaptureOpen {
        stream_id: u32,
        width: u32,
        height: u32,
        frames_per_second: u32,
    },
    /// Video capture: start capturing.
    VideoCaptureStart {
        stream_id: u32,
    },
    /// Video capture: stop capturing.
    VideoCaptureStop {
        stream_id: u32,
    },
    /// Video capture: close and release a capture stream.
    VideoCaptureClose {
        stream_id: u32,
    },
    /// Protocol version handshake.
    Version(&'a str),
}

// Message type tags.
const TAG_FRAME: u8 = 0x01;
const TAG_STATE: u8 = 0x02;
const TAG_CURSOR: u8 = 0x03;
const TAG_ERROR: u8 = 0x04;
const TAG_SCRIPT: u8 = 0x10;
const TAG_NAVIGATE: u8 = 0x05;
const TAG_AUDIO_INIT: u8 = 0x20;
const TAG_AUDIO_SAMPLES: u8 = 0x21;
const TAG_AUDIO_START: u8 = 0x22;
const TAG_AUDIO_STOP: u8 = 0x23;
const TAG_AUDIO_CLOSE: u8 = 0x24;
const TAG_AUDIO_INPUT_OPEN: u8 = 0x30;
const TAG_AUDIO_INPUT_START: u8 = 0x31;
const TAG_AUDIO_INPUT_STOP: u8 = 0x32;
const TAG_AUDIO_INPUT_CLOSE: u8 = 0x33;
const TAG_CONTEXT_MENU: u8 = 0x40;
const TAG_PRINT: u8 = 0x50;
const TAG_VIDEO_CAPTURE_OPEN: u8 = 0x60;
const TAG_VIDEO_CAPTURE_START: u8 = 0x61;
const TAG_VIDEO_CAPTURE_STOP: u8 = 0x62;
const TAG_VIDEO_CAPTURE_CLOSE: u8 = 0x63;
const TAG_VERSION: u8 = 0x70;

impl<'a> HostMessage<'a> {
    /// Serialize to a compact binary representation (little-endian).
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            HostMessage::Frame {
                x, y, width, height, frame_width, frame_height, stride, pixels,
            } => {
                let mut buf = Vec::with_capacity(1 + 7 * 4 + pixels.len());
                buf.push(TAG_FRAME);
                buf.extend_from_slice(&x.to_le_bytes());
                buf.extend_from_slice(&y.to_le_bytes());
                buf.extend_from_slice(&width.to_le_bytes());
                buf.extend_from_slice(&height.to_le_bytes());
                buf.extend_from_slice(&frame_width.to_le_bytes());
                buf.extend_from_slice(&frame_height.to_le_bytes());
                buf.extend_from_slice(&stride.to_le_bytes());
                buf.extend_from_slice(pixels);
                buf
            }
            HostMessage::State { code, width, height } => {
                let mut buf = Vec::with_capacity(1 + 1 + 4 + 4);
                buf.push(TAG_STATE);
                buf.push(*code);
                buf.extend_from_slice(&width.to_le_bytes());
                buf.extend_from_slice(&height.to_le_bytes());
                buf
            }
            HostMessage::Cursor(cursor) => {
                let mut buf = Vec::with_capacity(1 + 4);
                buf.push(TAG_CURSOR);
                buf.extend_from_slice(&cursor.to_le_bytes());
                buf
            }
            HostMessage::Error(msg) => {
                let bytes = msg.as_bytes();
                let mut buf = Vec::with_capacity(1 + 4 + bytes.len());
                buf.push(TAG_ERROR);
                buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(bytes);
                buf
            }
            HostMessage::ScriptRequest(json) => {
                let bytes = json.as_bytes();
                let mut buf = Vec::with_capacity(1 + 4 + bytes.len());
                buf.push(TAG_SCRIPT);
                buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(bytes);
                buf
            }
            HostMessage::Navigate { url, target } => {
                let url_bytes = url.as_bytes();
                let target_bytes = target.as_bytes();
                let mut buf = Vec::with_capacity(1 + 4 + url_bytes.len() + 4 + target_bytes.len());
                buf.push(TAG_NAVIGATE);
                buf.extend_from_slice(&(url_bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(url_bytes);
                buf.extend_from_slice(&(target_bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(target_bytes);
                buf
            }
            HostMessage::AudioInit { stream_id, sample_rate, sample_frame_count } => {
                let mut buf = Vec::with_capacity(1 + 12);
                buf.push(TAG_AUDIO_INIT);
                buf.extend_from_slice(&stream_id.to_le_bytes());
                buf.extend_from_slice(&sample_rate.to_le_bytes());
                buf.extend_from_slice(&sample_frame_count.to_le_bytes());
                buf
            }
            HostMessage::AudioSamples { stream_id, samples } => {
                let mut buf = Vec::with_capacity(1 + 4 + samples.len());
                buf.push(TAG_AUDIO_SAMPLES);
                buf.extend_from_slice(&stream_id.to_le_bytes());
                buf.extend_from_slice(samples);
                buf
            }
            HostMessage::AudioStart { stream_id } => {
                let mut buf = Vec::with_capacity(5);
                buf.push(TAG_AUDIO_START);
                buf.extend_from_slice(&stream_id.to_le_bytes());
                buf
            }
            HostMessage::AudioStop { stream_id } => {
                let mut buf = Vec::with_capacity(5);
                buf.push(TAG_AUDIO_STOP);
                buf.extend_from_slice(&stream_id.to_le_bytes());
                buf
            }
            HostMessage::AudioClose { stream_id } => {
                let mut buf = Vec::with_capacity(5);
                buf.push(TAG_AUDIO_CLOSE);
                buf.extend_from_slice(&stream_id.to_le_bytes());
                buf
            }
            HostMessage::AudioInputOpen { stream_id, sample_rate, sample_frame_count } => {
                let mut buf = Vec::with_capacity(1 + 12);
                buf.push(TAG_AUDIO_INPUT_OPEN);
                buf.extend_from_slice(&stream_id.to_le_bytes());
                buf.extend_from_slice(&sample_rate.to_le_bytes());
                buf.extend_from_slice(&sample_frame_count.to_le_bytes());
                buf
            }
            HostMessage::AudioInputStart { stream_id } => {
                let mut buf = Vec::with_capacity(5);
                buf.push(TAG_AUDIO_INPUT_START);
                buf.extend_from_slice(&stream_id.to_le_bytes());
                buf
            }
            HostMessage::AudioInputStop { stream_id } => {
                let mut buf = Vec::with_capacity(5);
                buf.push(TAG_AUDIO_INPUT_STOP);
                buf.extend_from_slice(&stream_id.to_le_bytes());
                buf
            }
            HostMessage::AudioInputClose { stream_id } => {
                let mut buf = Vec::with_capacity(5);
                buf.push(TAG_AUDIO_INPUT_CLOSE);
                buf.extend_from_slice(&stream_id.to_le_bytes());
                buf
            }
            HostMessage::ContextMenu(json) => {
                let bytes = json.as_bytes();
                let mut buf = Vec::with_capacity(1 + 4 + bytes.len());
                buf.push(TAG_CONTEXT_MENU);
                buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(bytes);
                buf
            }
            HostMessage::Print => {
                vec![TAG_PRINT]
            }
            HostMessage::VideoCaptureOpen { stream_id, width, height, frames_per_second } => {
                let mut buf = Vec::with_capacity(1 + 16);
                buf.push(TAG_VIDEO_CAPTURE_OPEN);
                buf.extend_from_slice(&stream_id.to_le_bytes());
                buf.extend_from_slice(&width.to_le_bytes());
                buf.extend_from_slice(&height.to_le_bytes());
                buf.extend_from_slice(&frames_per_second.to_le_bytes());
                buf
            }
            HostMessage::VideoCaptureStart { stream_id } => {
                let mut buf = Vec::with_capacity(5);
                buf.push(TAG_VIDEO_CAPTURE_START);
                buf.extend_from_slice(&stream_id.to_le_bytes());
                buf
            }
            HostMessage::VideoCaptureStop { stream_id } => {
                let mut buf = Vec::with_capacity(5);
                buf.push(TAG_VIDEO_CAPTURE_STOP);
                buf.extend_from_slice(&stream_id.to_le_bytes());
                buf
            }
            HostMessage::VideoCaptureClose { stream_id } => {
                let mut buf = Vec::with_capacity(5);
                buf.push(TAG_VIDEO_CAPTURE_CLOSE);
                buf.extend_from_slice(&stream_id.to_le_bytes());
                buf
            }
            HostMessage::Version(version) => {
                let bytes = version.as_bytes();
                let mut buf = Vec::with_capacity(1 + 4 + bytes.len());
                buf.push(TAG_VERSION);
                buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(bytes);
                buf
            }
        }
    }
}

// -----------------------------------------------------------------------
// Chunked sending
// -----------------------------------------------------------------------

/// Serialize a [`HostMessage`] to binary, LZ4-compress it, base64-encode,
/// chunk it to stay under the 1 MB native-messaging limit, and send each chunk.
pub fn send_host_message(msg: &HostMessage<'_>) -> io::Result<()> {
    let binary = msg.to_bytes();
    let compressed = lz4_flex::compress_prepend_size(&binary);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&compressed);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);

    let total_chunks = (b64.len() + MAX_B64_PER_CHUNK - 1) / MAX_B64_PER_CHUNK;
    let total_chunks = total_chunks.max(1); // at least one chunk even if empty

    let saved = SAVED_STDOUT
        .get()
        .expect("init_saved_handles was not called");
    let mut handle = saved.lock();

    for i in 0..total_chunks {
        let start = i * MAX_B64_PER_CHUNK;
        let end = ((i + 1) * MAX_B64_PER_CHUNK).min(b64.len());
        let chunk_data = &b64[start..end];

        // Build JSON manually - faster than pulling in serde for 4 fields.
        let json = format!(
            r#"{{"s":{},"c":{},"t":{},"d":"{}"}}"#,
            seq, i, total_chunks, chunk_data,
        );
        let payload = json.as_bytes();
        let len = payload.len() as u32;

        handle.write_all(&len.to_ne_bytes())?;
        handle.write_all(payload)?;
    }

    handle.flush()?;
    Ok(())
}
