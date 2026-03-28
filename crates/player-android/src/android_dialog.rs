//! Android dialog provider — alert/confirm/prompt via IPC.

use crate::ipc_transport::IpcTransport;
use crate::protocol::{tags, PayloadReader, PayloadWriter};
use player_ui_traits::DialogProvider;
use std::sync::Arc;
use std::time::Duration;

const DIALOG_TIMEOUT: Duration = Duration::from_secs(120);

pub struct AndroidDialogProvider {
    ipc: Arc<IpcTransport>,
}

impl AndroidDialogProvider {
    pub fn new(ipc: Arc<IpcTransport>) -> Self {
        Self { ipc }
    }
}

impl DialogProvider for AndroidDialogProvider {
    fn alert(&self, message: &str) {
        let mut pw = PayloadWriter::new();
        pw.write_u8(0); // alert type
        pw.write_string(message);
        pw.write_string(""); // no default

        // Block until dismissed
        let _ = self
            .ipc
            .request_blocking(tags::DIALOG_SHOW, pw.finish(), DIALOG_TIMEOUT);
    }

    fn confirm(&self, message: &str) -> bool {
        let mut pw = PayloadWriter::new();
        pw.write_u8(1); // confirm type
        pw.write_string(message);
        pw.write_string(""); // no default

        match self
            .ipc
            .request_blocking(tags::DIALOG_SHOW, pw.finish(), DIALOG_TIMEOUT)
        {
            Ok(response) => {
                let mut pr = PayloadReader::new(&response.payload);
                pr.read_u8().unwrap_or(0) != 0
            }
            Err(_) => false,
        }
    }

    fn prompt(&self, message: &str, default: &str) -> Option<String> {
        let mut pw = PayloadWriter::new();
        pw.write_u8(2); // prompt type
        pw.write_string(message);
        pw.write_string(default);

        match self
            .ipc
            .request_blocking(tags::DIALOG_SHOW, pw.finish(), DIALOG_TIMEOUT)
        {
            Ok(response) => {
                let mut pr = PayloadReader::new(&response.payload);
                let cancelled = pr.read_u8().unwrap_or(1) == 0;
                if cancelled {
                    return None;
                }
                pr.read_string().ok()
            }
            Err(_) => None,
        }
    }
}
