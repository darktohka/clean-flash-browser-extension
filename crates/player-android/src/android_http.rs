//! Android HTTP request provider — delegates HTTP to Android via IPC.

use crate::ipc_transport::IpcTransport;
use crate::protocol::{tags, PayloadReader, PayloadWriter};
use player_ui_traits::{CookieProvider, HttpRequestProvider, HttpResponse};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

/// Default timeout for HTTP requests (5 minutes).
const HTTP_TIMEOUT: Duration = Duration::from_secs(300);

pub struct AndroidHttpProvider {
    ipc: Arc<IpcTransport>,
}

impl AndroidHttpProvider {
    pub fn new(ipc: Arc<IpcTransport>) -> Self {
        Self { ipc }
    }
}

impl HttpRequestProvider for AndroidHttpProvider {
    fn http_request(
        &self,
        url: &str,
        method: &str,
        headers: &str,
        body: Option<&[u8]>,
        follow_redirects: bool,
        cookie_provider: Option<&dyn CookieProvider>,
    ) -> Result<HttpResponse, i32> {
        // Inject cookies from the cookie provider
        let mut full_headers = headers.to_string();
        if let Some(cp) = cookie_provider {
            if let Some(cookies) = cp.get_cookies_for_url(url) {
                if !cookies.is_empty() {
                    full_headers.push_str(&format!("Cookie: {}\r\n", cookies));
                }
            }
        }

        let mut pw = PayloadWriter::new();
        pw.write_string(method);
        pw.write_string(url);
        pw.write_string(&full_headers);
        pw.write_u8(if follow_redirects { 1 } else { 0 });
        match body {
            Some(b) => {
                pw.write_u8(1); // has body
                pw.write_bytes(b);
            }
            None => {
                pw.write_u8(0); // no body
            }
        }

        let response = self
            .ipc
            .request_blocking(tags::HTTP_REQUEST, pw.finish(), HTTP_TIMEOUT)
            .map_err(|_| -1i32)?;

        let mut pr = PayloadReader::new(&response.payload);
        let status_code = pr.read_u16().map_err(|_| -1i32)?;
        let status_line = pr.read_string().map_err(|_| -1i32)?;
        let resp_headers = pr.read_string().map_err(|_| -1i32)?;
        let has_final_url = pr.read_u8().map_err(|_| -1i32)?;
        let final_url = if has_final_url == 1 {
            Some(pr.read_string().map_err(|_| -1i32)?)
        } else {
            None
        };
        let body_data = pr.read_bytes().map_err(|_| -1i32)?;
        let content_length = if body_data.is_empty() {
            None
        } else {
            Some(body_data.len() as i64)
        };

        // Store response cookies
        if let Some(cp) = cookie_provider {
            let set_cookies: Vec<String> = resp_headers
                .lines()
                .filter(|line| line.to_ascii_lowercase().starts_with("set-cookie:"))
                .map(|line| line[11..].trim().to_string())
                .collect();
            if !set_cookies.is_empty() {
                cp.set_cookies_from_response(url, &set_cookies);
            }
        }

        Ok(HttpResponse {
            status_code,
            status_line,
            headers: resp_headers,
            body: Box::new(Cursor::new(body_data)),
            content_length,
            final_url,
        })
    }
}
