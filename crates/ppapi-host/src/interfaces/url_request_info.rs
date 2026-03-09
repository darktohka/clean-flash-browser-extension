//! PPB_URLRequestInfo;1.0 implementation.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::c_void;
use std::io::{Read, Seek, SeekFrom};

use super::super::HOST;

const DEFAULT_PREFETCH_BUFFER_UPPER_THRESHOLD: i32 = 100 * 1000 * 1000;
const DEFAULT_PREFETCH_BUFFER_LOWER_THRESHOLD: i32 = 50 * 1000 * 1000;

#[inline]
fn pp_var_as_bool(value: PP_Var) -> Option<bool> {
    if value.type_ == PP_VARTYPE_BOOL {
        Some(unsafe { value.value.as_bool } != 0)
    } else {
        None
    }
}

#[inline]
fn pp_var_as_i32(value: PP_Var) -> Option<i32> {
    if value.type_ == PP_VARTYPE_INT32 {
        Some(unsafe { value.value.as_int })
    } else {
        None
    }
}

/// URLRequestInfo resource.
pub struct URLRequestInfoResource {
    pub url: Option<String>,
    pub method: Option<String>,
    pub headers: Option<String>,
    pub follow_redirects: bool,
    pub record_download_progress: bool,
    pub record_upload_progress: bool,
    pub allow_cross_origin_requests: bool,
    pub allow_credentials: bool,
    pub stream_to_file: bool,
    pub has_custom_referrer_url: bool,
    pub custom_referrer_url: String,
    pub has_custom_content_transfer_encoding: bool,
    pub custom_content_transfer_encoding: String,
    pub has_custom_user_agent: bool,
    pub custom_user_agent: String,
    pub prefetch_buffer_upper_threshold: i32,
    pub prefetch_buffer_lower_threshold: i32,
    pub body: Vec<u8>,
}

impl Resource for URLRequestInfoResource {
    fn resource_type(&self) -> &'static str {
        "PPB_URLRequestInfo"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

static VTABLE: PPB_URLRequestInfo_1_0 = PPB_URLRequestInfo_1_0 {
    Create: Some(create),
    IsURLRequestInfo: Some(is_url_request_info),
    SetProperty: Some(set_property),
    AppendDataToBody: Some(append_data_to_body),
    AppendFileToBody: Some(append_file_to_body),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_URLREQUESTINFO_INTERFACE_1_0, &VTABLE);
    }
}

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    tracing::debug!("PPB_URLRequestInfo::Create(instance={})", instance);
    let Some(host) = HOST.get() else {
        return 0;
    };
    let req = URLRequestInfoResource {
        url: None,
        method: None,
        headers: None,
        follow_redirects: true,
        record_download_progress: false,
        record_upload_progress: false,
        allow_cross_origin_requests: false,
        allow_credentials: false,
        stream_to_file: false,
        has_custom_referrer_url: false,
        custom_referrer_url: String::new(),
        has_custom_content_transfer_encoding: false,
        custom_content_transfer_encoding: String::new(),
        has_custom_user_agent: false,
        custom_user_agent: String::new(),
        prefetch_buffer_upper_threshold: DEFAULT_PREFETCH_BUFFER_UPPER_THRESHOLD,
        prefetch_buffer_lower_threshold: DEFAULT_PREFETCH_BUFFER_LOWER_THRESHOLD,
        body: Vec::new(),
    };
    host.resources.insert(instance, Box::new(req))
}

