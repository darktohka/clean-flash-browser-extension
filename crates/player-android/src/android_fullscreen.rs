//! Android fullscreen provider — queries/sets fullscreen via IPC.

use crate::ipc_transport::IpcTransport;
use crate::protocol::{tags, PayloadReader, PayloadWriter};
use player_ui_traits::FullscreenProvider;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;

const FULLSCREEN_TIMEOUT: Duration = Duration::from_secs(5);

pub struct AndroidFullscreenProvider {
    ipc: Arc<IpcTransport>,
    /// Cached screen dimensions from the last query.
    screen_size: Mutex<Option<(i32, i32)>>,
    is_fullscreen: Mutex<bool>,
}

impl AndroidFullscreenProvider {
    pub fn new(ipc: Arc<IpcTransport>) -> Self {
        Self {
            ipc,
            screen_size: Mutex::new(None),
            is_fullscreen: Mutex::new(false), // Start non-fullscreen; Flash will request if needed
        }
    }

    /// Update cached screen size (called from command dispatcher).
    pub fn update_screen_size(&self, width: i32, height: i32) {
        *self.screen_size.lock() = Some((width, height));
    }
}

impl FullscreenProvider for AndroidFullscreenProvider {
    fn is_fullscreen(&self) -> bool {
        *self.is_fullscreen.lock()
    }

    fn set_fullscreen(&self, fullscreen: bool) -> bool {
        let mut pw = PayloadWriter::new();
        pw.write_u8(if fullscreen { 1 } else { 0 });

        match self
            .ipc
            .request_blocking(tags::FULLSCREEN_SET, pw.finish(), FULLSCREEN_TIMEOUT)
        {
            Ok(response) => {
                let mut pr = PayloadReader::new(&response.payload);
                let success = pr.read_u8().unwrap_or(0) != 0;
                if success {
                    *self.is_fullscreen.lock() = fullscreen;
                }
                success
            }
            Err(_) => false,
        }
    }

    fn get_screen_size(&self) -> Option<(i32, i32)> {
        // Try cached value first
        if let Some(size) = *self.screen_size.lock() {
            return Some(size);
        }

        // Query Android
        let response = self
            .ipc
            .request_blocking(tags::FULLSCREEN_QUERY, Vec::new(), FULLSCREEN_TIMEOUT)
            .ok()?;

        let mut pr = PayloadReader::new(&response.payload);
        let width = pr.read_i32().ok()?;
        let height = pr.read_i32().ok()?;
        *self.screen_size.lock() = Some((width, height));
        Some((width, height))
    }
}
