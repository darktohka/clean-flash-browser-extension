//! PPB_Printing(Dev);0.7 implementation.
//!
//! Flash queries this interface for print capabilities.  When a
//! `PrintProvider` is registered on the host, `GetDefaultPrintSettings`
//! returns realistic page dimensions from the provider.  Otherwise it
//! falls back to sensible defaults (US Letter, 72 DPI).

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

/// Printing resource - mostly empty, just satisfies the interface contract.
pub struct PrintingResource;

impl Resource for PrintingResource {
    fn resource_type(&self) -> &'static str {
        "PPB_Printing"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

static VTABLE: PPB_Printing_Dev_0_7 = PPB_Printing_Dev_0_7 {
    Create: Some(create),
    GetDefaultPrintSettings: Some(get_default_print_settings),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_PRINTING_DEV_INTERFACE_0_7, &VTABLE);
    }
}

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    tracing::debug!("PPB_Printing::Create(instance={})", instance);
    let Some(host) = HOST.get() else {
        return 0;
    };
    host.resources.insert(instance, Box::new(PrintingResource))
}

unsafe extern "C" fn get_default_print_settings(
    resource: PP_Resource,
    print_settings: *mut PP_PrintSettings_Dev,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!(
        "PPB_Printing::GetDefaultPrintSettings(resource={})",
        resource
    );

    if print_settings.is_null() {
        return crate::callback::complete_immediately(callback, PP_OK);
    }

    let Some(host) = HOST.get() else {
        unsafe { *print_settings = PP_PrintSettings_Dev::default() };
        return crate::callback::complete_immediately(callback, PP_OK);
    };

    if let Some(provider) = host.get_print_provider() {
        let ps = provider.get_default_print_settings();
        unsafe {
            (*print_settings).printable_area = PP_Rect {
                point: PP_Point {
                    x: ps.printable_area.0,
                    y: ps.printable_area.1,
                },
                size: PP_Size {
                    width: ps.printable_area.2,
                    height: ps.printable_area.3,
                },
            };
            (*print_settings).content_area = PP_Rect {
                point: PP_Point {
                    x: ps.content_area.0,
                    y: ps.content_area.1,
                },
                size: PP_Size {
                    width: ps.content_area.2,
                    height: ps.content_area.3,
                },
            };
            (*print_settings).paper_size = PP_Size {
                width: ps.paper_size.0,
                height: ps.paper_size.1,
            };
            (*print_settings).dpi = ps.dpi;
            (*print_settings).orientation = PP_PRINTORIENTATION_NORMAL;
            (*print_settings).grayscale = PP_FALSE;
            (*print_settings).print_scaling_option = PP_PRINTSCALINGOPTION_SOURCE_SIZE;
            (*print_settings).format = PP_PRINTOUTPUTFORMAT_PDF;
        }
        tracing::debug!(
            "PPB_Printing: returning settings from provider: paper={}x{}, dpi={}",
            ps.paper_size.0,
            ps.paper_size.1,
            ps.dpi
        );
    } else {
        // No provider - return sensible defaults (US Letter, 72 DPI).
        let defaults = player_ui_traits::PrintSettings::default();
        unsafe {
            (*print_settings).printable_area = PP_Rect {
                point: PP_Point {
                    x: defaults.printable_area.0,
                    y: defaults.printable_area.1,
                },
                size: PP_Size {
                    width: defaults.printable_area.2,
                    height: defaults.printable_area.3,
                },
            };
            (*print_settings).content_area = PP_Rect {
                point: PP_Point {
                    x: defaults.content_area.0,
                    y: defaults.content_area.1,
                },
                size: PP_Size {
                    width: defaults.content_area.2,
                    height: defaults.content_area.3,
                },
            };
            (*print_settings).paper_size = PP_Size {
                width: defaults.paper_size.0,
                height: defaults.paper_size.1,
            };
            (*print_settings).dpi = defaults.dpi;
            (*print_settings).orientation = PP_PRINTORIENTATION_NORMAL;
            (*print_settings).grayscale = PP_FALSE;
            (*print_settings).print_scaling_option = PP_PRINTSCALINGOPTION_SOURCE_SIZE;
            (*print_settings).format = PP_PRINTOUTPUTFORMAT_PDF;
        }
        tracing::debug!("PPB_Printing: no print provider, returning default settings");
    }
    crate::callback::complete_immediately(callback, PP_OK)
}
