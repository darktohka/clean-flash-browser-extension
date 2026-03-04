//! PPB_BrokerTrusted;0.3 / 0.2 implementation.
//!
//! The broker interface provides access to a trusted broker process with
//! greater privileges. In a standalone player there is no separate broker
//! process — we are already running with full privileges. We create valid
//! resources so the plugin's interface availability check passes, `Connect`
//! completes immediately with PP_OK, `GetHandle` returns PP_ERROR_FAILED
//! (no pipe), and `IsAllowed` returns PP_TRUE (the user implicitly trusts
//! the standalone player).

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Broker resource — no real broker process behind it.
pub struct BrokerResource {
    pub instance: PP_Instance,
    pub connected: bool,
}

impl Resource for BrokerResource {
    fn resource_type(&self) -> &'static str {
        "PPB_BrokerTrusted"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// VTables
// ---------------------------------------------------------------------------

static VTABLE_0_3: PPB_BrokerTrusted_0_3 = PPB_BrokerTrusted_0_3 {
    CreateTrusted: Some(create_trusted),
    IsBrokerTrusted: Some(is_broker_trusted),
    Connect: Some(connect),
    GetHandle: Some(get_handle),
    IsAllowed: Some(is_allowed),
};

static VTABLE_0_2: PPB_BrokerTrusted_0_2 = PPB_BrokerTrusted_0_2 {
    CreateTrusted: Some(create_trusted),
    IsBrokerTrusted: Some(is_broker_trusted),
    Connect: Some(connect),
    GetHandle: Some(get_handle),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_BROKER_TRUSTED_INTERFACE_0_3, &VTABLE_0_3);
        registry.register(PPB_BROKER_TRUSTED_INTERFACE_0_2, &VTABLE_0_2);
    }
}

// ---------------------------------------------------------------------------
// Interface functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn create_trusted(instance: PP_Instance) -> PP_Resource {
    let host = HOST.get().unwrap();

    if !host.instances.exists(instance) {
        tracing::error!("ppb_broker_create_trusted: bad instance {}", instance);
        return 0;
    }

    let resource = BrokerResource {
        instance,
        connected: false,
    };
    let id = host.resources.insert(instance, Box::new(resource));
    tracing::debug!(
        "ppb_broker_create_trusted: instance={} -> resource={}",
        instance,
        id
    );
    id
}

unsafe extern "C" fn is_broker_trusted(resource: PP_Resource) -> PP_Bool {
    let host = HOST.get().unwrap();
    if host.resources.is_type(resource, "PPB_BrokerTrusted") {
        PP_TRUE
    } else {
        PP_FALSE
    }
}

unsafe extern "C" fn connect(
    broker: PP_Resource,
    connect_callback: PP_CompletionCallback,
) -> i32 {
    let host = HOST.get().unwrap();

    let ok = host
        .resources
        .with_downcast_mut::<BrokerResource, _>(broker, |b| {
            b.connected = true;
        })
        .is_some();

    if !ok {
        tracing::error!("ppb_broker_connect: bad resource {}", broker);
        return PP_ERROR_BADRESOURCE;
    }

    tracing::debug!("ppb_broker_connect: resource={} -> immediate PP_OK", broker);

    // Fire the completion callback with PP_OK immediately.
    if let Some(func) = connect_callback.func {
        unsafe {
            func(connect_callback.user_data, PP_OK);
        }
    }

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn get_handle(broker: PP_Resource, handle: *mut i32) -> i32 {
    let host = HOST.get().unwrap();

    let connected = host
        .resources
        .with_downcast::<BrokerResource, _>(broker, |b| b.connected);

    match connected {
        Some(true) => {
            // In a standalone player there is no broker pipe. Return an
            // invalid handle. The plugin typically only needs Connect to
            // succeed; the actual pipe handle is used for NaCl/out-of-process
            // communication which doesn't apply here.
            if !handle.is_null() {
                unsafe { *handle = -1 };
            }
            tracing::debug!("ppb_broker_get_handle: resource={} -> handle=-1", broker);
            PP_OK
        }
        Some(false) => {
            tracing::warn!("ppb_broker_get_handle: not connected yet");
            PP_ERROR_FAILED
        }
        None => {
            tracing::error!("ppb_broker_get_handle: bad resource {}", broker);
            PP_ERROR_BADRESOURCE
        }
    }
}

unsafe extern "C" fn is_allowed(broker: PP_Resource) -> PP_Bool {
    let host = HOST.get().unwrap();
    if host.resources.is_type(broker, "PPB_BrokerTrusted") {
        // Standalone player — always allowed.
        PP_TRUE
    } else {
        PP_FALSE
    }
}
