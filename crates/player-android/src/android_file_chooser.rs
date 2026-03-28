//! Android file chooser provider — SAF file picker via IPC.

use crate::ipc_transport::IpcTransport;
use crate::protocol::{tags, PayloadReader, PayloadWriter};
use player_ui_traits::{FileChooserMode, FileChooserProvider};
use std::sync::Arc;
use std::time::Duration;

const FILE_CHOOSER_TIMEOUT: Duration = Duration::from_secs(300);

pub struct AndroidFileChooserProvider {
    ipc: Arc<IpcTransport>,
}

impl AndroidFileChooserProvider {
    pub fn new(ipc: Arc<IpcTransport>) -> Self {
        Self { ipc }
    }
}

impl FileChooserProvider for AndroidFileChooserProvider {
    fn show_file_chooser(
        &self,
        mode: FileChooserMode,
        accept_types: &str,
        suggested_name: &str,
    ) -> Vec<String> {
        let mut pw = PayloadWriter::new();
        pw.write_u8(match mode {
            FileChooserMode::Open => 0,
            FileChooserMode::OpenMultiple => 1,
            FileChooserMode::Save => 2,
        });
        pw.write_string(accept_types);
        pw.write_string(suggested_name);

        match self
            .ipc
            .request_blocking(tags::FILE_CHOOSER_SHOW, pw.finish(), FILE_CHOOSER_TIMEOUT)
        {
            Ok(response) => {
                let mut pr = PayloadReader::new(&response.payload);
                let count = pr.read_u32().unwrap_or(0);
                let mut paths = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    if let Ok(path) = pr.read_string() {
                        paths.push(path);
                    }
                }
                paths
            }
            Err(_) => Vec::new(),
        }
    }
}
