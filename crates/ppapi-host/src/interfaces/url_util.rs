//! PPB_URLUtil(Dev);0.7 implementation.
//!
//! URL utility functions: canonicalization, relative URL resolution,
//! document/plugin URL queries.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;

use super::super::HOST;

static VTABLE_0_7: PPB_URLUtil_Dev_0_7 = PPB_URLUtil_Dev_0_7 {
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

static VTABLE_0_6: PPB_URLUtil_Dev_0_6 = PPB_URLUtil_Dev_0_6 {
    Canonicalize: Some(canonicalize),
    ResolveRelativeToURL: Some(resolve_relative_to_url),
    ResolveRelativeToDocument: Some(resolve_relative_to_document),
    IsSameSecurityOrigin: Some(is_same_security_origin),
    DocumentCanRequest: Some(document_can_request),
    DocumentCanAccessDocument: Some(document_can_access_document),
    GetDocumentURL: Some(get_document_url),
    GetPluginInstanceURL: Some(get_plugin_instance_url),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_URLUTIL_DEV_INTERFACE_0_7, &VTABLE_0_7);
        registry.register(PPB_URLUTIL_DEV_INTERFACE_0_6, &VTABLE_0_6);
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

    // Parse fragment/query boundaries first so authority/path parsing does not
    // accidentally consume them.
    let ref_start = url.find('#');
    let end_no_ref = ref_start.unwrap_or(url.len());
    if let Some(hash) = ref_start {
        comp.ref_ = PP_URLComponent_Dev {
            begin: (hash + 1) as i32,
            len: (url.len() - hash - 1) as i32,
        };
    }

    let query_start = url[..end_no_ref].find('?');
    let end_no_query = query_start.unwrap_or(end_no_ref);
    if let Some(qmark) = query_start {
        comp.query = PP_URLComponent_Dev {
            begin: (qmark + 1) as i32,
            len: (end_no_ref - qmark - 1) as i32,
        };
    }

    let mut pos = 0usize;

    // Scheme (supports both hierarchical and non-hierarchical URLs).
    if let Some((scheme, _)) = split_scheme(&url[..end_no_query]) {
        let scheme_end = scheme.len();
        comp.scheme = PP_URLComponent_Dev {
            begin: 0,
            len: scheme_end as i32,
        };
        pos = scheme_end + 1; // skip ":"

        // Authority (if present): //[userinfo@]host[:port]
        if url[pos..end_no_query].starts_with("//") {
            let auth_start = pos + 2;
            let auth_end = url[auth_start..end_no_query]
                .find('/')
                .map(|i| auth_start + i)
                .unwrap_or(end_no_query);

            let authority = &url[auth_start..auth_end];
            let mut hostport_start = auth_start;

            // Userinfo: username[:password]@
            if let Some(at_rel) = authority.rfind('@') {
                let userinfo_start = auth_start;
                let userinfo_end = auth_start + at_rel;
                let userinfo = &url[userinfo_start..userinfo_end];
                hostport_start = userinfo_end + 1;

                if let Some(colon_rel) = userinfo.find(':') {
                    comp.username = PP_URLComponent_Dev {
                        begin: userinfo_start as i32,
                        len: colon_rel as i32,
                    };
                    comp.password = PP_URLComponent_Dev {
                        begin: (userinfo_start + colon_rel + 1) as i32,
                        len: (userinfo.len() - colon_rel - 1) as i32,
                    };
                } else {
                    comp.username = PP_URLComponent_Dev {
                        begin: userinfo_start as i32,
                        len: userinfo.len() as i32,
                    };
                }
            }

            let hostport = &url[hostport_start..auth_end];

            // Host + optional port, with IPv6 literal support.
            if hostport.starts_with('[') {
                if let Some(close_rel) = hostport.find(']') {
                    // [ipv6]
                    comp.host = PP_URLComponent_Dev {
                        begin: hostport_start as i32,
                        len: (close_rel + 1) as i32,
                    };

                    if close_rel + 1 < hostport.len() {
                        if hostport.as_bytes()[close_rel + 1] == b':' {
                            comp.port = PP_URLComponent_Dev {
                                begin: (hostport_start + close_rel + 2) as i32,
                                len: (hostport.len() - close_rel - 2) as i32,
                            };
                        } else {
                            // Malformed tail after ]: keep entire hostport as host.
                            comp.host = PP_URLComponent_Dev {
                                begin: hostport_start as i32,
                                len: hostport.len() as i32,
                            };
                        }
                    }
                } else {
                    // Malformed IPv6 literal: keep as host.
                    comp.host = PP_URLComponent_Dev {
                        begin: hostport_start as i32,
                        len: hostport.len() as i32,
                    };
                }
            } else if let Some(colon_rel) = hostport.rfind(':') {
                comp.host = PP_URLComponent_Dev {
                    begin: hostport_start as i32,
                    len: colon_rel as i32,
                };
                comp.port = PP_URLComponent_Dev {
                    begin: (hostport_start + colon_rel + 1) as i32,
                    len: (hostport.len() - colon_rel - 1) as i32,
                };
            } else {
                // Also covers empty host (eg file:///path) with len=0.
                comp.host = PP_URLComponent_Dev {
                    begin: hostport_start as i32,
                    len: hostport.len() as i32,
                };
            }

            pos = auth_end;
        }
    }

    // Path (may be empty when query/fragment exists immediately after authority)
    if pos < url.len() {
        comp.path = PP_URLComponent_Dev {
            begin: pos as i32,
            len: (end_no_query - pos) as i32,
        };
    }

    unsafe { *components = comp };
}

