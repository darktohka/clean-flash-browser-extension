//! Fetch-based [`HttpRequestProvider`] for the web player.
//!
//! Delegates HTTP requests to the browser extension via the
//! [`ScriptBridge`], which forwards them to the browser's `fetch()` API.
//! The response body is returned as a single blob (not streamed) because
//! the native-messaging channel is message-oriented, not streaming.

use std::sync::Arc;

use base64::Engine;
use player_ui_traits::{CookieProvider, HttpRequestProvider, HttpResponse};
use ppapi_sys::*;

use crate::script_bridge::ScriptBridge;

/// HTTP request provider that uses the browser's `fetch()` API via the
/// native-messaging script bridge.
///
/// When a CORS error is detected and a fallback provider is set (typically
/// reqwest-based), the request is retried through the fallback.
pub struct FetchHttpRequestProvider {
    bridge: Arc<ScriptBridge>,
    fallback: Option<Arc<dyn HttpRequestProvider>>,
}

impl FetchHttpRequestProvider {
    pub fn new(bridge: Arc<ScriptBridge>) -> Self {
        Self { bridge, fallback: None }
    }

    /// Set a fallback provider to use when `fetch()` encounters a CORS error.
    pub fn with_fallback(mut self, fallback: Arc<dyn HttpRequestProvider>) -> Self {
        self.fallback = Some(fallback);
        self
    }
}

impl HttpRequestProvider for FetchHttpRequestProvider {
    fn http_request(
        &self,
        url: &str,
        method: &str,
        headers: &str,
        body: Option<&[u8]>,
        follow_redirects: bool,
        cookie_provider: Option<&dyn CookieProvider>,
    ) -> Result<HttpResponse, i32> {
        // Build the header map as a JSON object.
        let mut header_map = serde_json::Map::new();
        let mut has_cookie_header = false;
        for line in headers.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim();
                let value = value.trim();
                if key.eq_ignore_ascii_case("cookie") {
                    has_cookie_header = true;
                }
                header_map.insert(key.to_string(), serde_json::Value::String(value.to_string()));
            }
        }

        // Inject cookies from the cookie provider if no explicit Cookie
        // header was supplied by the plugin.
        if !has_cookie_header {
            if let Some(provider) = cookie_provider {
                if let Some(cookie_val) = provider.get_cookies_for_url(url) {
                    if !cookie_val.is_empty() {
                        tracing::debug!("fetch: injecting cookies for {}", url);
                        header_map.insert("Cookie".to_string(), serde_json::Value::String(cookie_val));
                    }
                }
            }
        }

        // Base64-encode the body if present.
        let body_b64 = body
            .filter(|b| !b.is_empty())
            .map(|b| base64::engine::general_purpose::STANDARD.encode(b));

        let payload = serde_json::json!({
            "op": "httpFetch",
            "url": url,
            "method": method,
            "headers": header_map,
            "body": body_b64,
            "followRedirects": follow_redirects,
        });

        let resp = self.bridge.request(payload);
        let resp = match resp {
            Some(r) => r,
            None => {
                tracing::warn!("fetch: no response from browser for {}", url);
                return Err(PP_ERROR_FAILED);
            }
        };

        // Check for error response.
        if let Some(err) = resp.get("error").and_then(|v| v.as_str()) {
            tracing::warn!("fetch: browser returned error for {}: {}", url, err);

            // Detect CORS errors and fall back to the reqwest provider,
            // but only if the network settings allow native fallback.
            let err_lower = err.to_ascii_lowercase();
            if err_lower.starts_with("cors:") || err_lower.contains("cors") {
                if let Some(ref fallback) = self.fallback {
                    let allow_fallback = crate::WEB_SETTINGS
                        .get()
                        .map(|ws| *ws.network_fallback_native.lock())
                        .unwrap_or(false);

                    if allow_fallback {
                        tracing::info!(
                            "fetch: CORS error for {}, falling back to direct HTTP",
                            url
                        );
                        return fallback.http_request(
                            url,
                            method,
                            headers,
                            body,
                            follow_redirects,
                            cookie_provider,
                        );
                    } else {
                        tracing::info!(
                            "fetch: CORS error for {}, native fallback disabled by settings",
                            url
                        );
                    }
                }
            }

            // Map common fetch error messages to PP_ERROR codes.
            if err_lower.contains("timeout") {
                return Err(PP_ERROR_CONNECTION_TIMEDOUT);
            }
            if err_lower.contains("network") || err_lower.contains("failed to fetch") {
                return Err(PP_ERROR_CONNECTION_FAILED);
            }
            return Err(PP_ERROR_FAILED);
        }

        let value = resp.get("value").unwrap_or(&resp);

        let status_code = value
            .get("statusCode")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u16;
        let status_text = value
            .get("statusText")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");
        let status_line = format!("HTTP/1.1 {} {}", status_code, status_text);

        // Reconstruct headers string from the JSON response.
        let mut resp_headers = String::new();
        if let Some(hdrs) = value.get("headers").and_then(|v| v.as_object()) {
            for (name, val) in hdrs {
                if let Some(val_str) = val.as_str() {
                    resp_headers.push_str(name);
                    resp_headers.push_str(": ");
                    resp_headers.push_str(val_str);
                    resp_headers.push_str("\r\n");
                }
            }
        }
        resp_headers.push_str("\r\n");

        let final_url = value
            .get("finalUrl")
            .and_then(|v| v.as_str())
            .map(str::to_owned);

        // Decode the base64-encoded body.
        let body_bytes = value
            .get("body")
            .and_then(|v| v.as_str())
            .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
            .unwrap_or_default();

        let content_length = Some(body_bytes.len() as i64);

        // Store Set-Cookie response headers via the cookie provider.
        if let Some(provider) = cookie_provider {
            if let Some(hdrs) = value.get("headers").and_then(|v| v.as_object()) {
                let set_cookie_values: Vec<String> = hdrs
                    .iter()
                    .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
                    .filter_map(|(_, v)| v.as_str().map(String::from))
                    .collect();
                if !set_cookie_values.is_empty() {
                    let store_url = final_url.as_deref().unwrap_or(url);
                    tracing::debug!(
                        "fetch: storing {} Set-Cookie header(s) for {}",
                        set_cookie_values.len(),
                        store_url
                    );
                    provider.set_cookies_from_response(store_url, &set_cookie_values);
                }
            }
        }

        tracing::info!(
            "fetch: HTTP {} {} → {} (content_length={:?})",
            method,
            url,
            status_code,
            content_length
        );

        Ok(HttpResponse {
            status_code,
            status_line,
            headers: resp_headers,
            body: Box::new(std::io::Cursor::new(body_bytes)),
            content_length,
            final_url,
        })
    }
}

