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
//! | Tag   | Type   | Payload                                              |
//! |-------|--------|------------------------------------------------------|
//! | 0x01  | Frame  | 7×u32 (x y w h frameW frameH stride) + BGRA pixels   |
//! | 0x02  | State  | u8 state_code + u32 width + u32 height               |
//! | 0x03  | Cursor | i32 cursor_type                                      |
//! | 0x04  | Error  | u32 msg_len + UTF-8 bytes                            |
//!
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

/// The saved stdout file, initialised by [`init_saved_stdout`] before
/// the real stdout is redirected to `/dev/null`.
static SAVED_STDOUT: OnceLock<parking_lot::Mutex<std::fs::File>> = OnceLock::new();

/// Duplicate the current stdout fd and store it for later use.
/// Must be called **before** stdout is redirected.
#[cfg(unix)]
pub fn init_saved_stdout() {
    let raw: RawFd = unsafe { libc::dup(1) };
    assert!(raw >= 0, "failed to dup stdout");
    let file = unsafe { std::fs::File::from_raw_fd(raw) };
    SAVED_STDOUT
        .set(parking_lot::Mutex::new(file))
        .ok()
        .expect("init_saved_stdout called twice");
}

/// Duplicate the current stdout handle and store it for later use.
/// Must be called **before** stdout is redirected.
#[cfg(windows)]
pub fn init_saved_stdout() {
    use windows_sys::Win32::Foundation::{
        DuplicateHandle, DUPLICATE_SAME_ACCESS, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    unsafe {
        let stdout_handle = GetStdHandle(STD_OUTPUT_HANDLE);
        assert!(
            stdout_handle != INVALID_HANDLE_VALUE && !stdout_handle.is_null(),
            "failed to get stdout handle",
        );

        let process = GetCurrentProcess();
        let mut dup_handle: *mut std::ffi::c_void = std::ptr::null_mut();
        let ok = DuplicateHandle(
            process,
            stdout_handle,
            process,
            &mut dup_handle,
            0,            // dwDesiredAccess (ignored with DUPLICATE_SAME_ACCESS)
            0,            // bInheritHandle = FALSE
            DUPLICATE_SAME_ACCESS,
        );
        assert!(ok != 0, "failed to duplicate stdout handle");

        let file = std::fs::File::from_raw_handle(dup_handle as RawHandle);
        SAVED_STDOUT
            .set(parking_lot::Mutex::new(file))
            .ok()
            .expect("init_saved_stdout called twice");
    }
}

// -----------------------------------------------------------------------
// Read (extension → host) — still plain JSON
// -----------------------------------------------------------------------

/// Read one native messaging frame from stdin.
///
/// Returns `None` on EOF (extension disconnected).
pub fn read_message() -> io::Result<Option<serde_json::Value>> {
    let stdin = io::stdin();
    let mut handle = stdin.lock();

    let mut len_buf = [0u8; 4];
    match handle.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
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
        /// Raw BGRA_PREMUL pixels for the dirty sub-rect (row-major,
        /// `width * 4` bytes per row).
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
}

// Message type tags.
const TAG_FRAME: u8 = 0x01;
const TAG_STATE: u8 = 0x02;
const TAG_CURSOR: u8 = 0x03;
const TAG_ERROR: u8 = 0x04;

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
        }
    }
}

// -----------------------------------------------------------------------
// Chunked sending
// -----------------------------------------------------------------------

/// Serialize a [`HostMessage`] to binary, base64-encode it, chunk it to
/// stay under the 1 MB native-messaging limit, and send each chunk.
pub fn send_host_message(msg: &HostMessage<'_>) -> io::Result<()> {
    let binary = msg.to_bytes();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&binary);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);

    let total_chunks = (b64.len() + MAX_B64_PER_CHUNK - 1) / MAX_B64_PER_CHUNK;
    let total_chunks = total_chunks.max(1); // at least one chunk even if empty

    let saved = SAVED_STDOUT
        .get()
        .expect("init_saved_stdout was not called");
    let mut handle = saved.lock();

    for i in 0..total_chunks {
        let start = i * MAX_B64_PER_CHUNK;
        let end = ((i + 1) * MAX_B64_PER_CHUNK).min(b64.len());
        let chunk_data = &b64[start..end];

        // Build JSON manually — faster than pulling in serde for 4 fields.
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
