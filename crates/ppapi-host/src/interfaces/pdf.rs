//! PPB_PDF;1 implementation.
//!
//! The PDF interface provides PDF-specific functionality such as font file
//! fallback, string searching, and V8 snapshot loading. In a standalone Flash
//! player most of these are no-ops - the plugin is not a PDF viewer. The font
//! functions delegate to the same infrastructure as PPB_Flash_FontFile when
//! that is available; for now they return failure/empty results. All calls are
//! traced for debugging purposes.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::c_void;

use super::super::HOST;

// ---------------------------------------------------------------------------
// VTable
// ---------------------------------------------------------------------------

static VTABLE: PPB_PDF = PPB_PDF {
    GetFontFileWithFallback: Some(get_font_file_with_fallback),
    GetFontTableForPrivateFontFile: Some(get_font_table_for_private_font_file),
    SearchString: Some(search_string),
    DidStartLoading: Some(did_start_loading),
    DidStopLoading: Some(did_stop_loading),
    SetContentRestriction: Some(set_content_restriction),
    UserMetricsRecordAction: Some(user_metrics_record_action),
    HasUnsupportedFeature: Some(has_unsupported_feature),
    SaveAs: Some(save_as),
    Print: Some(print),
    IsFeatureEnabled: Some(is_feature_enabled),
    SetSelectedText: Some(set_selected_text),
    SetLinkUnderCursor: Some(set_link_under_cursor),
    GetV8ExternalSnapshotData: Some(get_v8_external_snapshot_data),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_PDF_INTERFACE, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Interface functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn get_font_file_with_fallback(
    instance: PP_Instance,
    description: *const PP_BrowserFont_Trusted_Description,
    charset: PP_PrivateFontCharset,
) -> PP_Resource {
    tracing::trace!(
        "ppb_pdf_get_font_file_with_fallback: instance={}, charset={}",
        instance,
        charset
    );
    let _ = description;
    // TODO: delegate to flash_font_file::create when implemented
    0
}

unsafe extern "C" fn get_font_table_for_private_font_file(
    font_file: PP_Resource,
    table: u32,
    output: *mut c_void,
    output_length: *mut u32,
) -> bool {
    tracing::trace!(
        "ppb_pdf_get_font_table_for_private_font_file: font_file={}, table=0x{:08x}",
        font_file,
        table
    );
    let _ = (output, output_length);
    // TODO: delegate to flash_font_file::get_font_table when implemented
    false
}

unsafe extern "C" fn search_string(
    instance: PP_Instance,
    string: *const u16,
    term: *const u16,
    case_sensitive: bool,
    results: *mut *mut PP_PrivateFindResult,
    count: *mut i32,
) {
    tracing::trace!(
        "ppb_pdf_search_string: instance={}, string={:?}, term={:?}, case_sensitive={}",
        instance,
        string,
        term,
        case_sensitive
    );
    // No search implementation - return empty results.
    if !results.is_null() {
        unsafe { *results = std::ptr::null_mut() };
    }
    if !count.is_null() {
        unsafe { *count = 0 };
    }
}

unsafe extern "C" fn did_start_loading(instance: PP_Instance) {
    tracing::trace!("ppb_pdf_did_start_loading: instance={}", instance);
}

unsafe extern "C" fn did_stop_loading(instance: PP_Instance) {
    tracing::trace!("ppb_pdf_did_stop_loading: instance={}", instance);
}

unsafe extern "C" fn set_content_restriction(instance: PP_Instance, restrictions: i32) {
    tracing::trace!(
        "ppb_pdf_set_content_restriction: instance={}, restrictions={}",
        instance,
        restrictions
    );
}

unsafe extern "C" fn user_metrics_record_action(instance: PP_Instance, action: PP_Var) {
    tracing::trace!(
        "ppb_pdf_user_metrics_record_action: instance={}, action type={}",
        instance,
        action.type_
    );
}

unsafe extern "C" fn has_unsupported_feature(instance: PP_Instance) {
    tracing::trace!("ppb_pdf_has_unsupported_feature: instance={}", instance);
}

unsafe extern "C" fn save_as(instance: PP_Instance) {
    tracing::trace!("ppb_pdf_save_as: instance={}", instance);
}

unsafe extern "C" fn print(instance: PP_Instance) {
    tracing::debug!("ppb_pdf_print: instance={}", instance);
    let Some(host) = HOST.get() else {
        tracing::warn!("ppb_pdf_print: HOST not initialised");
        return;
    };
    if let Some(provider) = host.get_print_provider() {
        let ok = provider.print();
        tracing::debug!("ppb_pdf_print: provider returned {}", ok);
    } else {
        tracing::debug!("ppb_pdf_print: no print provider set, ignoring");
    }
}

unsafe extern "C" fn is_feature_enabled(
    instance: PP_Instance,
    feature: PP_PDFFeature,
) -> PP_Bool {
    let name = match feature {
        PP_PDFFEATURE_HIDPI => "HIDPI",
        PP_PDFFEATURE_PRINTING => "PRINTING",
        _ => "UNKNOWN",
    };

    let result = match feature {
        PP_PDFFEATURE_PRINTING => {
            // Report printing as enabled when a print provider is available.
            let has_provider = HOST
                .get()
                .and_then(|h| h.get_print_provider())
                .is_some();
            if has_provider { PP_TRUE } else { PP_FALSE }
        }
        _ => PP_FALSE,
    };

    tracing::trace!(
        "ppb_pdf_is_feature_enabled: instance={}, feature={}({}) -> {}",
        instance,
        name,
        feature,
        if result == PP_TRUE { "PP_TRUE" } else { "PP_FALSE" },
    );
    result
}

unsafe extern "C" fn set_selected_text(
    instance: PP_Instance,
    selected_text: *const std::ffi::c_char,
) {
    let text = if selected_text.is_null() {
        "<null>"
    } else {
        unsafe { std::ffi::CStr::from_ptr(selected_text) }
            .to_str()
            .unwrap_or("<invalid utf8>")
    };
    tracing::trace!(
        "ppb_pdf_set_selected_text: instance={}, text={:?}",
        instance,
        text
    );
}

unsafe extern "C" fn set_link_under_cursor(
    instance: PP_Instance,
    url: *const std::ffi::c_char,
) {
    let url_str = if url.is_null() {
        "<null>"
    } else {
        unsafe { std::ffi::CStr::from_ptr(url) }
            .to_str()
            .unwrap_or("<invalid utf8>")
    };
    tracing::trace!(
        "ppb_pdf_set_link_under_cursor: instance={}, url={:?}",
        instance,
        url_str
    );
}

unsafe extern "C" fn get_v8_external_snapshot_data(
    instance: PP_Instance,
    natives_data_out: *mut *const std::ffi::c_char,
    natives_size_out: *mut i32,
    snapshot_data_out: *mut *const std::ffi::c_char,
    snapshot_size_out: *mut i32,
) {
    tracing::trace!(
        "ppb_pdf_get_v8_external_snapshot_data: instance={}",
        instance
    );

    // No V8 snapshot blobs available in standalone player.
    if !natives_data_out.is_null() {
        unsafe { *natives_data_out = std::ptr::null() };
    }
    if !natives_size_out.is_null() {
        unsafe { *natives_size_out = 0 };
    }
    if !snapshot_data_out.is_null() {
        unsafe { *snapshot_data_out = std::ptr::null() };
    }
    if !snapshot_size_out.is_null() {
        unsafe { *snapshot_size_out = 0 };
    }
}
