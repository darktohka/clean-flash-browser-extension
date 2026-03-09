//! PPB_URLLoader;1.0 implementation — chunked async download/upload.
//!
//! `Open()` spawns a background I/O task (via tokio) that calls
//! [`HostCallbacks::on_url_open`] and then streams the response body
//! through a shared ring-buffer.  `ReadResponseBody()` serves data from
//! that buffer with proper PPAPI completion-callback semantics.
//!
//! Upload progress is tracked by bytes delivered to the HTTP request body.
//! Download progress is tracked by bytes streamed into the buffer.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::io::{Cursor, Read};
use std::sync::{Arc, OnceLock};

use parking_lot::{Condvar, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::super::HOST;

// ---------------------------------------------------------------------------
// Pending read request — stored when ReadResponseBody has no data yet
// ---------------------------------------------------------------------------

struct PendingRead {
    buffer: *mut u8,
    bytes_to_read: usize,
    callback: PP_CompletionCallback,
}

// SAFETY: `PendingRead` is created on the main (plugin) thread and consumed
// on the background I/O thread.  The raw `buffer` pointer is provided by the
// plugin and guaranteed valid until the completion callback fires.  We write
// to the buffer *before* posting / running the callback, so:
//   1. There is no data race: only the I/O thread touches the pointer, and
//      the plugin cannot re-use the buffer until it receives the callback.
//   2. The pointer is never read — only written to via copy_nonoverlapping.
// `Sync` is technically not required (the pointer is never shared between
// two threads simultaneously), but `PendingRead` lives inside an
// `Arc<Mutex<…>>` which requires `Send + Sync` on the inner type.
unsafe impl Send for PendingRead {}
unsafe impl Sync for PendingRead {}

// ---------------------------------------------------------------------------
// Concurrency limiter — caps the number of simultaneous URL loader I/O tasks
// ---------------------------------------------------------------------------

/// Maximum number of concurrent URL loader I/O operations.
const MAX_CONCURRENT_LOADS: usize = 8;

fn loader_concurrency_limiter() -> &'static Arc<Semaphore> {
    static LIMITER: OnceLock<Arc<Semaphore>> = OnceLock::new();
    LIMITER.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_LOADS)))
}

async fn acquire_loader_permit() -> Result<OwnedSemaphorePermit, i32> {
    loader_concurrency_limiter()
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| PP_ERROR_FAILED)
}

const URL_FILE_CACHE_MAX_ENTRIES: usize = 128;
const URL_FILE_CACHE_MAX_BYTES: usize = 128 * 1024 * 1024;
const URL_FILE_CACHE_MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
struct CachedResponseEntry {
    status_code: u16,
    status_line: String,
    headers: String,
    content_length: Option<i64>,
    body: Arc<[u8]>,
}

impl CachedResponseEntry {
    fn byte_len(&self) -> usize {
        self.body.len()
    }

    fn into_url_response(&self) -> crate::UrlLoadResponse {
        crate::UrlLoadResponse {
            status_code: self.status_code,
            status_line: self.status_line.clone(),
            headers: self.headers.clone(),
            content_length: self.content_length,
            body: Box::new(Cursor::new(self.body.clone())),
        }
    }
}

#[derive(Default)]
struct UrlFileCache {
    entries: HashMap<String, CachedResponseEntry>,
    order: VecDeque<String>,
    total_bytes: usize,
}

impl UrlFileCache {
    fn get(&mut self, key: &str) -> Option<CachedResponseEntry> {
        let entry = self.entries.get(key).cloned();
        if entry.is_some() {
            self.touch(key);
        }
        entry
    }

    fn insert(&mut self, key: String, entry: CachedResponseEntry) {
        let entry_bytes = entry.byte_len();

        if entry_bytes > URL_FILE_CACHE_MAX_ENTRY_BYTES {
            return;
        }

        if let Some(existing) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(existing.byte_len());
            self.order.retain(|k| k != &key);
        }

        self.total_bytes += entry_bytes;
        self.entries.insert(key.clone(), entry);
        self.order.push_back(key);

        while self.entries.len() > URL_FILE_CACHE_MAX_ENTRIES
            || self.total_bytes > URL_FILE_CACHE_MAX_BYTES
        {
            let Some(oldest_key) = self.order.pop_front() else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&oldest_key) {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.byte_len());
            }
        }
    }

    fn touch(&mut self, key: &str) {
        self.order.retain(|k| k != key);
        self.order.push_back(key.to_string());
    }
}

fn url_file_cache() -> &'static Mutex<UrlFileCache> {
    static URL_FILE_CACHE: OnceLock<Mutex<UrlFileCache>> = OnceLock::new();
    URL_FILE_CACHE.get_or_init(|| Mutex::new(UrlFileCache::default()))
}

#[derive(Clone)]
struct CacheStorePlan {
    key: String,
    status_code: u16,
    status_line: String,
    headers: String,
    content_length: Option<i64>,
}

fn method_is_get(method: &str) -> bool {
    method.eq_ignore_ascii_case("GET")
}

fn request_cache_key(method: &str, headers: &str, body: Option<&[u8]>, url: &str) -> Option<String> {
    if !method_is_get(method) {
        return None;
    }

    if body.is_some_and(|b| !b.is_empty()) {
        return None;
    }

    if extract_header_value(headers, "Range").is_some() {
        return None;
    }

    Some(url.to_string())
}

