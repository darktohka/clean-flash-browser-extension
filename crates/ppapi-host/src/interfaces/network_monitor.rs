//! PPB_NetworkMonitor;1.0 implementation.
//!
//! Provides network monitoring resources. Flash queries this to detect
//! network configuration changes. In our standalone player, we create valid
//! resources and respond to type-checks, but UpdateNetworkList returns
//! PP_ERROR_NOACCESS (no network monitoring permissions), matching the
//! freshplayerplugin behaviour.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Network monitor resource (no actual monitoring is performed).
pub struct NetworkMonitorResource {
    pub instance: PP_Instance,
}

impl Resource for NetworkMonitorResource {
    fn resource_type(&self) -> &'static str {
        "PPB_NetworkMonitor"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// VTable
// ---------------------------------------------------------------------------

static VTABLE: PPB_NetworkMonitor_1_0 = PPB_NetworkMonitor_1_0 {
    Create: Some(create),
    UpdateNetworkList: Some(update_network_list),
    IsNetworkMonitor: Some(is_network_monitor),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_NETWORKMONITOR_INTERFACE_1_0, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Interface functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    let host = HOST.get().unwrap();

    if !host.instances.exists(instance) {
        tracing::error!("ppb_network_monitor_create: bad instance {}", instance);
        return 0;
    }

    let resource = NetworkMonitorResource { instance };
    let id = host.resources.insert(instance, Box::new(resource));
    tracing::debug!(
        "ppb_network_monitor_create: instance={} -> resource={}",
        instance,
        id
    );
    id
}

unsafe extern "C" fn update_network_list(
    _network_monitor: PP_Resource,
    _network_list: *mut PP_Resource,
    _callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("ppb_network_monitor_update_network_list: returning PP_ERROR_NOACCESS");
    PP_ERROR_NOACCESS
}

unsafe extern "C" fn is_network_monitor(resource: PP_Resource) -> PP_Bool {
    let host = HOST.get().unwrap();
    if host.resources.is_type(resource, "PPB_NetworkMonitor") {
        PP_TRUE
    } else {
        PP_FALSE
    }
}
