//! PPB_URLUtil(Dev);0.7 implementation.
//!
//! URL utility functions: canonicalization, relative URL resolution,
//! document/plugin URL queries.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;

use super::super::HOST;

static VTABLE: PPB_URLUtil_Dev_0_7 = PPB_URLUtil_Dev_0_7 {
    Canonicalize: Some(canonicalize),
    ResolveRelativeToURL: Some(resolve_relative_to_url),
    ResolveRelativeToDocument: Some(resolve_relative_to_document),
    IsSameSecurityOrigin: Some(is_same_security_origin),
    DocumentCanRequest: Some(document_can_request),
    DocumentCanAccessDocument: Some(document_can_access_document),
    GetDocumentURL: Some(get_document_url),
    GetPluginInstanceURL: Some(get_plugin_instance_url),
    GetPluginReferrerURL: Some(get_plugin_referrer_url),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_URLUTIL_DEV_INTERFACE_0_7, &VTABLE);
        registry.register(PPB_URLUTIL_DEV_INTERFACE_0_6, &VTABLE);
    }
}

/// Parse URL components from a &str, filling PP_URLComponents_Dev.
/// Simple heuristic parser — not a full RFC 3986 implementation.
/// Absent/unspecified URL component — matches Chrome's `url::Component()`
/// default: `begin=0, len=-1`.  Flash checks `len != -1` (i.e. `is_valid()`)
/// to decide whether a component is present, so using `len=0` for absent
/// components would trick Flash into treating them as present and then
/// indexing the URL string at `begin`, causing a SIGSEGV.
const ABSENT: PP_URLComponent_Dev = PP_URLComponent_Dev { begin: 0, len: -1 };

fn parse_components(url: &str, components: *mut PP_URLComponents_Dev) {
    if components.is_null() {
        return;
    }
    let mut comp = PP_URLComponents_Dev::default();
    // Initialize all as "not present" (len = -1, matching Chrome convention)
    comp.scheme = ABSENT;
    comp.username = ABSENT;
    comp.password = ABSENT;
    comp.host = ABSENT;
    comp.port = ABSENT;
    comp.path = ABSENT;
    comp.query = ABSENT;
    comp.ref_ = ABSENT;

    let bytes = url.as_bytes();
    let mut pos = 0usize;

    // Scheme: find "://"
    if let Some(scheme_end) = url.find("://") {
        comp.scheme = PP_URLComponent_Dev {
            begin: 0,
            len: scheme_end as i32,
        };
        pos = scheme_end + 3; // skip "://"

        // Authority (host[:port])
        let auth_start = pos;
        let auth_end = url[pos..]
            .find('/')
            .map(|i| pos + i)
            .unwrap_or(url.len());

        let auth = &url[auth_start..auth_end];
        // Check for port
        if let Some(colon) = auth.rfind(':') {
            comp.host = PP_URLComponent_Dev {
                begin: auth_start as i32,
                len: colon as i32,
            };
            comp.port = PP_URLComponent_Dev {
                begin: (auth_start + colon + 1) as i32,
                len: (auth.len() - colon - 1) as i32,
            };
        } else {
            comp.host = PP_URLComponent_Dev {
                begin: auth_start as i32,
                len: auth.len() as i32,
            };
        }
        pos = auth_end;
    }

    // Path
    if pos < url.len() {
        let path_start = pos;
        let path_end = url[pos..]
            .find(|c| c == '?' || c == '#')
            .map(|i| pos + i)
            .unwrap_or(url.len());
        comp.path = PP_URLComponent_Dev {
            begin: path_start as i32,
            len: (path_end - path_start) as i32,
        };
        pos = path_end;
    }

    // Query
    if pos < url.len() && bytes[pos] == b'?' {
        pos += 1;
        let query_start = pos;
        let query_end = url[pos..]
            .find('#')
            .map(|i| pos + i)
            .unwrap_or(url.len());
        comp.query = PP_URLComponent_Dev {
            begin: query_start as i32,
            len: (query_end - query_start) as i32,
        };
        pos = query_end;
    }

    // Fragment
    if pos < url.len() && bytes[pos] == b'#' {
        pos += 1;
        comp.ref_ = PP_URLComponent_Dev {
            begin: pos as i32,
            len: (url.len() - pos) as i32,
        };
    }

    unsafe { *components = comp };
}

unsafe extern "C" fn canonicalize(
    url: PP_Var,
    components: *mut PP_URLComponents_Dev,
) -> PP_Var {
    tracing::debug!("PPB_URLUtil::Canonicalize called with url={:?}", url);
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    let url_str = host.vars.get_string(url).unwrap_or_default();
    parse_components(&url_str, components);
    // Return the URL as-is (already canonical enough for our purposes).
    host.vars.var_from_str(&url_str)
}

