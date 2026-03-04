//! PPB_URLLoader;1.0 implementation.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::c_void;

use super::super::HOST;

/// URLLoader resource state.
#[derive(Debug)]
pub struct URLLoaderResource {
    pub instance: PP_Instance,
    pub url: Option<String>,
    pub response_info: Option<PP_Resource>,
    pub response_body: Vec<u8>,
    pub read_offset: usize,
    /// Set to true once Open() has been called (or the loader was pre-loaded).
    /// Flash may check this implicitly via GetResponseInfo/GetDownloadProgress.
    pub open_complete: bool,
    /// Set to true once all data has been delivered (EOF).
    pub finished_loading: bool,
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

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    tracing::trace!("PPB_URLLoader::Create(instance={})", instance);
    let Some(host) = HOST.get() else {
        return 0;
    };
    let loader = URLLoaderResource {
        instance,
        url: None,
        response_info: None,
        response_body: Vec::new(),
        read_offset: 0,
        open_complete: false,
        finished_loading: false,
    };
    let id = host.resources.insert(instance, Box::new(loader));
    tracing::debug!("PPB_URLLoader::Create(instance={}) -> resource={}", instance, id);
    id
}

unsafe extern "C" fn is_url_loader(resource: PP_Resource) -> PP_Bool {
    tracing::debug!("PPB_URLLoader::IsURLLoader(resource={})", resource);
    let result = HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_URLLoader")))
        .unwrap_or(PP_FALSE);
    tracing::debug!("PPB_URLLoader::IsURLLoader(resource={}) -> {}", resource, result);
    result
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

    // Read the URL from the request info resource.
    let url: Option<String> = host.resources.with_downcast::<super::url_request_info::URLRequestInfoResource, _>(
        request_info,
        |req| req.url.clone().unwrap_or_default(),
    );

    let url_str = url.clone().unwrap_or_default();
    tracing::debug!("PPB_URLLoader::Open: loader={} url={:?}", loader, url_str);

    if let Some(url) = url {
        // Notify the host that a URL load is requested.
        if let Some(cb) = host.host_callbacks.lock().as_ref() {
            let body = cb.on_url_load(&url);
            let body_len = body.len();
            host.resources.with_downcast_mut::<URLLoaderResource, _>(loader, |l| {
                l.url = Some(url.clone());
                l.response_body = body;
                l.read_offset = 0;
                l.open_complete = true;
                l.finished_loading = true;
            });
            tracing::debug!("PPB_URLLoader::Open: loader={} loaded {} bytes from {:?}", loader, body_len, url);
        }
    }

    // Complete: fire callback with PP_OK.
    // Use FLAG_OPTIONAL semantics: if the callback has FLAG_OPTIONAL set,
    // return the result directly. Otherwise fire the callback.
    if callback.is_null() {
        tracing::debug!("PPB_URLLoader::Open: loader={} -> PP_OK (blocking)", loader);
        PP_OK
    } else if callback.flags == PP_COMPLETIONCALLBACK_FLAG_OPTIONAL {
        tracing::debug!("PPB_URLLoader::Open: loader={} -> PP_OK (optional, sync)", loader);
        PP_OK
    } else {
        tracing::debug!("PPB_URLLoader::Open: loader={} -> PP_OK_COMPLETIONPENDING", loader);
        // Post callback to message loop so it fires asynchronously.
        // Use main_loop_poster (channel-based) instead of locking
        // main_message_loop directly — avoids deadlock when Open is
        // called from within a callback dispatched by poll_main_loop
        // (which already holds the main_message_loop lock).
        if let Some(poster) = &*host.main_loop_poster.lock() {
            poster.post_work(callback, 0, PP_OK);
        } else {
            // Fallback: fire inline if no message loop.
            unsafe { callback.run(PP_OK) };
        }
        PP_OK_COMPLETIONPENDING
    }
}

unsafe extern "C" fn follow_redirect(
    loader: PP_Resource,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("PPB_URLLoader::FollowRedirect(loader={})", loader);
    crate::callback::complete_immediately(callback, PP_OK)
}

unsafe extern "C" fn get_upload_progress(
    loader: PP_Resource,
    bytes_sent: *mut i64,
    total_bytes_to_be_sent: *mut i64,
) -> PP_Bool {
    tracing::debug!("PPB_URLLoader::GetUploadProgress(loader={})", loader);
    if !bytes_sent.is_null() {
        unsafe { *bytes_sent = 0 };
    }
    if !total_bytes_to_be_sent.is_null() {
        unsafe { *total_bytes_to_be_sent = 0 };
    }
    PP_FALSE
}

