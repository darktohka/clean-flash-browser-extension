//! PPB_URLLoader;1.0 and PPB_URLLoaderTrusted;0.3 implementation.
//!
//! This module provides a full URLLoader that:
//! - Uses the `HostCallbacks::on_url_open` trait method (or a direct internal
//!   HTTP client if no callbacks are set) to perform HTTP requests.
//! - Streams response data into an internal `VecDeque<u8>` buffer.
//! - Supports redirect following (with loop detection, max 20 hops).
//! - Tracks upload/download progress and fires the trusted status callback.
//! - Limits concurrency to 8 simultaneous requests via a global semaphore.
//!
//! ## Design
//!
//! Since we host the plugin in-process (no IPC), the Chrome two-process model
//! (URLLoaderResource ↔ PepperURLLoaderHost) is collapsed into a single
//! `URLLoaderResource` struct.  Background streaming runs on the shared tokio
//! runtime; data and state updates are synchronized through the
//! `ResourceManager`'s per-resource mutex (via `with_downcast_mut`).
//! Completion callbacks are posted to the main message loop via
//! `MessageLoopPoster`.

use std::any::Any;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::io::Read;
use std::sync::OnceLock;

use player_ui_traits::CookieProvider;
use ppapi_sys::*;
use tokio::sync::Semaphore;

use crate::interface_registry::InterfaceRegistry;
use crate::message_loop::MessageLoopPoster;
use crate::resource::Resource;
use crate::HOST;

use super::url_request_info::URLRequestInfoResource;
use super::url_response_info::URLResponseInfoResource;

// ---------------------------------------------------------------------------
// Concurrency limiter - at most 8 simultaneous in-flight URLLoader requests
// ---------------------------------------------------------------------------

/// Maximum number of concurrent HTTP requests.
const MAX_CONCURRENT_REQUESTS: usize = 8;

fn global_semaphore() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| Semaphore::new(MAX_CONCURRENT_REQUESTS))
}

// ---------------------------------------------------------------------------
// HTTP client infrastructure - self-contained URL loading
// ---------------------------------------------------------------------------

/// Response from performing URL loading (internal to this module).
struct UrlLoadResponse {
    status_code: u16,
    status_line: String,
    headers: String,
    body: Box<dyn std::io::Read + Send>,
    content_length: Option<i64>,
    final_url: Option<String>,
}

#[cfg(feature = "url-reqwest")]
fn reqwest_tls_config() -> rustls::ClientConfig {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth()
}

#[cfg(feature = "url-reqwest")]
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

#[cfg(feature = "url-reqwest")]
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

#[cfg(feature = "url-reqwest")]
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

#[cfg(feature = "url-reqwest")]
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

#[cfg(feature = "url-reqwest")]
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

/// Perform URL loading: file paths, http/https (via reqwest), or stub 404.
///
/// This is the self-contained URL loader that replaces the previous
/// `HostCallbacks::on_url_open` approach.
/// Fake crossdomain.xml served for any request whose path ends with
/// `/crossdomain.xml` (case-insensitive).  This allows Flash content to make
/// cross-origin HTTP requests without depending on the remote server hosting
/// a real policy file.
const FAKE_CROSSDOMAIN_XML: &[u8] = b"\
<cross-domain-policy>\n\
    <site-control permitted-cross-domain-policies=\"all\"/>\n\
    <allow-access-from domain=\"*\" secure=\"false\"/>\n\
    <allow-http-request-headers-from domain=\"*\" headers=\"*\" secure=\"false\"/>\n\
</cross-domain-policy>\n";

/// Returns `true` when the URL path component ends with `/crossdomain.xml`
/// (case-insensitive), so we can intercept and serve a permissive policy.
fn is_crossdomain_xml_request(url: &str) -> bool {
    // Strip query string and fragment to isolate the path.
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let path_lower = path.to_ascii_lowercase();
    path_lower.ends_with("/crossdomain.xml")
}

fn perform_url_open(
    url: &str,
    method: &str,
    headers: &str,
    body: Option<&[u8]>,
    follow_redirects: bool,
    cookie_provider: Option<&dyn CookieProvider>,
) -> Result<UrlLoadResponse, i32> {
    tracing::info!("URL open requested: {} {}", method, url);

    // Suppress unused-variable warnings when reqwest is not enabled.
    #[cfg(not(feature = "url-reqwest"))]
    let _ = (headers, body, follow_redirects, cookie_provider);

    // ----- crossdomain.xml interception -----
    if is_crossdomain_xml_request(url) {
        tracing::info!(
            "URL open: intercepting crossdomain.xml request for {} - serving fake permissive policy",
            url
        );
        let body_bytes: &[u8] = FAKE_CROSSDOMAIN_XML;
        let len = body_bytes.len() as i64;
        let headers_str = format!(
            "Content-Type: text/xml\r\nContent-Length: {}\r\n\r\n",
            len
        );
        return Ok(UrlLoadResponse {
            status_code: 200,
            status_line: "HTTP/1.1 200 OK".to_string(),
            headers: headers_str,
            body: Box::new(std::io::Cursor::new(body_bytes.to_vec())),
            content_length: Some(len),
            final_url: Some(url.to_string()),
        });
    }

    // ----- file:// or bare path → local filesystem -----
    let path = if let Some(stripped) = url.strip_prefix("file://") {
        stripped
    } else {
        url
    };

    if let Ok(file) = std::fs::File::open(path) {
        let meta = file.metadata().ok();
        let len = meta.as_ref().map(|m| m.len() as i64);
        let content_type = if path.to_ascii_lowercase().ends_with(".swf") {
            "application/x-shockwave-flash"
        } else {
            "application/octet-stream"
        };
        let headers_str = format!(
            "Content-Type: {}\r\n{}\r\n",
            content_type,
            len.map(|l| format!("Content-Length: {}\r\n", l))
                .unwrap_or_default(),
        );
        tracing::info!(
            "URL open: serving file {} ({} bytes)",
            path,
            len.unwrap_or(-1)
        );
        return Ok(UrlLoadResponse {
            status_code: 200,
            status_line: "HTTP/1.1 200 OK".to_string(),
            headers: headers_str,
            body: Box::new(std::io::BufReader::new(file)),
            content_length: len,
            final_url: Some(url.to_string()),
        });
    }

    // ----- http:// / https:// → reqwest -----
    #[cfg(feature = "url-reqwest")]
    if url.starts_with("http://") || url.starts_with("https://") {
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

        return Ok(UrlLoadResponse {
            status_code,
            status_line,
            headers: resp_headers,
            body: Box::new(response),
            content_length,
            final_url: Some(final_url),
        });
    }

    // ----- http:// / https:// → stub (404) -----
    #[cfg(all(feature = "url-stub", not(feature = "url-reqwest")))]
    if url.starts_with("http://") || url.starts_with("https://") {
        tracing::warn!("URL open: stub loader returning 404 for {}", url);
        return Ok(UrlLoadResponse {
            status_code: 404,
            status_line: "HTTP/1.1 404 Not Found".to_string(),
            headers: "Content-Length: 0\r\n\r\n".to_string(),
            body: Box::new(std::io::empty()),
            content_length: Some(0),
            final_url: Some(url.to_string()),
        });
    }

    // ----- Unknown scheme / not found -----
    tracing::warn!("Could not open URL: {} (path: {})", url, path);
    Err(PP_ERROR_FILENOTFOUND)
}

