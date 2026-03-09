//! PPB_URLResponseInfo;1.0 stub implementation.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

#[inline]
fn is_redirect(status_code: i32) -> bool {
    (300..=399).contains(&status_code)
}

/// URLResponseInfo resource.
pub struct URLResponseInfoResource {
    pub url: String,
    pub status_code: i32,
    pub status_line: String,
    pub headers: String,
    pub redirect_url: String,
}

impl Resource for URLResponseInfoResource {
    fn resource_type(&self) -> &'static str {
        "PPB_URLResponseInfo"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

static VTABLE: PPB_URLResponseInfo_1_0 = PPB_URLResponseInfo_1_0 {
    IsURLResponseInfo: Some(is_url_response_info),
    GetProperty: Some(get_property),
    GetBodyAsFileRef: Some(get_body_as_file_ref),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_URLRESPONSEINFO_INTERFACE_1_0, &VTABLE);
    }
}

unsafe extern "C" fn is_url_response_info(resource: PP_Resource) -> PP_Bool {
    let result = HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_URLResponseInfo")))
        .unwrap_or(PP_FALSE);
    tracing::info!("PPB_URLResponseInfo::IsURLResponseInfo(resource={}) -> {} <-- Flash checking response type", resource, result);
    result
}

unsafe extern "C" fn get_property(
    response: PP_Resource,
    property: PP_URLResponseProperty,
) -> PP_Var {
    tracing::trace!("PPB_URLResponseInfo::GetProperty(response={}, property={}) called", response, property);
    let Some(host) = HOST.get() else {
        tracing::debug!("PPB_URLResponseInfo::GetProperty(response={}, property={}) -> undefined (no host)", response, property);
        return PP_Var::undefined();
    };

    let result = host.resources
        .with_downcast::<URLResponseInfoResource, _>(response, |r| match property {
            PP_URLRESPONSEPROPERTY_URL => {
                let v = host.vars.var_from_str(&r.url);
                tracing::info!("PPB_URLResponseInfo::GetProperty(response={}, URL) -> {:?} <-- Flash querying URL", response, r.url);
                v
            }
            PP_URLRESPONSEPROPERTY_STATUSCODE => {
                tracing::info!("PPB_URLResponseInfo::GetProperty(response={}, STATUSCODE) -> {} <-- Flash querying status", response, r.status_code);
                PP_Var::from_int(r.status_code)
            }
            PP_URLRESPONSEPROPERTY_STATUSLINE => {
                tracing::info!("PPB_URLResponseInfo::GetProperty(response={}, STATUSLINE) -> {:?} <-- Flash querying status line", response, r.status_line);
                host.vars.var_from_str(&r.status_line)
            }
            PP_URLRESPONSEPROPERTY_HEADERS => {
                tracing::info!("PPB_URLResponseInfo::GetProperty(response={}, HEADERS) -> {:?} <-- Flash querying headers", response, r.headers);
                host.vars.var_from_str(&r.headers)
            }
            PP_URLRESPONSEPROPERTY_REDIRECTURL => {
                if is_redirect(r.status_code) {
                    tracing::trace!(
                        "PPB_URLResponseInfo::GetProperty(response={}, REDIRECTURL) -> {:?}",
                        response,
                        r.redirect_url
                    );
                    host.vars.var_from_str(&r.redirect_url)
                } else {
                    tracing::trace!(
                        "PPB_URLResponseInfo::GetProperty(response={}, REDIRECTURL) -> undefined",
                        response
                    );
                    PP_Var::undefined()
                }
            }
            PP_URLRESPONSEPROPERTY_REDIRECTMETHOD => {
                if is_redirect(r.status_code) {
                    // Chromium currently exposes status_text here.
                    host.vars.var_from_str(&r.status_line)
                } else {
                    tracing::info!("PPB_URLResponseInfo::GetProperty(response={}, property={}) -> undefined <-- Flash querying redirect", response, property);
                    PP_Var::undefined()
                }
            }
            _ => {
                tracing::info!("PPB_URLResponseInfo::GetProperty(response={}, property={}) -> undefined (unknown) <-- Flash querying unknown property", response, property);
                PP_Var::undefined()
            }
        })
        .unwrap_or_else(|| {
            tracing::warn!("PPB_URLResponseInfo::GetProperty(response={}, property={}) -> undefined (bad resource) <-- Flash querying invalid resource!", response, property);
            PP_Var::undefined()
        });
    result
}

unsafe extern "C" fn get_body_as_file_ref(response: PP_Resource) -> PP_Resource {
    // This should not be implemented, because Chrome does not support it
    tracing::debug!("PPB_URLResponseInfo::GetBodyAsFileRef(response={}) -> 0 (not implemented)", response);
    0 // Not implemented.
}
