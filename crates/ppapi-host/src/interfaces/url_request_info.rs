//! PPB_URLRequestInfo;1.0 implementation.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::c_void;

use super::super::HOST;

/// URLRequestInfo resource.
pub struct URLRequestInfoResource {
    pub url: Option<String>,
    pub method: Option<String>,
    pub headers: Option<String>,
    pub stream_to_file: bool,
    pub follow_redirects: bool,
    pub record_download_progress: bool,
    pub record_upload_progress: bool,
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
    let Some(host) = HOST.get() else {
        return 0;
    };
    let req = URLRequestInfoResource {
        url: None,
        method: None,
        headers: None,
        stream_to_file: false,
        follow_redirects: true,
        record_download_progress: false,
        record_upload_progress: false,
        body: Vec::new(),
    };
    host.resources.insert(instance, Box::new(req))
}

unsafe extern "C" fn is_url_request_info(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_URLRequestInfo")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn set_property(
    request: PP_Resource,
    property: PP_URLRequestProperty,
    value: PP_Var,
) -> PP_Bool {
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    host.resources
        .with_downcast_mut::<URLRequestInfoResource, _>(request, |req| {
            match property {
                PP_URLREQUESTPROPERTY_URL => {
                    req.url = host.vars.get_string(value);
                }
                PP_URLREQUESTPROPERTY_METHOD => {
                    req.method = host.vars.get_string(value);
                }
                PP_URLREQUESTPROPERTY_HEADERS => {
                    req.headers = host.vars.get_string(value);
                }
                PP_URLREQUESTPROPERTY_STREAMTOFILE => {
                    req.stream_to_file = value.type_ == PP_VARTYPE_BOOL && unsafe { value.value.as_bool } != 0;
                }
                PP_URLREQUESTPROPERTY_FOLLOWREDIRECTS => {
                    req.follow_redirects = value.type_ != PP_VARTYPE_BOOL || unsafe { value.value.as_bool } != 0;
                }
                PP_URLREQUESTPROPERTY_RECORDDOWNLOADPROGRESS => {
                    req.record_download_progress = value.type_ == PP_VARTYPE_BOOL && unsafe { value.value.as_bool } != 0;
                }
                PP_URLREQUESTPROPERTY_RECORDUPLOADPROGRESS => {
                    req.record_upload_progress = value.type_ == PP_VARTYPE_BOOL && unsafe { value.value.as_bool } != 0;
                }
                _ => {
                    // Other properties are accepted but ignored for now.
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
    _request: PP_Resource,
    _file_ref: PP_Resource,
    _start_offset: i64,
    _number_of_bytes: i64,
    _expected_last_modified_time: PP_Time,
) -> PP_Bool {
    // Not implemented yet.
    PP_FALSE
}