// ===========================================================================
// Dispatching HTTP provider — routes to fetch or reqwest based on settings
// ===========================================================================

/// HTTP request provider that dispatches to either the browser's `fetch()`
/// or direct `reqwest` based on the `networkBrowserOnly` setting.
///
/// When `networkBrowserOnly` is `true` (default), uses `FetchHttpRequestProvider`.
/// When `false`, uses `ReqwestHttpRequestProvider` directly.
pub struct DispatchingHttpRequestProvider {
    fetch_provider: FetchHttpRequestProvider,
    reqwest_provider: Arc<dyn HttpRequestProvider>,
}

impl DispatchingHttpRequestProvider {
    pub fn new(
        fetch_provider: FetchHttpRequestProvider,
        reqwest_provider: Arc<dyn HttpRequestProvider>,
    ) -> Self {
        Self {
            fetch_provider,
            reqwest_provider,
        }
    }
}

impl HttpRequestProvider for DispatchingHttpRequestProvider {
    fn http_request(
        &self,
        url: &str,
        method: &str,
        headers: &str,
        body: Option<&[u8]>,
        follow_redirects: bool,
        cookie_provider: Option<&dyn CookieProvider>,
    ) -> Result<HttpResponse, i32> {
        let use_browser = crate::WEB_SETTINGS
            .get()
            .map(|ws| *ws.network_browser_only.lock())
            .unwrap_or(true);

        if use_browser {
            self.fetch_provider.http_request(
                url, method, headers, body, follow_redirects, cookie_provider,
            )
        } else {
            self.reqwest_provider.http_request(
                url, method, headers, body, follow_redirects, cookie_provider,
            )
        }
    }
}
