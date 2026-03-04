//! PPB_URLLoader;1.0 implementation — chunked async download/upload.
//!
//! `Open()` spawns a background I/O thread that calls
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
use std::collections::VecDeque;
use std::ffi::c_void;
use std::io::Read;
use std::sync::Arc;

use parking_lot::Mutex;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Pending read request — stored when ReadResponseBody has no data yet
// ---------------------------------------------------------------------------

struct PendingRead {
    buffer: *mut u8,
    bytes_to_read: usize,
    callback: PP_CompletionCallback,
}

// Safety: the buffer pointer is provided by the plugin and remains valid
// until the callback fires.  We only write to it *before* posting the
// callback, so the pointer access is safe.
unsafe impl Send for PendingRead {}
unsafe impl Sync for PendingRead {}

// ---------------------------------------------------------------------------
// Shared streaming state between main thread and background I/O thread
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

    /// The URL being loaded.
    pub url: Option<String>,
}

// ---------------------------------------------------------------------------
// URLLoader resource
// ---------------------------------------------------------------------------

/// PPB_URLLoader resource — one per `Create()` call.
pub struct URLLoaderResource {
    pub instance: PP_Instance,
    /// ID of the associated `URLResponseInfo` resource (set after Open).
    pub response_info_id: Option<PP_Resource>,
    /// Shared state for streaming I/O.
    pub inner: Arc<Mutex<URLLoaderInner>>,
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
        url: None,
    };
    let loader = URLLoaderResource {
        instance,
        response_info_id: None,
        inner: Arc::new(Mutex::new(inner)),
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
// Open — spawn background I/O thread for async download + upload
// ---------------------------------------------------------------------------

/// Chunk size for reading the response body from the network.
const STREAM_CHUNK_SIZE: usize = 64 * 1024; // 64 KiB

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
                    req.stream_to_file,
                    req.follow_redirects,
                    req.record_download_progress,
                    req.record_upload_progress,
                )
            },
        );

    let Some((url, method, headers, body, _stream_to_file, _follow_redirects, record_dl, record_ul)) =
        req_data
    else {
        tracing::warn!(
            "PPB_URLLoader::Open: bad request_info resource {}",
            request_info
        );
        return PP_ERROR_BADRESOURCE;
    };

    tracing::info!(
        "PPB_URLLoader::Open: loader={} url={:?} method={}",
        loader,
        url,
        method
    );

    // --- Configure the loader's inner state ------------------------------
    let inner_arc = host
        .resources
        .with_downcast_mut::<URLLoaderResource, _>(loader, |l| {
            {
                let mut inner = l.inner.lock();
                inner.url = Some(url.clone());
                inner.record_download_progress = record_dl;
                inner.record_upload_progress = record_ul;
                if let Some(ref b) = body {
                    inner.total_bytes_to_send = b.len() as i64;
                }
            }
            l.inner.clone()
        });

    let Some(inner_arc) = inner_arc else {
        return PP_ERROR_BADRESOURCE;
    };

    let loader_instance = host
        .resources
        .with_downcast::<URLLoaderResource, _>(loader, |l| l.instance)
        .unwrap_or(0);

    // Clone the poster and the Arc<HostCallbacks> so the background thread
    // does NOT hold the host_callbacks mutex during long-running I/O.
    let poster = host.main_loop_poster.lock().clone();
    let host_cbs: Option<std::sync::Arc<dyn crate::HostCallbacks>> =
        host.host_callbacks.lock().clone();

    // --- Spawn background I/O thread -------------------------------------
    let cb = callback;
    let resource_id = loader;
    let instance_id = loader_instance;

    std::thread::spawn(move || {
        let Some(host) = HOST.get() else { return };

        // Call the host's on_url_open (may block for HTTP / file I/O).
        let result = if let Some(ref hcb) = host_cbs {
            hcb.on_url_open(&url, &method, &headers, body.as_deref())
        } else {
            Err(PP_ERROR_FAILED)
        };

        match result {
            Ok(mut response) => {
                // ------ Headers received —— populate metadata ------
                {
                    let mut inner = inner_arc.lock();
                    inner.open_complete = true;
                    inner.total_bytes = response.content_length.unwrap_or(-1);
                    // Upload is delivered atomically via the request body,
                    // so mark it fully sent once headers come back.
                    inner.bytes_sent = inner.total_bytes_to_send;
                }

                // Create the URLResponseInfo resource.
                let ri = super::url_response_info::URLResponseInfoResource {
                    url: url.clone(),
                    status_code: response.status_code as i32,
                    status_line: response.status_line.clone(),
                    headers: response.headers.clone(),
                    redirect_url: String::new(),
                };
                let ri_id = host.resources.insert(instance_id, Box::new(ri));
                host.resources
                    .with_downcast_mut::<URLLoaderResource, _>(resource_id, |l| {
                        l.response_info_id = Some(ri_id);
                    });

                tracing::debug!(
                    "PPB_URLLoader::Open: loader={} headers received, status={}, \
                     content_length={:?}, response_info={}",
                    resource_id,
                    response.status_code,
                    response.content_length,
                    ri_id
                );

                // Fire the Open completion callback (headers ready).
                if let Some(ref p) = poster {
                    p.post_work(cb, 0, PP_OK);
                } else {
                    unsafe { cb.run(PP_OK) };
                }

                // ------ Stream the response body in chunks ------
                let mut chunk_buf = vec![0u8; STREAM_CHUNK_SIZE];
                loop {
                    let n = match response.body.read(&mut chunk_buf) {
                        Ok(0) => {
                            // --- EOF ---
                            let mut inner = inner_arc.lock();
                            inner.finished = true;
                            tracing::debug!(
                                "PPB_URLLoader: loader={} download complete, total {} bytes",
                                resource_id,
                                inner.bytes_received
                            );
                            // Satisfy any pending ReadResponseBody with 0 (EOF).
                            if let Some(pending) = inner.pending_read.take() {
                                drop(inner);
                                if let Some(ref p) = poster {
                                    p.post_work(pending.callback, 0, 0);
                                } else {
                                    unsafe { pending.callback.run(0) };
                                }
                            }
                            break;
                        }
                        Ok(n) => n,
                        Err(e) => {
                            tracing::warn!(
                                "PPB_URLLoader: loader={} read error: {}",
                                resource_id,
                                e
                            );
                            let mut inner = inner_arc.lock();
                            inner.finished = true;
                            inner.error = Some(PP_ERROR_FAILED);
                            if let Some(pending) = inner.pending_read.take() {
                                drop(inner);
                                if let Some(ref p) = poster {
                                    p.post_work(pending.callback, 0, PP_ERROR_FAILED);
                                } else {
                                    unsafe { pending.callback.run(PP_ERROR_FAILED) };
                                }
                            }
                            break;
                        }
                    };

                    // We got `n` bytes — deliver them.
                    let mut inner = inner_arc.lock();
                    inner.bytes_received += n as i64;

                    if let Some(pending) = inner.pending_read.take() {
                        // A ReadResponseBody is waiting — serve directly into
                        // the plugin's buffer, bypassing the VecDeque.
                        let to_copy = n.min(pending.bytes_to_read);
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                chunk_buf.as_ptr(),
                                pending.buffer,
                                to_copy,
                            );
                        }
                        // Buffer any leftover bytes.
                        if n > to_copy {
                            inner.buffer.extend(&chunk_buf[to_copy..n]);
                        }
                        drop(inner);
                        if let Some(ref p) = poster {
                            p.post_work(pending.callback, 0, to_copy as i32);
                        } else {
                            unsafe { pending.callback.run(to_copy as i32) };
                        }
                    } else {
                        // No pending read — accumulate in the buffer.
                        inner.buffer.extend(&chunk_buf[..n]);
                    }
                }
            }
            Err(error_code) => {
                // ------- Request failed -------
                {
                    let mut inner = inner_arc.lock();
                    inner.open_complete = true;
                    inner.finished = true;
                    inner.error = Some(error_code);
                }
                tracing::warn!(
                    "PPB_URLLoader::Open: loader={} request failed with {}",
                    resource_id,
                    error_code
                );

                // Create a minimal response-info so GetResponseInfo
                // doesn't return 0 (which confuses Flash).
                let ri = super::url_response_info::URLResponseInfoResource {
                    url: url.clone(),
                    status_code: 0,
                    status_line: String::new(),
                    headers: String::new(),
                    redirect_url: String::new(),
                };
                let ri_id = host.resources.insert(instance_id, Box::new(ri));
                host.resources
                    .with_downcast_mut::<URLLoaderResource, _>(resource_id, |l| {
                        l.response_info_id = Some(ri_id);
                    });

                // Fire Open callback with the error code.
                if let Some(ref p) = poster {
                    p.post_work(cb, 0, error_code);
                } else {
                    unsafe { cb.run(error_code) };
                }
            }
        }
    });

    PP_OK_COMPLETIONPENDING
}

