//! Native Messaging protocol -- length-prefixed JSON over a saved stdout fd.
//!
//! Each message is a UTF-8 JSON blob prefixed by a 4-byte little-endian
//! unsigned 32-bit integer containing the byte length of the JSON payload.
//!
//! Because stdout/stderr are redirected to `/dev/null` (to prevent the
//! Flash plugin from corrupting the native messaging channel), we write
//! to a duplicated fd that was saved before the redirect.

use std::io::{self, Read, Write};
use std::sync::OnceLock;

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, RawFd};

/// The saved stdout file, initialised by [`init_saved_stdout`] before
/// the real stdout is redirected to `/dev/null`.
static SAVED_STDOUT: OnceLock<parking_lot::Mutex<std::fs::File>> = OnceLock::new();

/// Duplicate the current stdout fd and store it for later use by
/// [`write_message`].  Must be called **before** stdout is redirected.
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

/// Read one native messaging frame from stdin.
///
/// Returns `None` on EOF (extension disconnected).
pub fn read_message() -> io::Result<Option<serde_json::Value>> {
    let stdin = io::stdin();
    let mut handle = stdin.lock();

    // Read the 4-byte length prefix.
    let mut len_buf = [0u8; 4];
    match handle.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }

    let msg_len = u32::from_ne_bytes(len_buf) as usize;

    // Guard against unreasonably large messages (max 4 GB per spec, but
    // we cap at 64 MB for safety).
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

/// Write one native messaging frame to the saved stdout fd.
pub fn write_message(value: &serde_json::Value) -> io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to serialize JSON: {}", e),
        )
    })?;

    let len = payload.len() as u32;
    let saved = SAVED_STDOUT
        .get()
        .expect("init_saved_stdout was not called");
    let mut handle = saved.lock();
    handle.write_all(&len.to_ne_bytes())?;
    handle.write_all(&payload)?;
    handle.flush()?;
    Ok(())
}