/// Maximum number of redirects before we declare a loop.
const MAX_REDIRECTS: usize = 20;

/// Size of each read chunk when streaming the response body.
const STREAM_CHUNK_SIZE: usize = 32 * 1024; // 32 KB

// ---------------------------------------------------------------------------
// URLLoader state machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Plugin has not called Open() yet.
    WaitingToOpen,
    /// Open() or FollowRedirect() is in progress; waiting for headers.
    Opening,
    /// Response received; data is streaming in.
    StreamingData,
    /// Load finished (success or error) - all data consumed or error set.
    LoadComplete,
}

// ---------------------------------------------------------------------------
// URLLoaderResource
// ---------------------------------------------------------------------------

/// The core URLLoader resource, combining Chrome's `URLLoaderResource` (plugin
/// side) and `PepperURLLoaderHost` (browser side) into a single struct.
pub struct URLLoaderResource {
    pub instance: PP_Instance,
    mode: Mode,

    // ---- Request configuration (copied from URLRequestInfo on Open) ----
    url: String,
    method: String,
    request_headers: String,
    request_body: Vec<u8>,
    follow_redirects: bool,
    record_download_progress: bool,
    record_upload_progress: bool,
    stream_to_file: bool,
    allow_cross_origin_requests: bool,
    allow_credentials: bool,
    custom_referrer_url: Option<String>,
    custom_content_transfer_encoding: Option<String>,
    custom_user_agent: Option<String>,

    // ---- Response state ----
    response_info_id: Option<PP_Resource>,
    status_code: i32,
    status_line: String,
    response_headers: String,
    redirect_url: String,
    response_url: String,

    // ---- Streaming buffer ----
    buffer: VecDeque<u8>,
    /// `None` = still loading/waiting, `Some(PP_OK)` = clean finish,
    /// `Some(PP_ERROR_*)` = load failed/aborted.
    done_status: Option<i32>,

    // ---- Progress tracking ----
    bytes_sent: i64,
    total_bytes_to_be_sent: i64,
    bytes_received: i64,
    total_bytes_to_be_received: i64,

    // ---- Trusted interface state ----
    has_universal_access: bool,
    status_callback: PP_URLLoaderTrusted_StatusCallback,

    // ---- Pending callback (only one at a time) ----
    /// Stored callback for Open, FollowRedirect, or ReadResponseBody.
    pending_callback: Option<PP_CompletionCallback>,

    // ---- ReadResponseBody pending state ----
    /// When a ReadResponseBody call cannot be satisfied immediately, we save
    /// the user's buffer pointer and size here.  The background streaming task
    /// fills it when data arrives.
    user_buffer_ptr: usize, // stored as usize to be Send-safe (cast back on use)
    user_buffer_size: usize,

    // ---- Redirect tracking for loop detection ----
    redirect_chain: Vec<String>,

