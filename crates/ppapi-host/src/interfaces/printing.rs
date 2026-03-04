//! PPB_Printing(Dev);0.7 implementation.
//!
//! Flash queries this interface for print capabilities. As a standalone player,
//! we don't support printing, so Create returns a valid resource but
//! GetDefaultPrintSettings returns PP_OK immediately with zeroed settings
//! (same approach as freshplayerplugin).

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

/// Printing resource — mostly empty, just satisfies the interface contract.
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
    _callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!(
        "PPB_Printing::GetDefaultPrintSettings(resource={})",
        resource
    );
    // Zero out the print settings — we don't support printing.
    if !print_settings.is_null() {
        unsafe {
            *print_settings = PP_PrintSettings_Dev::default();
        }
    }
    PP_OK
}
