//! PPB_Flash_Print;1.0 implementation.
//!
//! Flash calls `InvokePrinting` to trigger a print of the current content.
//! This delegates to the registered `PrintProvider` on the host, matching
//! Chrome's `ChromePDFPrintClient::Print` flow.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;

use super::super::HOST;

static VTABLE: PPB_Flash_Print_1_0 = PPB_Flash_Print_1_0 {
    InvokePrinting: Some(invoke_printing),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASH_PRINT_INTERFACE_1_0, &VTABLE);
    }
}

unsafe extern "C" fn invoke_printing(instance: PP_Instance) {
    tracing::debug!("PPB_Flash_Print::InvokePrinting(instance={})", instance);
    let Some(host) = HOST.get() else {
        tracing::warn!("PPB_Flash_Print::InvokePrinting: HOST not initialised");
        return;
    };
    if let Some(provider) = host.get_print_provider() {
        let ok = provider.print();
        tracing::debug!("PPB_Flash_Print::InvokePrinting: provider returned {}", ok);
    } else {
        tracing::debug!("PPB_Flash_Print::InvokePrinting: no print provider set, ignoring");
    }
}