fn non_empty_url(url: Option<String>) -> Option<String> {
    url.and_then(|u| {
        if u.trim().is_empty() {
            None
        } else {
            // Remove trailing /
            Some(u.trim_end_matches('/').to_string())
        }
    })
}

fn var_to_string(host: &crate::HostState, var: PP_Var) -> Option<String> {
    host.vars.get_string(var)
}

fn split_scheme(url: &str) -> Option<(&str, &str)> {
    let colon = url.find(':')?;
    let scheme = &url[..colon];
    if scheme.is_empty() {
        return None;
    }

    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
        return None;
    }

    Some((scheme, &url[colon + 1..]))
}

fn split_hierarchical_url(url: &str) -> Option<(&str, &str, &str)> {
    let (scheme, after_scheme) = split_scheme(url)?;
    let after_slashes = after_scheme.strip_prefix("//")?;

    let authority_end = after_slashes
        .find(|c| c == '/' || c == '?' || c == '#')
        .unwrap_or(after_slashes.len());
    let authority = &after_slashes[..authority_end];
    let tail = &after_slashes[authority_end..];

    Some((scheme, authority, tail))
}

fn is_absolute_url(url: &str) -> bool {
    split_scheme(url).is_some()
}

fn document_base_url(host: &crate::HostState, instance: PP_Instance) -> Option<String> {
    let provider = host.get_url_provider();

    non_empty_url(
        provider
            .as_ref()
            .and_then(|p| p.get_document_base_url(instance)),
    )
    .or_else(|| {
        non_empty_url(
            host.instances
                .with_instance(instance, |inst| inst.swf_url.clone())
                .flatten(),
        )
    })
}

