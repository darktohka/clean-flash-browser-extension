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
    tracing::info!("BindGraphics called with instance {} and device {}", instance, device);
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    if device == 0 {
        // Unbind all graphics.
        let (old_2d, old_3d) = host.instances
            .with_instance_mut(instance, |inst| {
                let o2 = inst.bound_graphics_2d;
                let o3 = inst.bound_graphics_3d;
                inst.bound_graphics_2d = 0;
                inst.bound_graphics_3d = 0;
                (o2, o3)
            })
            .unwrap_or((0, 0));
        if old_2d != 0 { host.resources.release(old_2d); }
        if old_3d != 0 { host.resources.release(old_3d); }
        return PP_TRUE;
    }

    // Verify the device is a graphics resource and belongs to this instance.
    let resource_type = host.resources.with_resource(device, |entry| {
        if entry.instance != instance {
            return None;
        }
        let ty = entry.resource.resource_type();
        if ty == "PPB_Graphics2D" || ty == "PPB_Graphics3D" {
            Some(ty.to_string())
        } else {
            None
        }
    }).flatten();

    let Some(res_type) = resource_type else {
        tracing::warn!("BindGraphics: device {} is not a valid graphics resource for instance {}", device, instance);
        return PP_FALSE;
    };

    let is_3d = res_type == "PPB_Graphics3D";

    // Release old device of the same type, add ref to new device.
    let old_device = host.instances.with_instance(instance, |inst| {
        if is_3d { inst.bound_graphics_3d } else { inst.bound_graphics_2d }
    }).unwrap_or(0);
    if old_device != 0 && old_device != device {
        host.resources.release(old_device);
    }
    if device != old_device {
        host.resources.add_ref(device);
    }

    host.instances
        .with_instance_mut(instance, |inst| {
            if is_3d {
                inst.bound_graphics_3d = device;
            } else {
                inst.bound_graphics_2d = device;
            }
        });

    tracing::debug!("BindGraphics: instance {} bound {} to device {}", instance, res_type, device);
    PP_TRUE
}

unsafe extern "C" fn is_full_frame(_instance: PP_Instance) -> PP_Bool {
    tracing::trace!("IsFullFrame called");
    // In our projector, the Flash content always fills the full frame.
    PP_TRUE
}
