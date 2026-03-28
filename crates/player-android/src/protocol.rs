//! Binary IPC protocol for communication between flash-host and the Android app.
//!
//! Message frame format:
//! ```text
//! ┌──────────┬──────────┬──────────┬────────────────┐
//! │ Length   │ Tag      │ ReqID    │ Payload        │
//! │ (4 bytes)│ (1 byte) │ (4 bytes)│ (variable)     │
//! │ LE u32   │          │ LE u32   │                │
//! └──────────┴──────────┴──────────┴────────────────┘
//! ```

use std::io::{self, Read, Write};

/// Maximum message payload size (16 MB).
const MAX_MESSAGE_SIZE: u32 = 16 * 1024 * 1024;

// =========================================================================
// Tag constants: Android → Host  (Commands / Responses)
// =========================================================================
pub mod tags {
    // Control
    pub const OPEN: u8 = 0x01;
    pub const CLOSE: u8 = 0x02;
    pub const RESIZE: u8 = 0x03;
    pub const VIEW_UPDATE: u8 = 0x04;

    // Mouse input
    pub const MOUSE_DOWN: u8 = 0x10;
    pub const MOUSE_UP: u8 = 0x11;
    pub const MOUSE_MOVE: u8 = 0x12;
    pub const MOUSE_ENTER: u8 = 0x13;
    pub const MOUSE_LEAVE: u8 = 0x14;
    pub const WHEEL: u8 = 0x15;

    // Keyboard input
    pub const KEY_DOWN: u8 = 0x20;
    pub const KEY_UP: u8 = 0x21;
    pub const KEY_CHAR: u8 = 0x22;
    pub const IME_COMPOSITION_START: u8 = 0x23;
    pub const IME_COMPOSITION_UPDATE: u8 = 0x24;
    pub const IME_COMPOSITION_END: u8 = 0x25;

    // Focus
    pub const FOCUS: u8 = 0x30;

    // Responses from Android
    pub const HTTP_RESPONSE: u8 = 0x40;
    pub const AUDIO_INPUT_DATA: u8 = 0x41;
    pub const VIDEO_CAPTURE_DATA: u8 = 0x42;
    pub const MENU_RESPONSE: u8 = 0x43;
    pub const CLIPBOARD_RESPONSE: u8 = 0x44;
    pub const COOKIE_RESPONSE: u8 = 0x45;
    pub const DIALOG_RESPONSE: u8 = 0x46;
    pub const FILE_CHOOSER_RESPONSE: u8 = 0x47;
    pub const SETTINGS_UPDATE: u8 = 0x48;
    pub const CONTEXT_MENU: u8 = 0x50;

    // Host → Android (Events / Requests)
    pub const FRAME_READY: u8 = 0x80;
    pub const FRAME_INIT: u8 = 0x81;
    pub const STATE_CHANGE: u8 = 0x82;
    pub const CURSOR_CHANGE: u8 = 0x83;
    pub const NAVIGATE: u8 = 0x84;

    pub const AUDIO_INIT: u8 = 0x90;
    pub const AUDIO_START: u8 = 0x91;
    pub const AUDIO_STOP: u8 = 0x92;
    pub const AUDIO_CLOSE: u8 = 0x93;
    pub const AUDIO_SAMPLES: u8 = 0x94;

    pub const AUDIO_INPUT_OPEN: u8 = 0xA0;
    pub const AUDIO_INPUT_START: u8 = 0xA1;
    pub const AUDIO_INPUT_STOP: u8 = 0xA2;
    pub const AUDIO_INPUT_CLOSE: u8 = 0xA3;

    pub const VIDEO_CAPTURE_OPEN: u8 = 0xB0;
    pub const VIDEO_CAPTURE_START: u8 = 0xB1;
    pub const VIDEO_CAPTURE_STOP: u8 = 0xB2;
    pub const VIDEO_CAPTURE_CLOSE: u8 = 0xB3;

    pub const HTTP_REQUEST: u8 = 0xC0;
    pub const CLIPBOARD_READ: u8 = 0xC1;
    pub const CLIPBOARD_WRITE: u8 = 0xC2;
    pub const COOKIE_GET: u8 = 0xC3;
    pub const COOKIE_SET: u8 = 0xC4;
    pub const CONTEXT_MENU_SHOW: u8 = 0xC5;
    pub const DIALOG_SHOW: u8 = 0xC6;
    pub const FILE_CHOOSER_SHOW: u8 = 0xC7;
    pub const FULLSCREEN_SET: u8 = 0xC8;
    pub const FULLSCREEN_QUERY: u8 = 0xC9;
    pub const PRINT_REQUEST: u8 = 0xCA;