unsafe extern "C" fn is_url_request_info(resource: PP_Resource) -> PP_Bool {
    tracing::trace!("PPB_URLRequestInfo::IsURLRequestInfo(resource={})", resource);
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_URLRequestInfo")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn set_property(
    request: PP_Resource,
    property: PP_URLRequestProperty,
    value: PP_Var,
) -> PP_Bool {
    tracing::trace!(
        "PPB_URLRequestInfo::SetProperty called: request={}, property={}, value={:?}",
        request,
        property,
        value
    );
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    host.resources
        .with_downcast_mut::<URLRequestInfoResource, _>(request, |req| {
            match property {
                PP_URLREQUESTPROPERTY_URL => {
                    if value.type_ != PP_VARTYPE_STRING {
                        return PP_FALSE;
                    }
                    if let Some(v) = host.vars.get_string(value) {
                        req.url = Some(v);
                    } else {
                        return PP_FALSE;
                    }
                }
                PP_URLREQUESTPROPERTY_METHOD => {
                    if value.type_ != PP_VARTYPE_STRING {
                        return PP_FALSE;
                    }
                    if let Some(v) = host.vars.get_string(value) {
                        req.method = Some(v);
                    } else {
                        return PP_FALSE;
                    }
                }
                PP_URLREQUESTPROPERTY_HEADERS => {
                    if value.type_ != PP_VARTYPE_STRING {
                        return PP_FALSE;
                    }
                    if let Some(v) = host.vars.get_string(value) {
                        req.headers = Some(v);
                    } else {
                        return PP_FALSE;
                    }
                }
                PP_URLREQUESTPROPERTY_STREAMTOFILE => {
                    if let Some(v) = pp_var_as_bool(value) {
                        req.stream_to_file = v;
                    } else {
                        return PP_FALSE;
                    }
                }
                PP_URLREQUESTPROPERTY_FOLLOWREDIRECTS => {
                    if let Some(v) = pp_var_as_bool(value) {
                        req.follow_redirects = v;
                    } else {
                        return PP_FALSE;
                    }
                }
                PP_URLREQUESTPROPERTY_RECORDDOWNLOADPROGRESS => {
                    if let Some(v) = pp_var_as_bool(value) {
                        req.record_download_progress = v;
                    } else {
                        return PP_FALSE;
                    }
                }
                PP_URLREQUESTPROPERTY_RECORDUPLOADPROGRESS => {
                    if let Some(v) = pp_var_as_bool(value) {
                        req.record_upload_progress = v;
                    } else {
                        return PP_FALSE;
                    }
                }
                PP_URLREQUESTPROPERTY_ALLOWCROSSORIGINREQUESTS => {
                    if let Some(v) = pp_var_as_bool(value) {
                        req.allow_cross_origin_requests = v;
                    } else {
                        return PP_FALSE;
                    }
                }
                PP_URLREQUESTPROPERTY_ALLOWCREDENTIALS => {
                    if let Some(v) = pp_var_as_bool(value) {
                        req.allow_credentials = v;
                    } else {
                        return PP_FALSE;
                    }
                }
                PP_URLREQUESTPROPERTY_CUSTOMREFERRERURL => {
                    match value.type_ {
                        PP_VARTYPE_UNDEFINED => {
                            req.has_custom_referrer_url = false;
                            req.custom_referrer_url.clear();
                        }
                        PP_VARTYPE_STRING => {
                            if let Some(v) = host.vars.get_string(value) {
                                req.has_custom_referrer_url = true;
                                req.custom_referrer_url = v;
                            } else {
                                return PP_FALSE;
                            }
                        }
                        _ => return PP_FALSE,
                    }
                }
                PP_URLREQUESTPROPERTY_CUSTOMCONTENTTRANSFERENCODING => {
                    match value.type_ {
                        PP_VARTYPE_UNDEFINED => {
                            req.has_custom_content_transfer_encoding = false;
                            req.custom_content_transfer_encoding.clear();
                        }
                        PP_VARTYPE_STRING => {
                            if let Some(v) = host.vars.get_string(value) {
                                req.has_custom_content_transfer_encoding = true;
                                req.custom_content_transfer_encoding = v;
                            } else {
                                return PP_FALSE;
                            }
                        }
                        _ => return PP_FALSE,
                    }
                }
                PP_URLREQUESTPROPERTY_CUSTOMUSERAGENT => {
                    match value.type_ {
                        PP_VARTYPE_UNDEFINED => {
                            req.has_custom_user_agent = false;
                            req.custom_user_agent.clear();
                        }
                        PP_VARTYPE_STRING => {
                            if let Some(v) = host.vars.get_string(value) {
                                req.has_custom_user_agent = true;
                                req.custom_user_agent = v;
                            } else {
                                return PP_FALSE;
                            }
                        }
                        _ => return PP_FALSE,
                    }
                }
                PP_URLREQUESTPROPERTY_PREFETCHBUFFERUPPERTHRESHOLD => {
                    if let Some(v) = pp_var_as_i32(value) {
                        req.prefetch_buffer_upper_threshold = v;
                    } else {
                        return PP_FALSE;
                    }
                }
                PP_URLREQUESTPROPERTY_PREFETCHBUFFERLOWERTHRESHOLD => {
                    if let Some(v) = pp_var_as_i32(value) {
                        req.prefetch_buffer_lower_threshold = v;
                    } else {
                        return PP_FALSE;
                    }
                }
                _ => {
                    // Unknown properties are rejected.
                    return PP_FALSE;
                }
            }
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn append_data_to_body(
    request: PP_Resource,
    data: *const c_void,
    len: u32,
) -> PP_Bool {
    tracing::trace!(
        "PPB_URLRequestInfo::AppendDataToBody called: request={}, data={:?}, len={}",
        request,
        data,
        len
    );
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    if data.is_null() {
        return PP_FALSE;
    }

    host.resources
        .with_downcast_mut::<URLRequestInfoResource, _>(request, |req| {
            let slice = unsafe { std::slice::from_raw_parts(data as *const u8, len as usize) };
            req.body.extend_from_slice(slice);
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn append_file_to_body(
    request: PP_Resource,
    file_ref: PP_Resource,
    start_offset: i64,
    number_of_bytes: i64,
    _expected_last_modified_time: PP_Time,
) -> PP_Bool {
    tracing::trace!(
        "PPB_URLRequestInfo::AppendFileToBody(request={}, file_ref={}, start_offset={}, number_of_bytes={})",
        request,
        file_ref,
        start_offset,
        number_of_bytes
    );

    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    // Ignore appending zero bytes.
    if number_of_bytes == 0 {
        return PP_TRUE;
    }

    // Follow Chromium's basic argument validation.
    if start_offset < 0 || number_of_bytes < -1 {
        return PP_FALSE;
    }

    let path = host
        .resources
        .with_downcast::<super::file_ref::FileRefResource, _>(file_ref, |fr| {
            if fr.file_type == super::file_ref::FileRefType::Name {
                fr.path.clone()
            } else {
                None
            }
        })
        .flatten();

    let Some(path) = path else {
        return PP_FALSE;
    };

    let mut file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return PP_FALSE,
    };

    if file.seek(SeekFrom::Start(start_offset as u64)).is_err() {
        return PP_FALSE;
    }

    let mut data = Vec::new();
    if number_of_bytes == -1 {
        if file.read_to_end(&mut data).is_err() {
            return PP_FALSE;
        }
    } else {
        let mut limited = file.take(number_of_bytes as u64);
        if limited.read_to_end(&mut data).is_err() {
            return PP_FALSE;
        }
    }

    host.resources
        .with_downcast_mut::<URLRequestInfoResource, _>(request, |req| {
            req.body.extend_from_slice(&data);
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}