unsafe extern "C" fn get_download_progress(
    loader: PP_Resource,
    bytes_received: *mut i64,
    total_bytes_to_be_received: *mut i64,
) -> PP_Bool {
    tracing::debug!("PPB_URLLoader::GetDownloadProgress(loader={})", loader);
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    let result = host.resources
        .with_downcast::<URLLoaderResource, _>(loader, |l| {
            let total = l.response_body.len() as i64;
            if !bytes_received.is_null() {
                unsafe { *bytes_received = total };
            }
            if !total_bytes_to_be_received.is_null() {
                unsafe { *total_bytes_to_be_received = total };
            }
            tracing::debug!(
                "PPB_URLLoader::GetDownloadProgress(loader={}) -> received={}, total={}",
                loader, total, total
            );
            PP_TRUE
        })
        .unwrap_or(PP_FALSE);
    if result == PP_FALSE {
        tracing::debug!("PPB_URLLoader::GetDownloadProgress(loader={}) -> PP_FALSE (bad resource)", loader);
    }
    result
}

unsafe extern "C" fn get_response_info(loader: PP_Resource) -> PP_Resource {
    tracing::debug!("PPB_URLLoader::GetResponseInfo(loader={})", loader);
    let Some(host) = HOST.get() else {
        tracing::debug!("PPB_URLLoader::GetResponseInfo(loader={}) -> 0 (no host)", loader);
        return 0;
    };

    // Check if we already have a response info resource.
    let existing = host.resources
        .with_downcast::<URLLoaderResource, _>(loader, |l| l.response_info)
        .unwrap_or(None);

    if let Some(id) = existing {
        // Add a ref since we're returning a new handle to the caller.
        host.resources.add_ref(id);
        tracing::debug!("PPB_URLLoader::GetResponseInfo(loader={}) -> {} (existing)", loader, id);
        return id;
    }

    // Lazily create a response info for loaders opened via Open().
    let instance = host.resources.get_instance(loader);
    let Some(instance_id) = instance else {
        tracing::debug!("PPB_URLLoader::GetResponseInfo(loader={}) -> 0 (no instance)", loader);
        return 0;
    };

    // Use the URL from the loader if available.
    let url = host.resources
        .with_downcast::<URLLoaderResource, _>(loader, |l| l.url.clone().unwrap_or_default())
        .unwrap_or_default();

    let ri = super::url_response_info::URLResponseInfoResource {
        url,
        status_code: 200,
        status_line: "OK".to_string(),
        headers: String::new(),
    };
    let ri_id = host.resources.insert(instance_id, Box::new(ri));

    host.resources.with_downcast_mut::<URLLoaderResource, _>(loader, |l| {
        l.response_info = Some(ri_id);
    });

    tracing::debug!("PPB_URLLoader::GetResponseInfo(loader={}) -> {} (created)", loader, ri_id);
    ri_id
}

unsafe extern "C" fn read_response_body(
    loader: PP_Resource,
    buffer: *mut c_void,
    bytes_to_read: i32,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::trace!(
        "PPB_URLLoader::ReadResponseBody(loader={}, buffer={:?}, bytes_to_read={}, callback={:?})",
        loader,
        buffer,
        bytes_to_read,
        callback
    );

    let Some(host) = HOST.get() else {
        tracing::trace!("PPB_URLLoader::ReadResponseBody -> host not initialized");
        return PP_ERROR_FAILED;
    };

    let bytes_read = host.resources.with_downcast_mut::<URLLoaderResource, _>(loader, |l| {
        let remaining = l.response_body.len() - l.read_offset;
        let to_read = (bytes_to_read as usize).min(remaining);
        if to_read > 0 && !buffer.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    l.response_body.as_ptr().add(l.read_offset),
                    buffer as *mut u8,
                    to_read,
                );
            }
            l.read_offset += to_read;
        }
        to_read as i32
    }).unwrap_or(PP_ERROR_BADRESOURCE);

    if callback.is_null() {
        tracing::debug!("PPB_URLLoader::ReadResponseBody(loader={}, buffer={:?}, bytes_to_read={}, callback=null) -> bytes_read={}", loader, buffer, bytes_to_read, bytes_read);
        bytes_read
    } else {
        tracing::debug!("PPB_URLLoader::ReadResponseBody(loader={}, buffer={:?}, bytes_to_read={}, callback={:?}) -> bytes_read={}", loader, buffer, bytes_to_read, callback, bytes_read);
        unsafe { callback.run(bytes_read) };
        PP_OK_COMPLETIONPENDING
    }
}

unsafe extern "C" fn finish_streaming_to_file(
    loader: PP_Resource,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("PPB_URLLoader::FinishStreamingToFile(loader={})", loader);
    crate::callback::complete_immediately(callback, PP_OK)
}

unsafe extern "C" fn close(loader: PP_Resource) {
    tracing::debug!("PPB_URLLoader::Close(loader={})", loader);
    // Resource will be cleaned up when released.
}

// --- Trusted ---

unsafe extern "C" fn grant_universal_access(loader: PP_Resource) {
    tracing::debug!("PPB_URLLoaderTrusted::GrantUniversalAccess(loader={})", loader);
    // No-op: we always grant access in our standalone projector.
}

unsafe extern "C" fn register_status_callback(
    loader: PP_Resource,
    _cb: PP_URLLoaderTrusted_StatusCallback,
) {
    tracing::debug!("PPB_URLLoaderTrusted::RegisterStatusCallback(loader={})", loader);
    // No-op stub.
}