    pub const VERSION: u8 = 0xD0;
}

// =========================================================================
// Raw message
// =========================================================================

/// A raw IPC message (tag + request ID + payload bytes).
#[derive(Debug, Clone)]
pub struct RawMessage {
    pub tag: u8,
    pub req_id: u32,
    pub payload: Vec<u8>,
}

impl RawMessage {
    pub fn new(tag: u8, req_id: u32, payload: Vec<u8>) -> Self {
        Self { tag, req_id, payload }
    }

    /// Create a fire-and-forget message (req_id = 0).
    pub fn fire_and_forget(tag: u8, payload: Vec<u8>) -> Self {
        Self { tag, req_id: 0, payload }
    }

    /// Read a message from a stream.
    pub fn read_from<R: Read>(reader: &mut R) -> io::Result<Self> {
        let mut header = [0u8; 9]; // 4 (length) + 1 (tag) + 4 (req_id)
        reader.read_exact(&mut header)?;

        let length = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let tag = header[4];
        let req_id = u32::from_le_bytes([header[5], header[6], header[7], header[8]]);

        // Length includes tag + req_id + payload = total - 4 (the length field itself)
        // So payload_len = length - 5 (1 tag + 4 req_id)
        if length < 5 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "message too short"));
        }
        let payload_len = length - 5;
        if payload_len > MAX_MESSAGE_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("message payload too large: {} bytes", payload_len),
            ));
        }

        let mut payload = vec![0u8; payload_len as usize];
        if !payload.is_empty() {
            reader.read_exact(&mut payload)?;
        }

        Ok(Self { tag, req_id, payload })
    }

    /// Write a message to a stream.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let length = 5 + self.payload.len() as u32; // tag(1) + req_id(4) + payload
        writer.write_all(&length.to_le_bytes())?;
        writer.write_all(&[self.tag])?;
        writer.write_all(&self.req_id.to_le_bytes())?;
        writer.write_all(&self.payload)?;
        writer.flush()?;
        Ok(())
    }
}

// =========================================================================
// Payload builder / reader helpers
// =========================================================================

/// Helper for building binary payloads.
pub struct PayloadWriter {
    buf: Vec<u8>,
}

impl PayloadWriter {
    pub fn new() -> Self {
        Self { buf: Vec::with_capacity(64) }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self { buf: Vec::with_capacity(cap) }
    }

    pub fn write_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn write_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Write a length-prefixed UTF-8 string (u32 length + bytes).
    pub fn write_string(&mut self, s: &str) {
        let bytes = s.as_bytes();
        self.write_u32(bytes.len() as u32);
        self.buf.extend_from_slice(bytes);
    }

    /// Write a length-prefixed byte blob (u32 length + bytes).
    pub fn write_bytes(&mut self, data: &[u8]) {
        self.write_u32(data.len() as u32);
        self.buf.extend_from_slice(data);
    }

    /// Write raw bytes without a length prefix.
    pub fn write_raw(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// Helper for reading binary payloads.
pub struct PayloadReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> PayloadReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn read_u8(&mut self) -> io::Result<u8> {
        if self.pos >= self.data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "payload underflow"));
        }
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    pub fn read_u16(&mut self) -> io::Result<u16> {
        if self.pos + 2 > self.data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "payload underflow"));
        }
        let v = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    pub fn read_u32(&mut self) -> io::Result<u32> {
        if self.pos + 4 > self.data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "payload underflow"));
        }
        let v = u32::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }

    pub fn read_i32(&mut self) -> io::Result<i32> {
        if self.pos + 4 > self.data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "payload underflow"));
        }
        let v = i32::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }

    pub fn read_f32(&mut self) -> io::Result<f32> {
        if self.pos + 4 > self.data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "payload underflow"));
        }
        let v = f32::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }

    /// Read a length-prefixed UTF-8 string.
    pub fn read_string(&mut self) -> io::Result<String> {
        let len = self.read_u32()? as usize;
        if self.pos + len > self.data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "payload underflow"));
        }
        let s = std::str::from_utf8(&self.data[self.pos..self.pos + len])
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .to_string();
        self.pos += len;
        Ok(s)
    }

    /// Read a length-prefixed byte blob.
    pub fn read_bytes(&mut self) -> io::Result<Vec<u8>> {
        let len = self.read_u32()? as usize;
        if self.pos + len > self.data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "payload underflow"));
        }
        let data = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(data)
    }

    /// Read remaining bytes as a slice.
    pub fn read_remaining(&mut self) -> &'a [u8] {
        let rest = &self.data[self.pos..];
        self.pos = self.data.len();
        rest
    }
}