    // ---- Abort flag shared with background task ----
    /// When the loader is closed, we set this to signal the background task
    /// to stop streaming.
    abort_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Resource for URLLoaderResource {
    fn resource_type(&self) -> &'static str {
        "PPB_URLLoader"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// Safety: The user_buffer_ptr is only dereferenced while the plugin is blocked
// on a callback (i.e. no concurrent access), and only from the thread that
// eventually fires the callback.
unsafe impl Send for URLLoaderResource {}
unsafe impl Sync for URLLoaderResource {}

impl URLLoaderResource {
    fn new(instance: PP_Instance) -> Self {
        Self {
            instance,
            mode: Mode::WaitingToOpen,
            url: String::new(),
            method: String::from("GET"),
            request_headers: String::new(),
            request_body: Vec::new(),
            follow_redirects: true,
            record_download_progress: false,
            record_upload_progress: false,
            stream_to_file: false,
            allow_cross_origin_requests: false,
            allow_credentials: false,
            custom_referrer_url: None,
            custom_content_transfer_encoding: None,
            custom_user_agent: None,
            response_info_id: None,
            status_code: 0,
            status_line: String::new(),
            response_headers: String::new(),
            redirect_url: String::new(),
            response_url: String::new(),
            buffer: VecDeque::new(),
            done_status: None,
            bytes_sent: 0,
            total_bytes_to_be_sent: -1,
            bytes_received: 0,
            total_bytes_to_be_received: -1,
            has_universal_access: false,
            status_callback: None,
            pending_callback: None,
            user_buffer_ptr: 0,
            user_buffer_size: 0,
            redirect_chain: Vec::new(),
            abort_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

// ---------------------------------------------------------------------------
// PPB_URLLoader;1.0 vtable
// ---------------------------------------------------------------------------

static VTABLE: PPB_URLLoader_1_0 = PPB_URLLoader_1_0 {
    Create: Some(create),
    IsURLLoader: Some(is_url_loader),
    Open: Some(open),
    FollowRedirect: Some(follow_redirect),
    GetUploadProgress: Some(get_upload_progress),
    GetDownloadProgress: Some(get_download_progress),
    GetResponseInfo: Some(get_response_info),
    ReadResponseBody: Some(read_response_body),
    FinishStreamingToFile: Some(finish_streaming_to_file),
    Close: Some(close),
};

static TRUSTED_VTABLE: PPB_URLLoaderTrusted_0_3 = PPB_URLLoaderTrusted_0_3 {
    GrantUniversalAccess: Some(grant_universal_access),
    RegisterStatusCallback: Some(register_status_callback),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_URLLOADER_INTERFACE_1_0, &VTABLE);
        registry.register(PPB_URLLOADERTRUSTED_INTERFACE_0_3, &TRUSTED_VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Helper: post a callback to the main message loop
// ---------------------------------------------------------------------------

fn get_main_poster() -> Option<MessageLoopPoster> {
    HOST.get()?.main_loop_poster.lock().clone()
}

/// Post a completion callback to the main message loop.
fn post_callback(poster: &MessageLoopPoster, cb: PP_CompletionCallback, result: i32) {
    poster.post_work(cb, 0, result);
}

// ---------------------------------------------------------------------------
// Synchronous (blocking) Open path
// ---------------------------------------------------------------------------

/// Perform the HTTP request synchronously on the calling thread, populate the
/// loader with response metadata, then spawn a background task to stream the
/// body.  Returns `PP_OK` on success or an error code.
///
/// This is used when `PP_BlockUntilComplete()` is passed as the callback,
/// e.g. for `HandleDocumentLoad` document loaders.
#[allow(clippy::too_many_arguments)]
fn open_blocking(
    loader_id: PP_Resource,
    url: String,
    method: String,
    headers: String,
    body: Option<Vec<u8>>,
    follow_redirects: bool,
    poster: MessageLoopPoster,
    abort_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> i32 {
    let body_opt = body.as_deref().filter(|b| !b.is_empty());

    // Obtain the cookie provider once for both the request and response.
    let cookie_provider = HOST.get().and_then(|h| h.get_cookie_provider());

    // Perform the HTTP request synchronously on the calling thread.
    let response = perform_url_open(&url, &method, &headers, body_opt, follow_redirects, cookie_provider.as_deref());
    tracing::warn!("URLLoader (blocking): performed URL open for {} {}", method, url);

    let response = match response {
        Ok(resp) => resp,
        Err(pp_error) => {
            tracing::warn!("URLLoader (blocking): failed to open URL {}: pp_error={}", url, pp_error);
            // Match Chrome: set mode to LoadComplete with PP_ERROR_FAILED,
            // clear user_buffer, and fire any pending callback.
            if let Some(host) = HOST.get() {
                let maybe_cb = host.resources
                    .with_downcast_mut::<URLLoaderResource, _>(loader_id, |ul| {
                        ul.mode = Mode::LoadComplete;
                        ul.done_status = Some(PP_ERROR_FAILED);
                        ul.user_buffer_ptr = 0;
                        ul.user_buffer_size = 0;
                        ul.pending_callback.take().map(|c| (c, PP_ERROR_FAILED))
                    })
                    .flatten();
                // Fire callback (if any) outside the resource lock.
                if let Some((cb, result)) = maybe_cb {
                    post_callback(&poster, cb, result);
                }
            }
            return PP_ERROR_FAILED;
        }
    };

    // ---- Store response metadata (same logic as spawn_open_task) ----
    let content_length = response.content_length;
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    let is_redirect = (300..=399).contains(&(response.status_code as i32));
    let extracted_redirect_url = if is_redirect {
        extract_location_header(&response.headers)
    } else {
        None
    };

    let redirect_loop = if let Some(ref redir) = extracted_redirect_url {
        host.resources
            .with_downcast::<URLLoaderResource, _>(loader_id, |ul| {
                ul.redirect_chain.contains(redir) || ul.redirect_chain.len() >= MAX_REDIRECTS
            })
            .unwrap_or(false)
    } else {
        false
    };

    if redirect_loop {
        tracing::warn!(
            "URLLoader (blocking): redirect loop detected for {}",
            extracted_redirect_url.as_deref().unwrap_or("?")
        );
        return PP_ERROR_FAILED;
    }

    let should_auto_follow = host
        .resources
        .with_downcast::<URLLoaderResource, _>(loader_id, |ul| ul.follow_redirects)
        .unwrap_or(true);

    let response_url = response.final_url.unwrap_or_else(|| url.clone());

    host.resources
        .with_downcast_mut::<URLLoaderResource, _>(loader_id, |ul| {
            ul.status_code = response.status_code as i32;
            ul.status_line = response.status_line.clone();
            ul.response_headers = response.headers.clone();
            ul.response_url = response_url.clone();
            ul.total_bytes_to_be_received = content_length.unwrap_or(-1);
            if let Some(ref redir) = extracted_redirect_url {
                ul.redirect_url = redir.clone();
            }
            if !is_redirect || should_auto_follow {
                ul.mode = Mode::StreamingData;
            }
        });

    // Create the URLResponseInfoResource.
    let resp_info = URLResponseInfoResource {
        url: response_url,
        status_code: response.status_code as i32,
        status_line: response.status_line.clone(),
        headers: response.headers.clone(),
        redirect_url: extracted_redirect_url.clone().unwrap_or_default(),
    };
    let resp_id = host.resources.insert(
        host.resources.get_instance(loader_id).unwrap_or(0),
        Box::new(resp_info),
    );
    host.resources
        .with_downcast_mut::<URLLoaderResource, _>(loader_id, |ul| {
            ul.response_info_id = Some(resp_id);
        });

    // If this is a redirect the plugin needs to inspect, don't stream the body.
    if is_redirect && !should_auto_follow {
        return PP_OK;
    }

    // Spawn only the body-streaming task in the background.
    let abort_clone = abort_flag.clone();
    let rt = crate::tokio_runtime();
    rt.spawn(async move {
        let _permit = global_semaphore().acquire().await;
        if _permit.is_err() {
            finish_loading(loader_id, PP_ERROR_ABORTED, &poster);
            return;
        }
        stream_body(loader_id, response.body, &poster, &abort_clone);
    });

    PP_OK
}

// ---------------------------------------------------------------------------
// Background task: stream response body into the loader's buffer
// ---------------------------------------------------------------------------

/// Spawn a tokio task that:
/// 1. Acquires a concurrency permit.
/// 2. Calls `HostCallbacks::on_url_open` on a blocking thread.
/// 3. Streams the response body into the URLLoaderResource buffer.
/// 4. Fires the pending open callback, then handles ReadResponseBody wakeups.
fn spawn_open_task(
    loader_id: PP_Resource,
    url: String,
    method: String,
    headers: String,
    body: Option<Vec<u8>>,
    follow_redirects: bool,
    poster: MessageLoopPoster,
    abort_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    // Obtain the cookie provider once, before entering the async task.
    let cookie_provider = HOST.get().and_then(|h| h.get_cookie_provider());

    let rt = crate::tokio_runtime();
    rt.spawn(async move {
        // ---- Step 1: Acquire concurrency permit ----
        let _permit = global_semaphore().acquire().await;
        if _permit.is_err() {
            // Semaphore closed - host is shutting down.
            finish_loading(loader_id, PP_ERROR_ABORTED, &poster);
            return;
        }
        let _permit = _permit.unwrap();

        // Check abort before starting.
        if abort_flag.load(std::sync::atomic::Ordering::Relaxed) {
            finish_loading(loader_id, PP_ERROR_ABORTED, &poster);
            return;
        }

        // ---- Step 2: Perform HTTP request on a blocking thread ----
        let body_opt = if body.as_ref().map_or(true, |b| b.is_empty()) {
            None
        } else {
            body.as_deref().map(|b| b.to_vec())
        };

        let response = {
            let url_c = url.clone();
            let method_c = method.clone();
            let headers_c = headers.clone();
            let body_c = body_opt;
            let cp = cookie_provider.clone();
            // Run on blocking thread pool since perform_url_open may block.
            tokio::task::spawn_blocking(move || {
                perform_url_open(
                    &url_c,
                    &method_c,
                    &headers_c,
                    body_c.as_deref(),
                    follow_redirects,
                    cp.as_deref(),
                )
            })
            .await
        };

        let response = match response {
            Ok(Ok(resp)) => resp,
            Ok(Err(pp_error)) => {
                tracing::warn!("URLLoader: open failed for loader_id {} with pp_error={}", loader_id, pp_error);
                finish_loading(loader_id, PP_ERROR_FAILED, &poster);
                return;
            }
            Err(_join_error) => {
                tracing::warn!("URLLoader: open task panicked for loader_id {}", loader_id);
                finish_loading(loader_id, PP_ERROR_FAILED, &poster);
                return;
            }
        };

        // ---- Step 3: Store response metadata ----
        let content_length = response.content_length;
        let Some(host) = HOST.get() else {
            return;
        };

        // Check if this is a redirect that the plugin needs to see.
        let is_redirect = (300..=399).contains(&(response.status_code as i32));

        // Determine the redirect URL from Location header if present.
        let extracted_redirect_url = if is_redirect {
            extract_location_header(&response.headers)
        } else {
            None
        };

        // Check for redirect loop before storing.
        let redirect_loop = if let Some(ref redir) = extracted_redirect_url {
            host.resources
                .with_downcast::<URLLoaderResource, _>(loader_id, |ul| {
                    ul.redirect_chain.contains(redir)
                        || ul.redirect_chain.len() >= MAX_REDIRECTS
                })
                .unwrap_or(false)
        } else {
            false
        };

        if redirect_loop {
            tracing::warn!(
                "URLLoader: redirect loop detected for {}",
                extracted_redirect_url.as_deref().unwrap_or("?")
            );
            finish_loading(loader_id, PP_ERROR_FAILED, &poster);
            return;
        }

        // We need to check follow_redirects state from the resource.
        let should_auto_follow = host
            .resources
            .with_downcast::<URLLoaderResource, _>(loader_id, |ul| ul.follow_redirects)
            .unwrap_or(true);

        // If this is a redirect AND follow_redirects was true, the host
        // callbacks should have followed it already (reqwest follows redirects
        // by default when follow_redirects=true).  If not, we surface the
        // redirect to the plugin.

        // Use the final URL from the response (after any redirects) rather
        // than the original request URL, matching Chrome's DataFromWebURLResponse.
        let response_url = response.final_url.unwrap_or_else(|| url.clone());

        host.resources
            .with_downcast_mut::<URLLoaderResource, _>(loader_id, |ul| {
                ul.status_code = response.status_code as i32;
                ul.status_line = response.status_line.clone();
                ul.response_headers = response.headers.clone();
                ul.response_url = response_url.clone();
                ul.total_bytes_to_be_received = content_length.unwrap_or(-1);
                if let Some(ref redir) = extracted_redirect_url {
                    ul.redirect_url = redir.clone();
                }
                // For non-redirect responses or auto-followed redirects, we
                // transition to streaming mode.
                if !is_redirect || should_auto_follow {
                    ul.mode = Mode::StreamingData;
                }
            });

        // Create a URLResponseInfoResource.
        let resp_info = URLResponseInfoResource {
            url: response_url,
            status_code: response.status_code as i32,
            status_line: response.status_line.clone(),
            headers: response.headers.clone(),
            redirect_url: extracted_redirect_url.clone().unwrap_or_default(),
        };
        let resp_id = host.resources.insert(
            host.resources
                .get_instance(loader_id)
                .unwrap_or(0),
            Box::new(resp_info),
        );
        host.resources
            .with_downcast_mut::<URLLoaderResource, _>(loader_id, |ul| {
                ul.response_info_id = Some(resp_id);
            });

        // ---- Fire the open callback ----
        fire_open_callback(loader_id, &poster);

        // If this was a redirect that the plugin needs to inspect (follow_redirects=false),
        // don't stream the body - wait for FollowRedirect.
        if is_redirect && !should_auto_follow {
            return;
        }

        // ---- Step 4: Stream response body in chunks ----
        stream_body(loader_id, response.body, &poster, &abort_flag);

        // _permit is dropped here, releasing the concurrency slot.
    });
}

/// Extract the `Location` header value from a header string.
fn extract_location_header(headers: &str) -> Option<String> {
    for line in headers.lines() {
        let line = line.trim_end_matches('\r');
        if let Some((key, value)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case("location") {
                let v = value.trim().to_string();
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Fire the pending Open/FollowRedirect callback with PP_OK (success).
///
/// For **error** paths, use `finish_loading(loader_id, PP_ERROR_FAILED, poster)`
/// instead - that matches Chrome's `FinishedLoading` code path which
/// atomically sets mode + done_status and fires the pending callback.
fn fire_open_callback(loader_id: PP_Resource, poster: &MessageLoopPoster) {
    tracing::debug!("URLLoader: firing open callback with PP_OK for loader_id {}", loader_id);
    let Some(host) = HOST.get() else {
        return;
    };
    let cb = host
        .resources
        .with_downcast_mut::<URLLoaderResource, _>(loader_id, |ul| {
            ul.pending_callback.take()
        })
        .flatten();
    if let Some(cb) = cb {
        post_callback(poster, cb, PP_OK);
    }
}

/// Stream the response body in chunks, updating progress and fulfilling
/// pending ReadResponseBody calls.
fn stream_body(
    loader_id: PP_Resource,
    mut body: Box<dyn Read + Send>,
    poster: &MessageLoopPoster,
    abort_flag: &std::sync::atomic::AtomicBool,
) {
    let mut chunk = vec![0u8; STREAM_CHUNK_SIZE];

    loop {
        if abort_flag.load(std::sync::atomic::Ordering::Relaxed) {
            finish_loading(loader_id, PP_ERROR_ABORTED, poster);
            return;
        }

        let bytes_read = match body.read(&mut chunk) {
            Ok(0) => {
                tracing::debug!("URLLoader: reached EOF");
                // EOF
                finish_loading(loader_id, PP_OK, poster);
                return;
            }
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("URLLoader: read error: {}", e);
                finish_loading(loader_id, PP_ERROR_FAILED, poster);
                return;
            }
        };

        let Some(host) = HOST.get() else {
            return;
        };

        // Append data to the loader's buffer and update progress.
        // Extract status callback info and pending read state while holding
        // the lock, but fire the status callback AFTER releasing it to avoid
        // deadlocks if the callback re-enters PPAPI.
        let (status_cb_info, maybe_read_cb) = host
            .resources
            .with_downcast_mut::<URLLoaderResource, _>(loader_id, |ul| {
                // Update download progress.
                ul.bytes_received += bytes_read as i64;

                // Capture status callback info to fire outside the lock.
                let cb_info = ul.status_callback.map(|cb_fn| {
                    (
                        cb_fn,
                        ul.instance,
                        ul.bytes_sent,
                        ul.total_bytes_to_be_sent,
                        ul.bytes_received,
                        ul.total_bytes_to_be_received,
                    )
                });

                // If a ReadResponseBody is pending and waiting for data,
                // fill the user buffer directly instead of buffering.
                let read_cb = if ul.user_buffer_size > 0 && ul.pending_callback.is_some() {
                    let copy_len = bytes_read.min(ul.user_buffer_size);
                    let dst = ul.user_buffer_ptr as *mut u8;
                    // Safety: the plugin is blocked waiting for the callback;
                    // the buffer pointer is valid until the callback fires.
                    unsafe {
                        std::ptr::copy_nonoverlapping(chunk.as_ptr(), dst, copy_len);
                    }
                    ul.user_buffer_ptr = 0;
                    ul.user_buffer_size = 0;

                    // If there's leftover data beyond what the user requested,
                    // buffer it.
                    if bytes_read > copy_len {
                        ul.buffer.extend(&chunk[copy_len..bytes_read]);
                    }

                    // Return the callback to fire.
                    let cb = ul.pending_callback.take();
                    cb.map(|c| (c, copy_len as i32))
                } else {
                    // No pending read - just buffer the data.
                    ul.buffer.extend(&chunk[..bytes_read]);
                    None
                };

                (cb_info, read_cb)
            })
            .unwrap_or((None, None));

        // Fire the trusted status callback outside the lock.
        if let Some((cb_fn, instance, bs, tbs, br, tbr)) = status_cb_info {
            tracing::trace!("Firing status callback: bytes_sent={}, total_bytes_to_be_sent={}, bytes_received={}, total_bytes_to_be_received={}", bs, tbs, br, tbr);
            unsafe {
                cb_fn(instance, loader_id, bs, tbs, br, tbr);
            }
        }

        // Fire ReadResponseBody callback if we just satisfied one.
        if let Some((cb, n)) = maybe_read_cb {
            post_callback(poster, cb, n);
        }
    }
}

/// Mark the load as complete and wake any pending ReadResponseBody.
///
/// This matches Chrome's `OnPluginMsgFinishedLoading`: it atomically sets
/// mode + done_status, clears user_buffer, and fires whatever callback is
/// pending (Open, FollowRedirect, or ReadResponseBody).
fn finish_loading(loader_id: PP_Resource, status: i32, poster: &MessageLoopPoster) {
    let Some(host) = HOST.get() else {
        return;
    };

    let maybe_cb = host
        .resources
        .with_downcast_mut::<URLLoaderResource, _>(loader_id, |ul| {
            ul.mode = Mode::LoadComplete;
            ul.done_status = Some(status);

            // Snapshot and clear user_buffer before processing, matching
            // Chrome's OnPluginMsgFinishedLoading behaviour.
            let had_user_buffer = ul.user_buffer_size > 0;
            let saved_buf_ptr = ul.user_buffer_ptr;
            let saved_buf_size = ul.user_buffer_size;
            ul.user_buffer_ptr = 0;
            ul.user_buffer_size = 0;

            // If a ReadResponseBody is pending with a user buffer, try to
            // drain remaining buffered data into it before reporting EOF/error.
            if ul.pending_callback.is_some() && had_user_buffer {
                if !ul.buffer.is_empty() {
                    let copy_len = ul.buffer.len().min(saved_buf_size);
                    let dst = saved_buf_ptr as *mut u8;
                    let (a, b) = ul.buffer.as_slices();
                    let mut offset = 0;
                    for slice in [a, b] {
                        let take = slice.len().min(copy_len - offset);
                        if take == 0 {
                            break;
                        }
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                slice.as_ptr(),
                                dst.add(offset),
                                take,
                            );
                        }
                        offset += take;
                    }
                    ul.buffer.drain(..copy_len);
                    ul.pending_callback.take().map(|c| (c, copy_len as i32))
                } else {
                    // Buffer empty - report EOF (0 bytes) or error.
                    let result = if status == PP_OK { 0 } else { status };
                    ul.pending_callback.take().map(|c| (c, result))
                }
            } else if ul.pending_callback.is_some() {
                // A callback is pending but no user_buffer (Open or FollowRedirect).
                let result = if status == PP_OK { PP_OK } else { status };
                ul.pending_callback.take().map(|c| (c, result))
            } else {
                None
            }
        })
        .flatten();

    if let Some((cb, result)) = maybe_cb {
        post_callback(poster, cb, result);
    }
}

// ===========================================================================
// PPB_URLLoader;1.0 extern "C" functions
// ===========================================================================

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    tracing::debug!("PPB_URLLoader::Create(instance={})", instance);
    let Some(host) = HOST.get() else {
        return 0;
    };
    host.resources
        .insert(instance, Box::new(URLLoaderResource::new(instance)))
}

unsafe extern "C" fn is_url_loader(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_URLLoader")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn open(
    loader: PP_Resource,
    request_info: PP_Resource,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!(
        "PPB_URLLoader::Open(loader={}, request_info={}, callback={:?})",
        loader,
        request_info,
        callback.func.map(|f| f as usize)
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    // ---- Extract request configuration from URLRequestInfoResource ----
    let req_data = host
        .resources
        .with_downcast::<URLRequestInfoResource, _>(request_info, |req| {
            tracing::debug!("PPB_URLLoader::Open: got request info: {:?}", req);
            (
                req.url.clone().unwrap_or_default(),
                req.method.clone().unwrap_or_else(|| "GET".to_string()),
                req.headers.clone().unwrap_or_default(),
                req.body.clone(),
                req.follow_redirects,
                req.record_download_progress,
                req.record_upload_progress,
                req.stream_to_file,
                req.allow_cross_origin_requests,
                req.allow_credentials,
                if req.has_custom_referrer_url {
                    Some(req.custom_referrer_url.clone())
                } else {
                    None
                },
                if req.has_custom_content_transfer_encoding {
                    Some(req.custom_content_transfer_encoding.clone())
                } else {
                    None
                },
                if req.has_custom_user_agent {
                    Some(req.custom_user_agent.clone())
                } else {
                    None
                },
            )
        });

    let Some((
        url,
        method,
        headers,
        body,
        follow_redirects,
        record_download_progress,
        record_upload_progress,
        stream_to_file,
        allow_cross_origin_requests,
        allow_credentials,
        custom_referrer_url,
        custom_content_transfer_encoding,
        custom_user_agent,
    )) = req_data
    else {
        tracing::warn!(
            "PPB_URLLoader::Open: invalid request_info resource {}",
            request_info
        );
        return PP_ERROR_BADARGUMENT;
    };

    if url.is_empty() {
        tracing::warn!("PPB_URLLoader::Open: empty URL");
        return PP_ERROR_BADARGUMENT;
    }

    // ---- Resolve relative URLs against the document base URL ----
    // Mirrors Chrome (PepperURLLoaderHost) behavior:
    // the browser resolves the URLRequestInfo URL relative to the embedding
    // page's base URL before issuing the request.
    let instance_id = host.resources.get_instance(loader).unwrap_or(0);
    let url = if url.starts_with("javascript:") {
        url
    } else {
        tracing::trace!("PPB_URLLoader::Open: resolving URL against document base");
        let base = super::url_util::document_base_url(host, instance_id);
        tracing::trace!("PPB_URLLoader::Open: document base URL is {:?}, resolving url {}", base, url);
        super::url_util::resolve_url(base.as_deref(), &url).unwrap_or(url)
    };

    // ---- Check if this request requires universal access (C++ parity) ----
    // Mirrors URLRequestRequiresUniversalAccess() from
    // pepper_url_loader_host.cc: custom referrer, custom content-transfer-
    // encoding, custom user-agent, or javascript: scheme all require it.
    let needs_universal = custom_referrer_url.is_some()
        || custom_content_transfer_encoding.is_some()
        || custom_user_agent.is_some()
        || url.starts_with("javascript:");

    if needs_universal {
        let has_ua = host
            .resources
            .with_downcast::<URLLoaderResource, _>(loader, |ul| ul.has_universal_access)
            .unwrap_or(false);
        if !has_ua {
            tracing::warn!(
                "PPB_URLLoader::Open: URL requires universal access but loader \
                 does not have it (url={})",
                url
            );
            return PP_ERROR_NOACCESS;
        }
    }

    // ---- Validate and update loader state ----
    let is_blocking = callback.func.is_none();
    let setup_ok = host
        .resources
        .with_downcast_mut::<URLLoaderResource, _>(loader, |ul| {
            if ul.mode != Mode::WaitingToOpen {
                return Err(PP_ERROR_INPROGRESS);
            }
            ul.mode = Mode::Opening;
            ul.url = url.clone();
            ul.method = method.clone();
            ul.request_headers = headers.clone();
            ul.request_body = body.clone();
            ul.follow_redirects = follow_redirects;
            ul.record_download_progress = record_download_progress;
            ul.record_upload_progress = record_upload_progress;
            ul.stream_to_file = stream_to_file;
            ul.allow_cross_origin_requests = allow_cross_origin_requests;
            ul.allow_credentials = allow_credentials;
            ul.custom_referrer_url = custom_referrer_url.clone();
            ul.custom_content_transfer_encoding = custom_content_transfer_encoding.clone();
            ul.custom_user_agent = custom_user_agent.clone();
            ul.redirect_chain.clear();
            ul.redirect_chain.push(url.clone());

            // Set upload progress total if we have a body.
            if !ul.request_body.is_empty() {
                ul.total_bytes_to_be_sent = ul.request_body.len() as i64;
                ul.bytes_sent = ul.request_body.len() as i64; // sent at open time
            }

            if !is_blocking {
                ul.pending_callback = Some(callback);
            }
            // Reset abort flag for new open.
            ul.abort_flag
                .store(false, std::sync::atomic::Ordering::Relaxed);
            Ok(ul.abort_flag.clone())
        });

    let abort_flag = match setup_ok {
        Some(Ok(flag)) => flag,
        Some(Err(e)) => return e,
        None => {
            tracing::warn!(
                "PPB_URLLoader::Open: invalid loader resource {}",
                loader
            );
            return PP_ERROR_BADRESOURCE;
        }
    };

    // ---- Get message loop poster ----
    let Some(poster) = get_main_poster() else {
        tracing::warn!("PPB_URLLoader::Open: no main message loop");
        return PP_ERROR_NO_MESSAGE_LOOP;
    };

    // ---- Blocking (synchronous) vs async path ----
    let body_opt = if body.is_empty() { None } else { Some(body) };

    if is_blocking {
        // PP_BlockUntilComplete: perform the HTTP request on the current
        // thread, populate response metadata, then spawn only the body
        // streaming task. Returns PP_OK synchronously.
        return open_blocking(
            loader,
            url,
            method,
            headers,
            body_opt,
            follow_redirects,
            poster,
            abort_flag,
        );
    }

    tracing::debug!("PPB_URLLoader::Open: async path for url {}", url);
    // ---- Spawn background task (async path) ----
    spawn_open_task(
        loader,
        url,
        method,
        headers,
        body_opt,
        follow_redirects,
        poster,
        abort_flag,
    );

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn follow_redirect(
    loader: PP_Resource,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!(
        "PPB_URLLoader::FollowRedirect(loader={})",
        loader
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    // Extract the redirect URL and validate state.
    let redirect_data = host
        .resources
        .with_downcast_mut::<URLLoaderResource, _>(loader, |ul| {
            if ul.mode != Mode::Opening && ul.mode != Mode::StreamingData {
                return Err(PP_ERROR_INPROGRESS);
            }
            if ul.redirect_url.is_empty() {
                return Err(PP_ERROR_FAILED);
            }
            if ul.pending_callback.is_some() {
                return Err(PP_ERROR_INPROGRESS);
            }

            let redirect_url = ul.redirect_url.clone();

            // Check redirect loop.
            if ul.redirect_chain.contains(&redirect_url)
                || ul.redirect_chain.len() >= MAX_REDIRECTS
            {
                tracing::warn!(
                    "PPB_URLLoader::FollowRedirect: redirect loop detected → {}",
                    redirect_url
                );
                return Err(PP_ERROR_FAILED);
            }

            // Clear old response state, per Chrome.
            ul.response_info_id = None;
            ul.status_code = 0;
            ul.status_line.clear();
            ul.response_headers.clear();
            ul.response_url.clear();
            ul.buffer.clear();
            ul.done_status = None;
            ul.bytes_received = 0;
            ul.total_bytes_to_be_received = -1;

            // Per HTTP spec, redirects change to GET and drop the body.
            ul.method = "GET".to_string();
            ul.request_body.clear();
            ul.bytes_sent = 0;
            ul.total_bytes_to_be_sent = -1;

            ul.url = redirect_url.clone();
            ul.redirect_chain.push(redirect_url.clone());
            ul.redirect_url.clear();
            ul.mode = Mode::Opening;
            ul.pending_callback = Some(callback);
            ul.abort_flag
                .store(false, std::sync::atomic::Ordering::Relaxed);

            Ok((redirect_url, ul.request_headers.clone(), ul.abort_flag.clone()))
        });

    let (redirect_url, headers, abort_flag) = match redirect_data {
        Some(Ok(data)) => data,
        Some(Err(e)) => return e,
        None => return PP_ERROR_BADRESOURCE,
    };

    let Some(poster) = get_main_poster() else {
        return PP_ERROR_NO_MESSAGE_LOOP;
    };

    // Spawn a new open task for the redirect URL.
    spawn_open_task(
        loader,
        redirect_url,
        "GET".to_string(),
        headers,
        None,    // no body on redirect
        true,    // follow further redirects automatically
        poster,
        abort_flag,
    );

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn get_upload_progress(
    loader: PP_Resource,
    bytes_sent: *mut i64,
    total_bytes_to_be_sent: *mut i64,
) -> PP_Bool {
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };
    host.resources
        .with_downcast::<URLLoaderResource, _>(loader, |ul| {
            if !ul.record_upload_progress {
                if !bytes_sent.is_null() {
                    *bytes_sent = 0;
                }
                if !total_bytes_to_be_sent.is_null() {
                    *total_bytes_to_be_sent = 0;
                }
                return PP_FALSE;
            }
            if !bytes_sent.is_null() {
                *bytes_sent = ul.bytes_sent;
            }
            if !total_bytes_to_be_sent.is_null() {
                *total_bytes_to_be_sent = ul.total_bytes_to_be_sent;
            }
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_download_progress(
    loader: PP_Resource,
    bytes_received: *mut i64,
    total_bytes_to_be_received: *mut i64,
) -> PP_Bool {
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };
    host.resources
        .with_downcast::<URLLoaderResource, _>(loader, |ul| {
            if !ul.record_download_progress {
                if !bytes_received.is_null() {
                    *bytes_received = 0;
                }
                if !total_bytes_to_be_received.is_null() {
                    *total_bytes_to_be_received = 0;
                }
                return PP_FALSE;
            }
            if !bytes_received.is_null() {
                *bytes_received = ul.bytes_received;
            }
            if !total_bytes_to_be_received.is_null() {
                *total_bytes_to_be_received = ul.total_bytes_to_be_received;
            }
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_response_info(loader: PP_Resource) -> PP_Resource {
    tracing::trace!("PPB_URLLoader::GetResponseInfo(loader={})", loader);
    let Some(host) = HOST.get() else {
        return 0;
    };
    // Extract the response_info_id WITHOUT calling add_ref while the mutex is
    // held - parking_lot::Mutex is non-reentrant and add_ref would deadlock.
    let resp_id = host
        .resources
        .with_downcast::<URLLoaderResource, _>(loader, |ul| ul.response_info_id)
        .flatten()
        .unwrap_or(0);
    if resp_id != 0 {
        // Increment refcount per PPAPI contract - caller owns a ref.
        host.resources.add_ref(resp_id);
    }
    resp_id
}

unsafe extern "C" fn read_response_body(
    loader: PP_Resource,
    buffer: *mut c_void,
    bytes_to_read: i32,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::trace!(
        "PPB_URLLoader::ReadResponseBody(loader={}, bytes_to_read={})",
        loader,
        bytes_to_read
    );

    if buffer.is_null() || bytes_to_read <= 0 {
        return PP_ERROR_BADARGUMENT;
    }

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    let result = host
        .resources
        .with_downcast_mut::<URLLoaderResource, _>(loader, |ul| {
            // Don't allow a second pending read.
            if ul.pending_callback.is_some() {
                return PP_ERROR_INPROGRESS;
            }

            // Must have a response before reading.
            if ul.response_info_id.is_none() {
                return PP_ERROR_FAILED;
            }

            let requested = bytes_to_read as usize;

            // If we have buffered data, return it immediately.
            if !ul.buffer.is_empty() {
                let copy_len = ul.buffer.len().min(requested);
                let dst = buffer as *mut u8;
                // VecDeque may have two slices.
                let (a, b) = ul.buffer.as_slices();
                let mut offset = 0;
                for slice in [a, b] {
                    let take = slice.len().min(copy_len - offset);
                    if take == 0 {
                        break;
                    }
                    std::ptr::copy_nonoverlapping(slice.as_ptr(), dst.add(offset), take);
                    offset += take;
                }
                ul.buffer.drain(..copy_len);
                return copy_len as i32;
            }

            // Buffer is empty - check if the load is done.
            if let Some(status) = ul.done_status {
                // Load is complete. Return 0 for EOF or the error code.
                return if status == PP_OK { 0 } else { status };
            }

            // Data not yet available - register callback for async wakeup.
            ul.pending_callback = Some(callback);
            ul.user_buffer_ptr = buffer as usize;
            ul.user_buffer_size = requested;
            PP_OK_COMPLETIONPENDING
        });

    result.unwrap_or(PP_ERROR_BADRESOURCE)
}

unsafe extern "C" fn finish_streaming_to_file(
    loader: PP_Resource,
    _callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!(
        "PPB_URLLoader::FinishStreamingToFile(loader={}) - not supported",
        loader
    );
    // Chrome returns PP_ERROR_NOTSUPPORTED here.
    PP_ERROR_NOTSUPPORTED
}

unsafe extern "C" fn close(loader: PP_Resource) {
    tracing::debug!("PPB_URLLoader::Close(loader={})", loader);

    let Some(host) = HOST.get() else {
        return;
    };

    host.resources
        .with_downcast_mut::<URLLoaderResource, _>(loader, |ul| {
            // Signal the background streaming task to abort.
            ul.abort_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
            ul.mode = Mode::LoadComplete;
            ul.done_status = Some(PP_ERROR_ABORTED);

            // If there's a pending callback, fire it with ABORTED.
            if let Some(cb) = ul.pending_callback.take() {
                ul.user_buffer_ptr = 0;
                ul.user_buffer_size = 0;
                if let Some(poster) = get_main_poster() {
                    post_callback(&poster, cb, PP_ERROR_ABORTED);
                }
            }

            ul.buffer.clear();
        });
}

// ===========================================================================
// PPB_URLLoaderTrusted;0.3 extern "C" functions
// ===========================================================================

unsafe extern "C" fn grant_universal_access(loader: PP_Resource) {
    tracing::debug!(
        "PPB_URLLoaderTrusted::GrantUniversalAccess(loader={})",
        loader
    );
    let Some(host) = HOST.get() else {
        return;
    };
    // In our host, we always grant it - we trust the plugin (Flash).
    host.resources
        .with_downcast_mut::<URLLoaderResource, _>(loader, |ul| {
            ul.has_universal_access = true;
        });
}

unsafe extern "C" fn register_status_callback(
    loader: PP_Resource,
    cb: PP_URLLoaderTrusted_StatusCallback,
) {
    tracing::debug!(
        "PPB_URLLoaderTrusted::RegisterStatusCallback(loader={}, cb={:?})",
        loader,
        cb.map(|f| f as usize)
    );
    let Some(host) = HOST.get() else {
        return;
    };
    host.resources
        .with_downcast_mut::<URLLoaderResource, _>(loader, |ul| {
            ul.status_callback = cb;
        });
}