unsafe extern "C" fn resolve_relative_to_url(
    base_url: PP_Var,
    relative_string: PP_Var,
    components: *mut PP_URLComponents_Dev,
) -> PP_Var {
    tracing::debug!(
        "PPB_URLUtil::ResolveRelativeToURL called with base_url={:?} and relative_string={:?}",
        base_url,
        relative_string
    );
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    let base = host.vars.get_string(base_url).unwrap_or_default();
    let relative = host.vars.get_string(relative_string).unwrap_or_default();

    let resolved = resolve_url(&base, &relative);
    parse_components(&resolved, components);
    host.vars.var_from_str(&resolved)
}

unsafe extern "C" fn resolve_relative_to_document(
    instance: PP_Instance,
    relative_string: PP_Var,
    components: *mut PP_URLComponents_Dev,
) -> PP_Var {
    tracing::debug!(
        "PPB_URLUtil::ResolveRelativeToDocument called with instance={}, relative_string={:?}",
        instance,
        relative_string
    );
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };

    // Get the instance's SWF URL as the base.
    let base_url: String = host
        .instances
        .with_instance(instance, |inst| {
            inst.swf_url.clone().unwrap_or_default()
        })
        .unwrap_or_default();

    let relative = host.vars.get_string(relative_string).unwrap_or_default();
    let resolved = resolve_url(&base_url, &relative);
    parse_components(&resolved, components);
    host.vars.var_from_str(&resolved)
}

unsafe extern "C" fn get_document_url(
    instance: PP_Instance,
    components: *mut PP_URLComponents_Dev,
) -> PP_Var {
    tracing::info!("PPB_URLUtil::GetDocumentURL called with instance={}", instance);
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };

    let url: String = host
        .instances
        .with_instance(instance, |inst| {
            inst.swf_url.clone().unwrap_or_else(|| "file:///".to_string())
        })
        .unwrap_or_else(|| "file:///".to_string());

    parse_components(&url, components);
    tracing::info!("Document URL for instance {}: {}", instance, url);
    host.vars.var_from_str(&url)
}

unsafe extern "C" fn get_plugin_instance_url(
    instance: PP_Instance,
    components: *mut PP_URLComponents_Dev,
) -> PP_Var {
    tracing::info!("PPB_URLUtil::GetPluginInstanceURL called with instance={}", instance);
    // Same as document URL in our standalone projector.
    get_document_url(instance, components)
}

unsafe extern "C" fn get_plugin_referrer_url(
    _instance: PP_Instance,
    _components: *mut PP_URLComponents_Dev,
) -> PP_Var {
    tracing::info!("PPB_URLUtil::GetPluginReferrerURL called");
    // No referrer in standalone mode.
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    host.vars.var_from_str("")
}

unsafe extern "C" fn is_same_security_origin(
    _url_a: PP_Var,
    _url_b: PP_Var,
) -> PP_Bool {
    tracing::debug!("PPB_URLUtil::IsSameSecurityOrigin called");
    tracing::debug!("URL A: {:?}, URL B: {:?}", _url_a, _url_b);
    tracing::debug!("URL A as string: {:?}", HOST.get().and_then(|h| h.vars.get_string(_url_a)));
    tracing::debug!("URL B as string: {:?}", HOST.get().and_then(|h| h.vars.get_string(_url_b)));
    // Everything is same-origin in our projector.
    PP_TRUE
}

unsafe extern "C" fn document_can_request(
    _instance: PP_Instance,
    _url: PP_Var,
) -> PP_Bool {
    tracing::debug!("PPB_URLUtil::DocumentCanRequest called");
    PP_TRUE
}

unsafe extern "C" fn document_can_access_document(
    _active: PP_Instance,
    _target: PP_Instance,
) -> PP_Bool {
    tracing::debug!(
        "PPB_URLUtil::DocumentCanAccessDocument called with active={} target={}",
        _active,
        _target
    );
    PP_TRUE
}

// ---------------------------------------------------------------------------
// URL resolution helper
// ---------------------------------------------------------------------------

/// Resolve a relative URL against a base URL.
/// Handles common cases: absolute URLs, protocol-relative, path-relative.
fn resolve_url(base: &str, relative: &str) -> String {
    // If relative is already absolute, return it.
    if relative.contains("://") {
        return relative.to_string();
    }
    // Protocol-relative.
    if relative.starts_with("//") {
        if let Some(scheme_end) = base.find("://") {
            return format!("{}{}", &base[..scheme_end + 1], relative);
        }
        return relative.to_string();
    }
    // Absolute path.
    if relative.starts_with('/') {
        if let Some(scheme_end) = base.find("://") {
            let authority_end = base[scheme_end + 3..]
                .find('/')
                .map(|i| scheme_end + 3 + i)
                .unwrap_or(base.len());
            return format!("{}{}", &base[..authority_end], relative);
        }
        return relative.to_string();
    }
    // Relative path: strip filename from base, append relative.
    let base_dir = if let Some(last_slash) = base.rfind('/') {
        &base[..last_slash + 1]
    } else {
        base
    };
    format!("{}{}", base_dir, relative)
}
