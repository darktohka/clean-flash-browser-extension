//! Android clipboard provider — reads/writes clipboard via IPC.

use crate::ipc_transport::IpcTransport;
use crate::protocol::{tags, PayloadReader, PayloadWriter};
use player_ui_traits::{ClipboardFormat, ClipboardProvider};
use std::sync::Arc;
use std::time::Duration;

const CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(5);

pub struct AndroidClipboardProvider {
    ipc: Arc<IpcTransport>,
}

impl AndroidClipboardProvider {
    pub fn new(ipc: Arc<IpcTransport>) -> Self {
        Self { ipc }
    }
}

fn format_to_u8(format: ClipboardFormat) -> u8 {
    match format {
        ClipboardFormat::PlainText => 0,
        ClipboardFormat::Html => 1,
        ClipboardFormat::Rtf => 2,
    }
}

impl ClipboardProvider for AndroidClipboardProvider {
    fn is_format_available(&self, format: ClipboardFormat) -> bool {
        self.read_text(format).is_some()
    }

    fn read_text(&self, format: ClipboardFormat) -> Option<String> {
        let mut pw = PayloadWriter::new();
        pw.write_u8(format_to_u8(format));

        let response = self
            .ipc
            .request_blocking(tags::CLIPBOARD_READ, pw.finish(), CLIPBOARD_TIMEOUT)
            .ok()?;

        let mut pr = PayloadReader::new(&response.payload);
        let has_data = pr.read_u8().ok()?;
        if has_data == 0 {
            return None;
        }
        pr.read_string().ok()
    }

    fn read_rtf(&self) -> Option<Vec<u8>> {
        let mut pw = PayloadWriter::new();
        pw.write_u8(format_to_u8(ClipboardFormat::Rtf));

        let response = self
            .ipc
            .request_blocking(tags::CLIPBOARD_READ, pw.finish(), CLIPBOARD_TIMEOUT)
            .ok()?;

        let mut pr = PayloadReader::new(&response.payload);
        let has_data = pr.read_u8().ok()?;
        if has_data == 0 {
            return None;
        }
        pr.read_bytes().ok()
    }

    fn write(&self, items: &[(ClipboardFormat, Vec<u8>)]) -> bool {
        let mut pw = PayloadWriter::new();
        pw.write_u32(items.len() as u32);
        for (format, data) in items {
            pw.write_u8(format_to_u8(*format));
            pw.write_bytes(data);
        }

        self.ipc
            .request_blocking(tags::CLIPBOARD_WRITE, pw.finish(), CLIPBOARD_TIMEOUT)
            .is_ok()
    }
}