fn url_has_file_extension(url: &str) -> bool {
    let cutoff = url.find(['?', '#']).unwrap_or(url.len());
    let without_query = &url[..cutoff];

    let path = if let Some(scheme_pos) = without_query.find("://") {
        let after_scheme = &without_query[(scheme_pos + 3)..];
        match after_scheme.find('/') {
            Some(path_pos) => &after_scheme[path_pos..],
            None => "",
        }
    } else {
        without_query
    };

    let Some(last_segment) = path.rsplit('/').next() else {
        return false;
    };
    if last_segment.is_empty() {
        return false;
    }

    let Some((_, ext)) = last_segment.rsplit_once('.') else {
        return false;
    };

    let ext = ext.trim();
    if ext.is_empty() || ext.len() > 8 {
        return false;
    }

    ext.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn content_type_looks_file_like(headers: &str) -> bool {
    let Some(value) = extract_header_value(headers, "Content-Type") else {
        return false;
    };

    let mime = value
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();

    if mime.is_empty() {
        return false;
    }

    if mime.starts_with("image/")
        || mime.starts_with("audio/")
        || mime.starts_with("video/")
        || mime.starts_with("font/")
    {
        return true;
    }

    matches!(
        mime.as_str(),
        "application/x-shockwave-flash"
            | "application/octet-stream"
            | "application/pdf"
            | "application/wasm"
            | "application/javascript"
            | "text/javascript"
            | "text/css"
    )
}

fn response_looks_file_like(url: &str, headers: &str) -> bool {
    url_has_file_extension(url) || content_type_looks_file_like(headers)
}

fn try_get_cached_response(cache_key: &str) -> Option<crate::UrlLoadResponse> {
    let entry = url_file_cache().lock().get(cache_key)?;
    tracing::debug!("PPB_URLLoader cache hit: {}", cache_key);
    Some(entry.into_url_response())
}

fn maybe_store_cached_response(plan: CacheStorePlan, body: Vec<u8>) {
    let entry = CachedResponseEntry {
        status_code: plan.status_code,
        status_line: plan.status_line,
        headers: plan.headers,
        content_length: plan.content_length,
        body: Arc::from(body),
    };

    url_file_cache().lock().insert(plan.key.clone(), entry);
    tracing::debug!("PPB_URLLoader cache store: {}", plan.key);
}

fn map_stream_read_error_to_pp(error: &std::io::Error) -> i32 {
    use std::io::ErrorKind;

    match error.kind() {
        ErrorKind::TimedOut => PP_ERROR_CONNECTION_TIMEDOUT,
        ErrorKind::ConnectionRefused => PP_ERROR_CONNECTION_REFUSED,
        ErrorKind::ConnectionReset => PP_ERROR_CONNECTION_RESET,
        ErrorKind::ConnectionAborted => PP_ERROR_CONNECTION_ABORTED,
        ErrorKind::NotConnected | ErrorKind::BrokenPipe | ErrorKind::UnexpectedEof => {
            PP_ERROR_CONNECTION_CLOSED
        }
        ErrorKind::AddrInUse => PP_ERROR_ADDRESS_IN_USE,
        ErrorKind::AddrNotAvailable => PP_ERROR_ADDRESS_INVALID,
        ErrorKind::NotFound => PP_ERROR_FILENOTFOUND,
        ErrorKind::PermissionDenied => PP_ERROR_NOACCESS,
        _ => PP_ERROR_FAILED,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoaderMode {
    WaitingToOpen,
    Opening,
    StreamingData,
    LoadComplete,
}

fn complete_sync_result(
    host: &crate::HostState,
    callback: PP_CompletionCallback,
    result: i32,
) -> i32 {
    if callback.is_null() || callback.flags == PP_COMPLETIONCALLBACK_FLAG_OPTIONAL {
        return result;
    }

    if let Some(poster) = &*host.main_loop_poster.lock() {
        poster.post_work(callback, 0, result);
    } else {
        unsafe { callback.run(result) };
    }
    PP_OK_COMPLETIONPENDING
}

fn post_completion(
    poster: Option<&crate::message_loop::MessageLoopPoster>,
    callback: PP_CompletionCallback,
    result: i32,
) {
    if let Some(p) = poster {
        p.post_work(callback, 0, result);
    } else {
        unsafe { callback.run(result) };
    }
}

#[inline]
fn is_redirect_status(status_code: i32) -> bool {
    (300..=399).contains(&status_code)
}

fn extract_header_value(headers: &str, name: &str) -> Option<String> {
    for raw_line in headers.split('\n') {
        let line = raw_line.trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case(name) {
            let v = value.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn status_text_from_line(status_line: &str, status_code: i32) -> String {
    let s = status_line.trim();
    if s.is_empty() {
        return status_code.to_string();
    }

    if let Some(pos) = s.find(status_code.to_string().as_str()) {
        let tail = s[pos + 3..].trim();
        if !tail.is_empty() {
            return tail.to_string();
        }
    }

    s.to_string()
}

enum OpenRequestOutcome {
    Streaming {
        response: crate::UrlLoadResponse,
        cache_plan: Option<CacheStorePlan>,
    },
    RedirectPending,
}

fn set_loader_response_info(
    host: &crate::HostState,
    resource_id: PP_Resource,
    instance_id: PP_Instance,
    response_info: super::url_response_info::URLResponseInfoResource,
    pending_redirect_url: Option<String>,
    mode: LoaderMode,
) {
    let ri_id = host.resources.insert(instance_id, Box::new(response_info));
    let old_id = host
        .resources
        .with_downcast_mut::<URLLoaderResource, _>(resource_id, |l| {
            let old = l.response_info_id.replace(ri_id);
            l.pending_redirect_url = pending_redirect_url;
            l.mode = mode;
            old
        })
        .flatten();
    if let Some(old_id) = old_id {
        host.resources.release(old_id);
    }
}

async fn try_open_request(
    host_cbs: Option<std::sync::Arc<dyn crate::HostCallbacks>>,
    inner_arc: Arc<Mutex<URLLoaderInner>>,
    read_ready_cv: Arc<Condvar>,
    resource_id: PP_Resource,
    instance_id: PP_Instance,
    url: String,
    method: String,
    headers: String,
    body: Option<Vec<u8>>,
    follow_redirects: bool,
) -> Result<OpenRequestOutcome, i32> {
    let host = HOST.get().ok_or(PP_ERROR_FAILED)?;

    let cache_key = request_cache_key(&method, &headers, body.as_deref(), &url);

    let (mut response, response_from_cache) =
        if let Some(cached_response) = cache_key.as_deref().and_then(try_get_cached_response) {
        (cached_response, true)
    } else {
        let result = if let Some(hcb) = host_cbs {
            let open_url = url.clone();
            let open_method = method.clone();
            let open_headers = headers.clone();
            let open_body = body.clone();
            match tokio::task::spawn_blocking(move || {
                hcb.on_url_open(
                    &open_url,
                    &open_method,
                    &open_headers,
                    open_body.as_deref(),
                    follow_redirects,
                )
            })
            .await
            {
                Ok(result) => result,
                Err(join_error) => {
                    tracing::error!(
                        "PPB_URLLoader: loader={} open task failed: {}",
                        resource_id,
                        join_error
                    );
                    Err(PP_ERROR_FAILED)
                }
            }
        } else {
            Err(PP_ERROR_FAILED)
        };

        match result {
            Ok(response) => (response, false),
            Err(error_code) => {
                {
                    let mut inner = inner_arc.lock();
                    inner.open_complete = true;
                    inner.finished = true;
                    inner.error = Some(error_code);
                }
                read_ready_cv.notify_all();

                host.resources
                    .with_downcast_mut::<URLLoaderResource, _>(resource_id, |l| {
                        l.pending_redirect_url = None;
                        l.mode = LoaderMode::LoadComplete;
                    });

                tracing::warn!(
                    "PPB_URLLoader: loader={} request failed with {}",
                    resource_id,
                    error_code
                );

                return Err(error_code);
            }
        }
        };

    // Chromium-compatible behavior: any valid HTTP response,
    // including non-2xx (e.g. 404/500), completes Open successfully.
    // Only transport/policy failures should use PP_ERROR_* paths.
    let redirect_url = if !follow_redirects && is_redirect_status(response.status_code as i32) {
        extract_header_value(&response.headers, "Location")
    } else {
        None
    };

    let upload_status = {
        let mut inner = inner_arc.lock();
        inner.open_complete = true;
        inner.total_bytes = if redirect_url.is_some() {
            0
        } else {
            response.content_length.unwrap_or(-1)
        };
        inner.bytes_sent = inner.total_bytes_to_send;
        inner.error = None;
        inner.finished = false;

        inner.status_callback.map(|cb| {
            let bytes_sent = if inner.record_upload_progress {
                inner.bytes_sent
            } else {
                -1
            };
            let total_to_send = if inner.record_upload_progress {
                inner.total_bytes_to_send
            } else {
                -1
            };
            let bytes_received = if inner.record_download_progress {
                inner.bytes_received
            } else {
                -1
            };
            let total_bytes = if inner.record_download_progress {
                inner.total_bytes
            } else {
                -1
            };
            (
                cb,
                bytes_sent,
                total_to_send,
                bytes_received,
                total_bytes,
            )
        })
    };

    if let Some((cb, bytes_sent, total_to_send, bytes_received, total_bytes)) = upload_status {
        unsafe {
            cb(
                instance_id,
                resource_id,
                bytes_sent,
                total_to_send,
                bytes_received,
                total_bytes,
            );
        }
    }

    let response_headers = std::mem::take(&mut response.headers);
    let cache_plan = if response_from_cache || redirect_url.is_some() {
        None
    } else if response.status_code >= 200
        && response.status_code < 300
        && response_looks_file_like(&url, &response_headers)
    {
        cache_key.map(|key| CacheStorePlan {
            key,
            status_code: response.status_code,
            status_line: response.status_line.clone(),
            headers: response_headers.clone(),
            content_length: response.content_length,
        })
    } else {
        None
    };

    let status_text = status_text_from_line(&response.status_line, response.status_code as i32);
    let response_info = super::url_response_info::URLResponseInfoResource {
        url: url.clone(),
        status_code: response.status_code as i32,
        status_line: status_text,
        headers: response_headers,
        redirect_url: redirect_url.clone().unwrap_or_default(),
    };

    if let Some(redirect_url) = redirect_url {
        set_loader_response_info(
            host,
            resource_id,
            instance_id,
            response_info,
            Some(redirect_url.clone()),
            LoaderMode::Opening,
        );

        tracing::debug!(
            "PPB_URLLoader: loader={} deferred redirect to {}",
            resource_id,
            redirect_url
        );

        read_ready_cv.notify_all();

        return Ok(OpenRequestOutcome::RedirectPending);
    }

    set_loader_response_info(
        host,
        resource_id,
        instance_id,
        response_info,
        None,
        LoaderMode::StreamingData,
    );

    tracing::debug!(
        "PPB_URLLoader: loader={} headers received, status={}, content_length={:?}, from_cache={}",
        resource_id,
        response.status_code,
        response.content_length,
        response_from_cache
    );

    read_ready_cv.notify_all();

    Ok(OpenRequestOutcome::Streaming {
        response,
        cache_plan,
    })
}

// ---------------------------------------------------------------------------
// Shared streaming state between main thread and background I/O task
// ---------------------------------------------------------------------------

pub struct URLLoaderInner {
    /// Buffered response body data received from the network / filesystem.
    buffer: VecDeque<u8>,

    /// Total bytes received so far.
    pub bytes_received: i64,
    /// Total expected bytes (−1 when unknown / chunked transfer).
    pub total_bytes: i64,

    /// Bytes of request body sent.
    pub bytes_sent: i64,
    /// Total request body size.
    pub total_bytes_to_send: i64,

    /// All response body data has been received (EOF or error).
    pub finished: bool,
    /// `Open` has completed (headers are available).
    pub open_complete: bool,
    /// Error code if the request failed (`None` = no error).
    pub error: Option<i32>,

    /// A single pending `ReadResponseBody` waiting for data.
    pending_read: Option<PendingRead>,

    /// Whether download progress should be tracked.
    pub record_download_progress: bool,
    /// Whether upload progress should be tracked.
    pub record_upload_progress: bool,

    /// Whether redirects should be followed automatically.
    pub follow_redirects: bool,

    /// The URL being loaded.
    pub url: Option<String>,

    /// Trusted status callback for reporting upload/download progress.
    pub status_callback: PP_URLLoaderTrusted_StatusCallback,
}

// ---------------------------------------------------------------------------
// URLLoader resource
// ---------------------------------------------------------------------------

/// PPB_URLLoader resource — one per `Create()` call.
pub struct URLLoaderResource {
    pub instance: PP_Instance,
    /// ID of the associated `URLResponseInfo` resource (set after Open).
    pub response_info_id: Option<PP_Resource>,
    /// Original request method used for redirect continuation.
    request_method: String,
    /// Original request headers used for redirect continuation.
    request_headers: String,
    /// Original request body used for redirect continuation.
    request_body: Option<Vec<u8>>,
    /// Redirect URL waiting for `FollowRedirect()`.
    pending_redirect_url: Option<String>,
    /// Lifecycle mode for Open/FollowRedirect semantics.
    mode: LoaderMode,
    /// Shared state for streaming I/O.
    pub inner: Arc<Mutex<URLLoaderInner>>,
    /// Notifies blocking readers when body state changes.
    read_ready_cv: Arc<Condvar>,
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

// ---------------------------------------------------------------------------
// Vtable
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

// PPB_URLLoaderTrusted;0.3 stub
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
// Create / IsURLLoader
// ---------------------------------------------------------------------------

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    tracing::trace!("PPB_URLLoader::Create(instance={})", instance);
    let Some(host) = HOST.get() else { return 0 };

    let inner = URLLoaderInner {
        buffer: VecDeque::new(),
        bytes_received: 0,
        total_bytes: -1,
        bytes_sent: 0,
        total_bytes_to_send: 0,
        finished: false,
        open_complete: false,
        error: None,
        pending_read: None,
        record_download_progress: false,
        record_upload_progress: false,
        follow_redirects: true,
        url: None,
        status_callback: None,
    };
    let loader = URLLoaderResource {
        instance,
        response_info_id: None,
        request_method: "GET".to_string(),
        request_headers: String::new(),
        request_body: None,
        pending_redirect_url: None,
        mode: LoaderMode::WaitingToOpen,
        inner: Arc::new(Mutex::new(inner)),
        read_ready_cv: Arc::new(Condvar::new()),
    };
    let id = host.resources.insert(instance, Box::new(loader));
    tracing::debug!(
        "PPB_URLLoader::Create(instance={}) -> resource={}",
        instance,
        id
    );
    id
}

unsafe extern "C" fn is_url_loader(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_URLLoader")))
        .unwrap_or(PP_FALSE)
}

// ---------------------------------------------------------------------------
// Open — spawn background I/O task for async download + upload
// ---------------------------------------------------------------------------

/// Chunk size for reading the response body from the network.
const STREAM_CHUNK_SIZE: usize = 64 * 1024; // 64 KiB

fn stream_body_blocking(
    mut response: crate::UrlLoadResponse,
    inner_arc: Arc<Mutex<URLLoaderInner>>,
    read_ready_cv: Arc<Condvar>,
    poster: Option<crate::message_loop::MessageLoopPoster>,
    resource_id: PP_Resource,
    instance_id: PP_Instance,
    cache_plan: Option<CacheStorePlan>,
) {
    let mut cache_plan = cache_plan;
    let mut cache_buffer = cache_plan.as_ref().map(|_| Vec::new());
    let mut chunk_buf = vec![0u8; STREAM_CHUNK_SIZE];
    loop {
        // Stop streaming immediately after Close() aborts the loader.
        {
            let inner = inner_arc.lock();
            if inner.finished && inner.error == Some(PP_ERROR_ABORTED) {
                if let Some(host) = HOST.get() {
                    host.resources
                        .with_downcast_mut::<URLLoaderResource, _>(resource_id, |l| {
                            l.mode = LoaderMode::LoadComplete;
                        });
                }
                read_ready_cv.notify_all();
                break;
            }
        }

        let n = match response.body.read(&mut chunk_buf) {
            Ok(0) => {
                let mut inner = inner_arc.lock();
                inner.finished = true;
                tracing::debug!(
                    "PPB_URLLoader: loader={} download complete, total {} bytes",
                    resource_id,
                    inner.bytes_received
                );
                if let Some(pending) = inner.pending_read.take() {
                    drop(inner);
                    post_completion(poster.as_ref(), pending.callback, 0);
                }
                if let Some(host) = HOST.get() {
                    host.resources
                        .with_downcast_mut::<URLLoaderResource, _>(resource_id, |l| {
                            l.mode = LoaderMode::LoadComplete;
                        });
                }

                if let (Some(plan), Some(body)) = (cache_plan.take(), cache_buffer.take()) {
                    maybe_store_cached_response(plan, body);
                }

                read_ready_cv.notify_all();
                break;
            }
            Ok(n) => n,
            Err(e) => {
                let mapped_error = map_stream_read_error_to_pp(&e);
                tracing::warn!(
                    "PPB_URLLoader: loader={} read error: {} (pp_error={})",
                    resource_id,
                    e,
                    mapped_error
                );
                let mut inner = inner_arc.lock();
                inner.finished = true;
                inner.error = Some(mapped_error);
                if let Some(pending) = inner.pending_read.take() {
                    drop(inner);
                    post_completion(poster.as_ref(), pending.callback, mapped_error);
                }
                if let Some(host) = HOST.get() {
                    host.resources
                        .with_downcast_mut::<URLLoaderResource, _>(resource_id, |l| {
                            l.mode = LoaderMode::LoadComplete;
                        });
                }

                read_ready_cv.notify_all();
                break;
            }
        };

        if let Some(buf) = cache_buffer.as_mut() {
            if buf.len().saturating_add(n) <= URL_FILE_CACHE_MAX_ENTRY_BYTES {
                buf.extend_from_slice(&chunk_buf[..n]);
            } else {
                tracing::debug!(
                    "PPB_URLLoader cache skip: loader={} body exceeded {} bytes",
                    resource_id,
                    URL_FILE_CACHE_MAX_ENTRY_BYTES
                );
                cache_plan = None;
                cache_buffer = None;
            }
        }

        let (status_update, pending_completion) = {
            let mut inner = inner_arc.lock();
            inner.bytes_received += n as i64;

            // Capture status callback payload and run it after unlocking.
            let status = inner.status_callback.map(|cb| {
                let bytes_sent = if inner.record_upload_progress {
                    inner.bytes_sent
                } else {
                    -1
                };
                let total_to_send = if inner.record_upload_progress {
                    inner.total_bytes_to_send
                } else {
                    -1
                };
                let bytes_received = if inner.record_download_progress {
                    inner.bytes_received
                } else {
                    -1
                };
                let total_bytes = if inner.record_download_progress {
                    inner.total_bytes
                } else {
                    -1
                };
                (
                    cb,
                    bytes_sent,
                    total_to_send,
                    bytes_received,
                    total_bytes,
                )
            });

            let pending = if let Some(pending) = inner.pending_read.take() {
                let to_copy = n.min(pending.bytes_to_read);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        chunk_buf.as_ptr(),
                        pending.buffer,
                        to_copy,
                    );
                }
                if n > to_copy {
                    inner.buffer.extend(&chunk_buf[to_copy..n]);
                }
                Some((pending.callback, to_copy as i32))
            } else {
                inner.buffer.extend(&chunk_buf[..n]);
                None
            };

            (status, pending)
        };

        if let Some((cb, bytes_sent, total_to_send, bytes_received, total_bytes)) = status_update {
            unsafe {
                cb(
                    instance_id,
                    resource_id,
                    bytes_sent,
                    total_to_send,
                    bytes_received,
                    total_bytes,
                );
            }
        }

        if let Some((pending_cb, completion_code)) = pending_completion {
            post_completion(poster.as_ref(), pending_cb, completion_code);
        }

        read_ready_cv.notify_all();
    }
}

async fn stream_body_async(
    response: crate::UrlLoadResponse,
    inner_arc: Arc<Mutex<URLLoaderInner>>,
    read_ready_cv: Arc<Condvar>,
    poster: Option<crate::message_loop::MessageLoopPoster>,
    resource_id: PP_Resource,
    instance_id: PP_Instance,
    cache_plan: Option<CacheStorePlan>,
) {
    let inner_for_error = inner_arc.clone();
    let cv_for_error = read_ready_cv.clone();
    let poster_for_error = poster.clone();

    let join_result = tokio::task::spawn_blocking(move || {
        stream_body_blocking(
            response,
            inner_arc,
            read_ready_cv,
            poster,
            resource_id,
            instance_id,
            cache_plan,
        );
    })
    .await;

    if let Err(join_error) = join_result {
        tracing::error!(
            "PPB_URLLoader: loader={} stream task failed: {}",
            resource_id,
            join_error
        );

        let (pending_callback, error_code) = {
            let mut inner = inner_for_error.lock();
            if inner.finished {
                return;
            }
            inner.finished = true;
            let error_code = inner.error.unwrap_or(PP_ERROR_FAILED);
            if inner.error.is_none() {
                inner.error = Some(error_code);
            }
            (inner.pending_read.take().map(|pending| pending.callback), error_code)
        };

        if let Some(pending_cb) = pending_callback {
            post_completion(poster_for_error.as_ref(), pending_cb, error_code);
        }

        if let Some(host) = HOST.get() {
            host.resources
                .with_downcast_mut::<URLLoaderResource, _>(resource_id, |l| {
                    l.mode = LoaderMode::LoadComplete;
                });
        }

        cv_for_error.notify_all();
    }
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
        callback
    );
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    // --- Extract request parameters from URLRequestInfoResource ----------
    let req_data = host
        .resources
        .with_downcast::<super::url_request_info::URLRequestInfoResource, _>(
            request_info,
            |req| {
                (
                    req.url.clone().unwrap_or_default(),
                    req.method.clone().unwrap_or_else(|| "GET".to_string()),
                    req.headers.clone().unwrap_or_default(),
                    if req.body.is_empty() {
                        None
                    } else {
                        Some(req.body.clone())
                    },
                    req.follow_redirects,
                    req.record_download_progress,
                    req.record_upload_progress,
                    req.prefetch_buffer_lower_threshold,
                    req.prefetch_buffer_upper_threshold,
                )
            },
        );

    let Some((url, method, headers, body, follow_redirects, record_dl, record_ul, prefetch_lower, prefetch_upper)) =
        req_data
    else {
        tracing::warn!(
            "PPB_URLLoader::Open: bad request_info resource {}",
            request_info
        );
        return complete_sync_result(host, callback, PP_ERROR_BADARGUMENT);
    };

    if prefetch_lower < 0 || prefetch_upper < 0 || prefetch_upper <= prefetch_lower {
        tracing::warn!(
            "PPB_URLLoader::Open: invalid prefetch thresholds lower={} upper={}",
            prefetch_lower,
            prefetch_upper
        );
        return complete_sync_result(host, callback, PP_ERROR_FAILED);
    }

    tracing::info!(
        "PPB_URLLoader::Open: loader={} url={:?} method={}",
        loader,
        url,
        method
    );

    // --- Configure the loader's inner state ------------------------------
    let prep = host
        .resources
        .with_downcast_mut::<URLLoaderResource, _>(loader, |l| {
            if l.mode != LoaderMode::WaitingToOpen {
                return Err(PP_ERROR_INPROGRESS);
            }
            l.mode = LoaderMode::Opening;
            l.request_method = method.clone();
            l.request_headers = headers.clone();
            l.request_body = body.clone();
            l.pending_redirect_url = None;

            {
                let mut inner = l.inner.lock();
                inner.url = Some(url.clone());
                inner.follow_redirects = follow_redirects;
                inner.record_download_progress = record_dl;
                inner.record_upload_progress = record_ul;
                inner.buffer.clear();
                inner.bytes_received = 0;
                inner.total_bytes = -1;
                inner.bytes_sent = 0;
                inner.total_bytes_to_send = body.as_ref().map(|b| b.len() as i64).unwrap_or(0);
                inner.finished = false;
                inner.open_complete = false;
                inner.error = None;
                inner.pending_read = None;
            }
            Ok((l.inner.clone(), l.read_ready_cv.clone()))
        });

    let (inner_arc, read_ready_cv) = match prep {
        None => return complete_sync_result(host, callback, PP_ERROR_BADRESOURCE),
        Some(Err(code)) => return complete_sync_result(host, callback, code),
        Some(Ok(data)) => data,
    };

    let loader_instance = host
        .resources
        .with_downcast::<URLLoaderResource, _>(loader, |l| l.instance)
        .unwrap_or(0);

    // Clone the poster and the Arc<HostCallbacks> so the background task
    // does NOT hold the host_callbacks mutex during long-running I/O.
    let poster = host.main_loop_poster.lock().clone();
    let host_cbs: Option<std::sync::Arc<dyn crate::HostCallbacks>> =
        host.host_callbacks.lock().clone();

    // --- Blocking vs async Open ------------------------------------------
    // PP_BlockUntilComplete (null callback) means "run synchronously and
    // return the result directly". For non-null callbacks, Open is completed
    // asynchronously and all blocking I/O stays on Tokio's blocking pool.
    let is_blocking = callback.is_null();
    let cb = callback;
    let resource_id = loader;
    let instance_id = loader_instance;

    if is_blocking {
        let permit = match crate::tokio_runtime().block_on(acquire_loader_permit()) {
            Ok(permit) => permit,
            Err(error_code) => return error_code,
        };

        match crate::tokio_runtime().block_on(try_open_request(
            host_cbs,
            inner_arc.clone(),
            read_ready_cv.clone(),
            resource_id,
            instance_id,
            url,
            method,
            headers,
            body,
            follow_redirects,
        )) {
            Ok(OpenRequestOutcome::Streaming {
                response,
                cache_plan,
            }) => {
                crate::tokio_runtime().spawn(async move {
                    let _permit = permit;
                    stream_body_async(
                        response,
                        inner_arc,
                        read_ready_cv,
                        poster,
                        resource_id,
                        instance_id,
                        cache_plan,
                    )
                    .await;
                });
                PP_OK
            }
            Ok(OpenRequestOutcome::RedirectPending) => PP_OK,
            Err(error_code) => error_code,
        }
    } else {
        // ----- Async Open (spawn background I/O task) -----
        crate::tokio_runtime().spawn(async move {
            let permit = match acquire_loader_permit().await {
                Ok(permit) => permit,
                Err(error_code) => {
                    post_completion(poster.as_ref(), cb, error_code);
                    return;
                }
            };
            let _permit = permit;

            match try_open_request(
                host_cbs,
                inner_arc.clone(),
                read_ready_cv.clone(),
                resource_id,
                instance_id,
                url,
                method,
                headers,
                body,
                follow_redirects,
            )
            .await
            {
                Ok(OpenRequestOutcome::Streaming {
                    response,
                    cache_plan,
                }) => {
                    post_completion(poster.as_ref(), cb, PP_OK);
                    stream_body_async(
                        response,
                        inner_arc,
                        read_ready_cv,
                        poster,
                        resource_id,
                        instance_id,
                        cache_plan,
                    )
                    .await;
                }
                Ok(OpenRequestOutcome::RedirectPending) => {
                    post_completion(poster.as_ref(), cb, PP_OK);
                }
                Err(error_code) => {
                    post_completion(poster.as_ref(), cb, error_code);
                }
            }
        });

        PP_OK_COMPLETIONPENDING
    }
}

// ---------------------------------------------------------------------------
// FollowRedirect
// ---------------------------------------------------------------------------

unsafe extern "C" fn follow_redirect(
    loader: PP_Resource,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("PPB_URLLoader::FollowRedirect(loader={})", loader);
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    let prep = host
        .resources
        .with_downcast_mut::<URLLoaderResource, _>(loader, |l| {
            if l.mode != LoaderMode::Opening {
                return Err(PP_ERROR_INPROGRESS);
            }

            let Some(next_url) = l.pending_redirect_url.take() else {
                return Err(PP_ERROR_FAILED);
            };

            let mut inner = l.inner.lock();
            let follow_redirects = inner.follow_redirects;
            inner.url = Some(next_url.clone());
            inner.open_complete = false;
            inner.finished = false;
            inner.error = None;
            inner.total_bytes = -1;
            inner.bytes_received = 0;
            inner.buffer.clear();

            Ok((
                l.instance,
                l.inner.clone(),
                l.read_ready_cv.clone(),
                next_url,
                l.request_method.clone(),
                l.request_headers.clone(),
                l.request_body.clone(),
                follow_redirects,
            ))
        });

    let (instance_id, inner_arc, read_ready_cv, url, method, headers, body, follow_redirects) = match prep {
        None => return complete_sync_result(host, callback, PP_ERROR_BADRESOURCE),
        Some(Err(code)) => return complete_sync_result(host, callback, code),
        Some(Ok(data)) => data,
    };

    let poster = host.main_loop_poster.lock().clone();
    let host_cbs: Option<std::sync::Arc<dyn crate::HostCallbacks>> =
        host.host_callbacks.lock().clone();

    if callback.is_null() {
        let permit = match crate::tokio_runtime().block_on(acquire_loader_permit()) {
            Ok(permit) => permit,
            Err(error_code) => return error_code,
        };

        match crate::tokio_runtime().block_on(try_open_request(
            host_cbs,
            inner_arc.clone(),
            read_ready_cv.clone(),
            loader,
            instance_id,
            url,
            method,
            headers,
            body,
            follow_redirects,
        )) {
            Ok(OpenRequestOutcome::Streaming {
                response,
                cache_plan,
            }) => {
                crate::tokio_runtime().spawn(async move {
                    let _permit = permit;
                    stream_body_async(
                        response,
                        inner_arc,
                        read_ready_cv,
                        poster,
                        loader,
                        instance_id,
                        cache_plan,
                    )
                    .await;
                });
                PP_OK
            }
            Ok(OpenRequestOutcome::RedirectPending) => PP_OK,
            Err(error_code) => error_code,
        }
    } else {
        let cb = callback;
        crate::tokio_runtime().spawn(async move {
            let permit = match acquire_loader_permit().await {
                Ok(permit) => permit,
                Err(error_code) => {
                    post_completion(poster.as_ref(), cb, error_code);
                    return;
                }
            };
            let _permit = permit;

            match try_open_request(
                host_cbs,
                inner_arc.clone(),
                read_ready_cv.clone(),
                loader,
                instance_id,
                url,
                method,
                headers,
                body,
                follow_redirects,
            )
            .await
            {
                Ok(OpenRequestOutcome::Streaming {
                    response,
                    cache_plan,
                }) => {
                    post_completion(poster.as_ref(), cb, PP_OK);
                    stream_body_async(
                        response,
                        inner_arc,
                        read_ready_cv,
                        poster,
                        loader,
                        instance_id,
                        cache_plan,
                    )
                    .await;
                }
                Ok(OpenRequestOutcome::RedirectPending) => {
                    post_completion(poster.as_ref(), cb, PP_OK);
                }
                Err(error_code) => {
                    post_completion(poster.as_ref(), cb, error_code);
                }
            }
        });

        PP_OK_COMPLETIONPENDING
    }
}

// ---------------------------------------------------------------------------
// Upload / download progress
// ---------------------------------------------------------------------------

unsafe extern "C" fn get_upload_progress(
    loader: PP_Resource,
    bytes_sent: *mut i64,
    total_bytes_to_be_sent: *mut i64,
) -> PP_Bool {
    if !bytes_sent.is_null() {
        unsafe { *bytes_sent = 0 };
    }
    if !total_bytes_to_be_sent.is_null() {
        unsafe { *total_bytes_to_be_sent = 0 };
    }

    let Some(host) = HOST.get() else { return PP_FALSE };

    host.resources
        .with_downcast::<URLLoaderResource, _>(loader, |l| {
            let inner = l.inner.lock();
            if !inner.record_upload_progress {
                return PP_FALSE;
            }
            if !bytes_sent.is_null() {
                unsafe { *bytes_sent = inner.bytes_sent };
            }
            if !total_bytes_to_be_sent.is_null() {
                unsafe { *total_bytes_to_be_sent = inner.total_bytes_to_send };
            }
            tracing::trace!(
                "PPB_URLLoader::GetUploadProgress(loader={}) -> sent={}, total={}",
                loader,
                inner.bytes_sent,
                inner.total_bytes_to_send
            );
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_download_progress(
    loader: PP_Resource,
    bytes_received: *mut i64,
    total_bytes_to_be_received: *mut i64,
) -> PP_Bool {
    if !bytes_received.is_null() {
        unsafe { *bytes_received = 0 };
    }
    if !total_bytes_to_be_received.is_null() {
        unsafe { *total_bytes_to_be_received = 0 };
    }

    let Some(host) = HOST.get() else { return PP_FALSE };

    host.resources
        .with_downcast::<URLLoaderResource, _>(loader, |l| {
            let inner = l.inner.lock();
            if !inner.record_download_progress {
                return PP_FALSE;
            }
            if !bytes_received.is_null() {
                unsafe { *bytes_received = inner.bytes_received };
            }
            if !total_bytes_to_be_received.is_null() {
                unsafe { *total_bytes_to_be_received = inner.total_bytes };
            }
            tracing::trace!(
                "PPB_URLLoader::GetDownloadProgress(loader={}) -> received={}, total={}",
                loader,
                inner.bytes_received,
                inner.total_bytes
            );
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}

// ---------------------------------------------------------------------------
// GetResponseInfo
// ---------------------------------------------------------------------------

unsafe extern "C" fn get_response_info(loader: PP_Resource) -> PP_Resource {
    tracing::trace!(
        "PPB_URLLoader::GetResponseInfo(loader={})",
        loader
    );
    let Some(host) = HOST.get() else { return 0 };

    let existing = host
        .resources
        .with_downcast::<URLLoaderResource, _>(loader, |l| l.response_info_id)
        .unwrap_or(None);

    if let Some(id) = existing {
        // Return a new reference to the existing response info.
        host.resources.add_ref(id);
        tracing::debug!(
            "PPB_URLLoader::GetResponseInfo(loader={}) -> {} (existing)",
            loader,
            id
        );
        return id;
    }

    // If Open hasn't completed yet there is no response info.
    tracing::debug!(
        "PPB_URLLoader::GetResponseInfo(loader={}) -> 0 (not yet available)",
        loader
    );
    0
}

// ---------------------------------------------------------------------------
// ReadResponseBody — serve from buffer or pend for async delivery
// ---------------------------------------------------------------------------

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
        .with_downcast::<URLLoaderResource, _>(loader, |l| {
            if l.response_info_id.is_none() {
                return PP_ERROR_FAILED;
            }

            let read_ready_cv = l.read_ready_cv.clone();
            let mut inner = l.inner.lock();
            if callback.is_null() {
                loop {
                    let available = inner.buffer.len();

                    if available > 0 {
                        let to_read = (bytes_to_read as usize).min(available);
                        let dst = buffer as *mut u8;

                        let (front, back) = inner.buffer.as_slices();
                        let front_n = front.len().min(to_read);
                        unsafe {
                            std::ptr::copy_nonoverlapping(front.as_ptr(), dst, front_n);
                            if front_n < to_read {
                                std::ptr::copy_nonoverlapping(
                                    back.as_ptr(),
                                    dst.add(front_n),
                                    to_read - front_n,
                                );
                            }
                        }
                        drop(inner.buffer.drain(..to_read));

                        tracing::trace!(
                            "PPB_URLLoader::ReadResponseBody: loader={} blocking served {} bytes",
                            loader,
                            to_read
                        );
                        return to_read as i32;
                    }

                    if inner.finished {
                        if let Some(err) = inner.error {
                            tracing::debug!(
                                "PPB_URLLoader::ReadResponseBody: loader={} blocking error {}",
                                loader,
                                err
                            );
                            return err;
                        }
                        return 0;
                    }

                    if l.pending_redirect_url.is_some() {
                        // Redirect-audit state: there is no body to read
                        // until FollowRedirect resumes loading.
                        return 0;
                    }

                    read_ready_cv.wait(&mut inner);
                }
            }

            let available = inner.buffer.len();

            if available > 0 {
                // --- Data ready — copy into the caller's buffer ---
                let to_read = (bytes_to_read as usize).min(available);
                let dst = buffer as *mut u8;

                // Copy from the two contiguous slices of VecDeque
                // directly, avoiding the O(n) rotation that
                // make_contiguous() may perform.
                let (front, back) = inner.buffer.as_slices();
                let front_n = front.len().min(to_read);
                unsafe {
                    std::ptr::copy_nonoverlapping(front.as_ptr(), dst, front_n);
                    if front_n < to_read {
                        std::ptr::copy_nonoverlapping(
                            back.as_ptr(),
                            dst.add(front_n),
                            to_read - front_n,
                        );
                    }
                }
                drop(inner.buffer.drain(..to_read));

                tracing::trace!(
                    "PPB_URLLoader::ReadResponseBody: loader={} served {} bytes \
                     ({} buffered)",
                    loader,
                    to_read,
                    inner.buffer.len()
                );
                to_read as i32
            } else if inner.finished {
                // --- No data, stream done ---
                if let Some(err) = inner.error {
                    tracing::debug!(
                        "PPB_URLLoader::ReadResponseBody: loader={} -> error {}",
                        loader,
                        err
                    );
                    err
                } else {
                    tracing::debug!(
                        "PPB_URLLoader::ReadResponseBody: loader={} -> EOF (0)",
                        loader
                    );
                    0
                }
            } else {
                // --- No data yet, stream still active — pend the read ---
                if inner.pending_read.is_some() {
                    tracing::warn!(
                        "PPB_URLLoader::ReadResponseBody: loader={} \
                         already has a pending read!",
                        loader
                    );
                    return PP_ERROR_INPROGRESS;
                }
                inner.pending_read = Some(PendingRead {
                    buffer: buffer as *mut u8,
                    bytes_to_read: bytes_to_read as usize,
                    callback,
                });
                tracing::trace!(
                    "PPB_URLLoader::ReadResponseBody: loader={} -> PENDING",
                    loader
                );
                PP_OK_COMPLETIONPENDING
            }
        });

    match result {
        Some(PP_OK_COMPLETIONPENDING) => PP_OK_COMPLETIONPENDING,
        Some(bytes) => {
            // Synchronous completion (data was available or EOF/error).
            if callback.is_null()
                || callback.flags == PP_COMPLETIONCALLBACK_FLAG_OPTIONAL
            {
                // Return the value directly.
                bytes
            } else {
                // Non-optional async callback: post it, return PENDING.
                if let Some(poster) = &*host.main_loop_poster.lock() {
                    poster.post_work(callback, 0, bytes);
                } else {
                    unsafe { callback.run(bytes) };
                }
                PP_OK_COMPLETIONPENDING
            }
        }
        None => complete_sync_result(host, callback, PP_ERROR_BADRESOURCE),
    }
}

// ---------------------------------------------------------------------------
// FinishStreamingToFile / Close
// ---------------------------------------------------------------------------

unsafe extern "C" fn finish_streaming_to_file(
    loader: PP_Resource,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!(
        "PPB_URLLoader::FinishStreamingToFile(loader={})",
        loader
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    let result = if host.resources.is_type(loader, "PPB_URLLoader") {
        PP_ERROR_NOTSUPPORTED
    } else {
        PP_ERROR_BADRESOURCE
    };

    complete_sync_result(host, callback, result)
}

unsafe extern "C" fn close(loader: PP_Resource) {
    tracing::debug!("PPB_URLLoader::Close(loader={})", loader);
    // Mark the stream as finished so the background task stops writing
    // and any pending read gets cancelled.
    let Some(host) = HOST.get() else { return };
    host.resources
        .with_downcast_mut::<URLLoaderResource, _>(loader, |l| {
            let read_ready_cv = l.read_ready_cv.clone();
            l.mode = LoaderMode::LoadComplete;
            l.pending_redirect_url = None;
            let mut inner = l.inner.lock();
            inner.finished = true;
            inner.error = Some(PP_ERROR_ABORTED);
            if let Some(pending) = inner.pending_read.take() {
                drop(inner);
                if let Some(poster) = &*host.main_loop_poster.lock() {
                    poster.post_work(pending.callback, 0, PP_ERROR_ABORTED);
                } else {
                    unsafe { pending.callback.run(PP_ERROR_ABORTED) };
                }
            }
            read_ready_cv.notify_all();
        });
}

// ---------------------------------------------------------------------------
// Trusted interface stubs
// ---------------------------------------------------------------------------

unsafe extern "C" fn grant_universal_access(loader: PP_Resource) {
    tracing::debug!(
        "PPB_URLLoaderTrusted::GrantUniversalAccess(loader={})",
        loader
    );
    // No-op: we always grant access in our standalone projector.
}

unsafe extern "C" fn register_status_callback(
    loader: PP_Resource,
    cb: PP_URLLoaderTrusted_StatusCallback,
) {
    tracing::debug!(
        "PPB_URLLoaderTrusted::RegisterStatusCallback(loader={}, cb={:?})",
        loader,
        cb
    );
    let Some(host) = HOST.get() else { return };
    host.resources
        .with_downcast::<URLLoaderResource, _>(loader, |l| {
            l.inner.lock().status_callback = cb;
        });
}
