//! Reqwest-based [`HttpRequestProvider`] implementation.
//!
//! Provides real HTTP/HTTPS networking using the `reqwest` crate with a
//! `rustls` TLS backend.

use std::sync::OnceLock;

use player_ui_traits::{CookieProvider, HttpRequestProvider, HttpResponse};
use ppapi_sys::*;

// ---------------------------------------------------------------------------
// TLS + client singletons
// ---------------------------------------------------------------------------

fn reqwest_tls_config() -> rustls::ClientConfig {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth()
}

fn http_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .tls_backend_preconfigured(reqwest_tls_config())
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 20 {
                    attempt.error("too many redirects")
                } else if attempt
                    .previous()
                    .iter()
                    .any(|u| u.as_str() == attempt.url().as_str())
                {
                    attempt.error("redirect loop detected")
                } else {
                    attempt.follow()
                }
            }))
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("failed to build HTTP client")
    })
}

fn http_client_no_redirect() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .tls_backend_preconfigured(reqwest_tls_config())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("failed to build no-redirect HTTP client")
    })
}

// ---------------------------------------------------------------------------
// Error mapping helpers
// ---------------------------------------------------------------------------

fn map_io_error_kind_to_pp(kind: std::io::ErrorKind) -> i32 {
    use std::io::ErrorKind;
    match kind {
        ErrorKind::TimedOut => PP_ERROR_CONNECTION_TIMEDOUT,
        ErrorKind::ConnectionRefused => PP_ERROR_CONNECTION_REFUSED,
        ErrorKind::ConnectionReset => PP_ERROR_CONNECTION_RESET,
        ErrorKind::ConnectionAborted => PP_ERROR_CONNECTION_ABORTED,
        ErrorKind::NotConnected | ErrorKind::BrokenPipe | ErrorKind::UnexpectedEof => {
            PP_ERROR_CONNECTION_CLOSED
        }
        ErrorKind::AddrInUse => PP_ERROR_ADDRESS_IN_USE,
        ErrorKind::AddrNotAvailable => PP_ERROR_ADDRESS_INVALID,
        _ => PP_ERROR_CONNECTION_FAILED,
    }
}

fn extract_io_error_kind(
    mut current: Option<&(dyn std::error::Error + 'static)>,
) -> Option<std::io::ErrorKind> {
    while let Some(err) = current {
        if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
            return Some(io_err.kind());
        }
        current = err.source();
    }
    None
}

fn map_reqwest_transport_error(error: &reqwest::Error) -> i32 {
    if error.is_timeout() {
        return PP_ERROR_CONNECTION_TIMEDOUT;
    }
    if let Some(kind) = extract_io_error_kind(Some(error)) {
        return map_io_error_kind_to_pp(kind);
    }
    if error.is_connect() {
        let msg = error.to_string().to_ascii_lowercase();
        if msg.contains("failed to lookup address")
            || msg.contains("no such host")
            || msg.contains("name or service not known")
            || msg.contains("temporary failure in name resolution")
            || msg.contains("nodename nor servname")
            || msg.contains("dns")
        {
            return PP_ERROR_NAME_NOT_RESOLVED;
        }
        return PP_ERROR_CONNECTION_FAILED;
    }
    PP_ERROR_FAILED
}

// ---------------------------------------------------------------------------
// ReqwestHttpRequestProvider
// ---------------------------------------------------------------------------

/// HTTP request provider backed by `reqwest` with `rustls`.
pub struct ReqwestHttpRequestProvider;

impl ReqwestHttpRequestProvider {
    pub fn new() -> Self {
        Self
    }
}

impl HttpRequestProvider for ReqwestHttpRequestProvider {
    fn http_request(
        &self,
        url: &str,
        method: &str,
        headers: &str,
        body: Option<&[u8]>,
        follow_redirects: bool,
        cookie_provider: Option<&dyn CookieProvider>,
    ) -> Result<HttpResponse, i32> {
        let client = if follow_redirects {
            http_client()
        } else {
            http_client_no_redirect()
        };

        let http_method = method
            .to_uppercase()
            .parse::<reqwest::Method>()
            .map_err(|e| {
                tracing::warn!("URL open: invalid HTTP method '{}': {}", method, e);
                PP_ERROR_FAILED
            })?;
        let mut req = client.request(http_method, url);

        // Parse PPAPI headers: lines separated by \r\n or \n.
        let mut has_cookie_header = false;
        for line in headers.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((key, value)) = line.split_once(':') {
                if key.trim().eq_ignore_ascii_case("cookie") {
                    has_cookie_header = true;
                }
                req = req.header(key.trim(), value.trim());
            }
        }

        // Inject cookies from the cookie provider if no explicit Cookie
        // header was supplied by the plugin.
        if !has_cookie_header {
            if let Some(provider) = cookie_provider {
                if let Some(cookie_val) = provider.get_cookies_for_url(url) {
                    if !cookie_val.is_empty() {
                        tracing::debug!("URL open: injecting cookies for {}", url);
                        req = req.header("Cookie", cookie_val);
                    }
                }
            }
        }

        if let Some(body_data) = body {
            req = req.body(body_data.to_vec());
        }

        let response = match req.send() {
            Ok(resp) => resp,
            Err(e) => {
                let pp_error = map_reqwest_transport_error(&e);
                tracing::warn!(
                    "URL open: transport error for {}: {} (pp_error={})",
                    url,
                    e,
                    pp_error
                );
                return Err(pp_error);
            }
        };

        let status_code = response.status().as_u16();
        let reason = response.status().canonical_reason().unwrap_or("Unknown");
        let status_line = format!("HTTP/1.1 {} {}", status_code, reason);
        let final_url = response.url().to_string();

        if !(200..=299).contains(&status_code) {
            tracing::info!(
                "URL open: non-2xx HTTP status {} for {} (returned as response, not transport error)",
                status_code,
                url
            );
        }

        // Store Set-Cookie response headers via the cookie provider.
        if let Some(provider) = cookie_provider {
            let set_cookie_values: Vec<String> = response
                .headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|v| v.to_str().ok().map(String::from))
                .collect();
            if !set_cookie_values.is_empty() {
                tracing::debug!(
                    "URL open: storing {} Set-Cookie header(s) for {}",
                    set_cookie_values.len(),
                    &final_url
                );
                provider.set_cookies_from_response(&final_url, &set_cookie_values);
            }
        }

        let mut resp_headers = String::new();
        for (name, val) in response.headers().iter() {
            resp_headers.push_str(name.as_str());
            resp_headers.push_str(": ");
            resp_headers.push_str(val.to_str().unwrap_or(""));
            resp_headers.push_str("\r\n");
        }
        resp_headers.push_str("\r\n");

        let content_length = response.content_length().map(|v| v as i64);

        tracing::info!(
            "URL open: HTTP {} {} → {} (content_length={:?})",
            method,
            url,
            status_code,
            content_length
        );

        Ok(HttpResponse {
            status_code,
            status_line,
            headers: resp_headers,
            body: Box::new(response),
            content_length,
            final_url: Some(final_url),
        })
    }
}
