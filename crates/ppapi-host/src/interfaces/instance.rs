//! PPB_Instance;1.0 implementation.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;

use super::super::HOST;

static VTABLE: PPB_Instance_1_0 = PPB_Instance_1_0 {
    BindGraphics: Some(bind_graphics),
    IsFullFrame: Some(is_full_frame),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_INSTANCE_INTERFACE_1_0, &VTABLE);
    }
}

unsafe extern "C" fn bind_graphics(instance: PP_Instance, device: PP_Resource) -> PP_Bool {
    tracing::trace!("BindGraphics called with instance {} and device {}", instance, device);
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    if device == 0 {
        // Unbind current graphics.
        host.instances
            .with_instance_mut(instance, |inst| {
                inst.bound_graphics = 0;
            });
        return PP_TRUE;
    }

    // Verify the device is a Graphics2D resource and belongs to this instance.
    let is_valid = host.resources.with_resource(device, |entry| {
        entry.instance == instance
            && (entry.resource.resource_type() == "PPB_Graphics2D"
                || entry.resource.resource_type() == "PPB_Graphics3D")
    }).unwrap_or(false);

    if !is_valid {
        tracing::warn!("BindGraphics: device {} is not a valid graphics resource for instance {}", device, instance);
        return PP_FALSE;
    }

    // Release old device ref, add ref to new device.
    let old_device = host.instances.with_instance(instance, |inst| inst.bound_graphics).unwrap_or(0);
    if old_device != 0 && old_device != device {
        host.resources.release(old_device);
    }
    if device != old_device {
        host.resources.add_ref(device);
    }

    host.instances
        .with_instance_mut(instance, |inst| {
            inst.bound_graphics = device;
        });

    tracing::debug!("BindGraphics: instance {} bound to device {}", instance, device);
    PP_TRUE
}

unsafe extern "C" fn is_full_frame(_instance: PP_Instance) -> PP_Bool {
    tracing::trace!("IsFullFrame called");
    // In our projector, the Flash content always fills the full frame.
    PP_TRUE
}
