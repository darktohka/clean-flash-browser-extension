//! Android cookie provider — persists cookies via IPC to Android's SQLite store.

use crate::ipc_transport::IpcTransport;
use crate::protocol::{tags, PayloadReader, PayloadWriter};
use player_ui_traits::CookieProvider;
use std::sync::Arc;
use std::time::Duration;

const COOKIE_TIMEOUT: Duration = Duration::from_secs(5);

pub struct AndroidCookieProvider {
    ipc: Arc<IpcTransport>,
}

impl AndroidCookieProvider {
    pub fn new(ipc: Arc<IpcTransport>) -> Self {
        Self { ipc }
    }
}

impl CookieProvider for AndroidCookieProvider {
    fn get_cookies_for_url(&self, url: &str) -> Option<String> {
        let mut pw = PayloadWriter::new();
        pw.write_string(url);

        let response = self
            .ipc
            .request_blocking(tags::COOKIE_GET, pw.finish(), COOKIE_TIMEOUT)
            .ok()?;

        let mut pr = PayloadReader::new(&response.payload);
        let has_cookies = pr.read_u8().ok()?;
        if has_cookies == 0 {
            return None;
        }
        pr.read_string().ok()
    }

    fn set_cookies_from_response(&self, url: &str, set_cookie_headers: &[String]) {
        let mut pw = PayloadWriter::new();
        pw.write_string(url);
        pw.write_u32(set_cookie_headers.len() as u32);
        for header in set_cookie_headers {
            pw.write_string(header);
        }

        let _ = self.ipc.send(tags::COOKIE_SET, pw.finish());
    }
}
