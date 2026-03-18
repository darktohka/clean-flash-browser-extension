//! Stub [`HttpRequestProvider`] implementation.
//!
//! Returns HTTP 404 for all requests.  Used as a fallback when no real
//! networking provider (reqwest, browser fetch) is available.

use player_ui_traits::{CookieProvider, HttpRequestProvider, HttpResponse};

/// Stub HTTP request provider that returns 404 for every request.
pub struct StubHttpRequestProvider;

impl StubHttpRequestProvider {
    pub fn new() -> Self {
        Self
    }
}

impl HttpRequestProvider for StubHttpRequestProvider {
    fn http_request(
        &self,
        url: &str,
        _method: &str,
        _headers: &str,
        _body: Option<&[u8]>,
        _follow_redirects: bool,
        _cookie_provider: Option<&dyn CookieProvider>,
    ) -> Result<HttpResponse, i32> {
        tracing::warn!("URL open: stub loader returning 404 for {}", url);
        Ok(HttpResponse {
            status_code: 404,
            status_line: "HTTP/1.1 404 Not Found".to_string(),
            headers: "Content-Length: 0\r\n\r\n".to_string(),
            body: Box::new(std::io::empty()),
            content_length: Some(0),
            final_url: Some(url.to_string()),
        })
    }
}