// ---------------------------------------------------------------------------
// FollowRedirect
// ---------------------------------------------------------------------------

unsafe extern "C" fn follow_redirect(
    loader: PP_Resource,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("PPB_URLLoader::FollowRedirect(loader={})", loader);
    crate::callback::complete_immediately(callback, PP_OK)
}

// ---------------------------------------------------------------------------
// Upload / download progress
// ---------------------------------------------------------------------------

unsafe extern "C" fn get_upload_progress(
    loader: PP_Resource,
    bytes_sent: *mut i64,
    total_bytes_to_be_sent: *mut i64,
) -> PP_Bool {
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
    let Some(host) = HOST.get() else { return PP_FALSE };

    host.resources
        .with_downcast::<URLLoaderResource, _>(loader, |l| {
            let inner = l.inner.lock();
            if !inner.record_download_progress {
                // Even without progress tracking we report what we know,
                // but the spec says to return PP_FALSE.
                if !bytes_received.is_null() {
                    unsafe { *bytes_received = inner.bytes_received };
                }
                if !total_bytes_to_be_received.is_null() {
                    unsafe { *total_bytes_to_be_received = inner.total_bytes };
                }
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
            let mut inner = l.inner.lock();
            let available = inner.buffer.len();

            if available > 0 {
                // --- Data ready — copy into the caller's buffer ---
                let to_read = (bytes_to_read as usize).min(available);
                let dst = buffer as *mut u8;

                // Make the VecDeque contiguous so we can memcpy.
                {
                    let contiguous = inner.buffer.make_contiguous();
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            contiguous.as_ptr(),
                            dst,
                            to_read,
                        );
                    }
                }
                let _ = inner.buffer.drain(..to_read);

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
        None => PP_ERROR_BADRESOURCE,
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
    crate::callback::complete_immediately(callback, PP_OK)
}

unsafe extern "C" fn close(loader: PP_Resource) {
    tracing::debug!("PPB_URLLoader::Close(loader={})", loader);
    // Mark the stream as finished so the background thread stops writing
    // and any pending read gets cancelled.
    let Some(host) = HOST.get() else { return };
    host.resources
        .with_downcast::<URLLoaderResource, _>(loader, |l| {
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
    _cb: PP_URLLoaderTrusted_StatusCallback,
) {
    tracing::debug!(
        "PPB_URLLoaderTrusted::RegisterStatusCallback(loader={})",
        loader
    );
    // No-op stub.
}