unsafe extern "C" fn canonicalize(
    url: PP_Var,
    components: *mut PP_URLComponents_Dev,
) -> PP_Var {
    tracing::trace!("PPB_URLUtil::Canonicalize called with url={:?}", url);
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    let Some(url_str) = var_to_string(host, url) else {
        return PP_Var::null();
    };
    if !is_absolute_url(url_str.trim()) {
        return PP_Var::null();
    }

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
    let Some(base) = var_to_string(host, base_url) else {
        return PP_Var::null();
    };
    let Some(relative) = var_to_string(host, relative_string) else {
        return PP_Var::null();
    };

    let Some(resolved) = resolve_url(Some(&base), &relative) else {
        return PP_Var::null();
    };

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

    let Some(relative) = var_to_string(host, relative_string) else {
        return PP_Var::null();
    };

    // PPAPI uses the containing document URL as the base for this API.
    let base_url = document_base_url(host, instance);

    let Some(resolved) = resolve_url(base_url.as_deref(), &relative) else {
        return PP_Var::null();
    };

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

    let provider = host.get_url_provider();

    let url: String = non_empty_url(
        provider
            .as_ref()
            .and_then(|p| p.get_document_url(instance)),
    )
    .or_else(|| {
        non_empty_url(
            host.instances
                .with_instance(instance, |inst| inst.swf_url.clone())
                .flatten(),
        )
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
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };

    let provider = host.get_url_provider();

    let url: String = non_empty_url(
        provider
            .as_ref()
            .and_then(|p| p.get_plugin_instance_url(instance)),
    )
    .or_else(|| {
        non_empty_url(
            host.instances
                .with_instance(instance, |inst| inst.swf_url.clone())
                .flatten(),
        )
    })
    .or_else(|| {
        non_empty_url(
            provider
                .as_ref()
                .and_then(|p| p.get_document_url(instance)),
        )
    })
    .unwrap_or_else(|| "file:///".to_string());

    parse_components(&url, components);
    tracing::info!("Plugin URL for instance {}: {}", instance, url);
    host.vars.var_from_str(&url)
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
    tracing::trace!("PPB_URLUtil::IsSameSecurityOrigin called");
    // Everything is same-origin in our projector.
    PP_TRUE
}

unsafe extern "C" fn document_can_request(
    _instance: PP_Instance,
    _url: PP_Var,
) -> PP_Bool {
    tracing::trace!("PPB_URLUtil::DocumentCanRequest called");
    PP_TRUE
}

unsafe extern "C" fn document_can_access_document(
    _active: PP_Instance,
    _target: PP_Instance,
) -> PP_Bool {
    tracing::trace!(
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
fn resolve_url(base: Option<&str>, relative: &str) -> Option<String> {
    // Absolute relative URL wins, even if base is missing/invalid.
    if is_absolute_url(relative) {
        return Some(relative.to_string());
    }

    // We need a base URL for all non-absolute relative forms.
    let base = base?.trim();

    // Protocol-relative form (`//host/path`) inherits base scheme.
    if relative.starts_with("//") {
        let (scheme, _) = split_scheme(base)?;
        return Some(format!("{}:{}", scheme, relative));
    }

    // Non-hierarchical bases (e.g. `data:`) do not support relative path resolution.
    let (scheme, authority, tail) = split_hierarchical_url(base)?;

    // Remove fragment from base first.
    let base_without_fragment = tail
        .split_once('#')
        .map(|(lhs, _)| lhs)
        .unwrap_or(tail);

    // Empty relative means "same document" (minus fragment).
    if relative.is_empty() {
        return Some(format!("{}://{}{}", scheme, authority, base_without_fragment));
    }

    // Fragment-only references replace fragment.
    if relative.starts_with('#') {
        return Some(format!("{}://{}{}{}", scheme, authority, base_without_fragment, relative));
    }

    // Query-only references keep path but replace query.
    let base_path = base_without_fragment
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(base_without_fragment);
    if relative.starts_with('?') {
        return Some(format!("{}://{}{}{}", scheme, authority, base_path, relative));
    }

    // Absolute-path reference (`/x`).
    if relative.starts_with('/') {
        return Some(format!("{}://{}{}", scheme, authority, relative));
    }

    // Relative path: strip filename from base path and append relative.
    let effective_base_path = if base_path.is_empty() { "/" } else { base_path };
    let base_dir = if effective_base_path.ends_with('/') {
        effective_base_path.to_string()
    } else if let Some((prefix, _)) = effective_base_path.rsplit_once('/') {
        if prefix.is_empty() {
            "/".to_string()
        } else {
            format!("{}/", prefix)
        }
    } else {
        "/".to_string()
    };

    Some(format!("{}://{}{}{}", scheme, authority, base_dir, relative))
}
