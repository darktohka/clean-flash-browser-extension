//! PPB_VideoCapture(Dev);0.3 implementation.
//!
//! The video capture interface allows plugins to access webcam / video capture
//! devices. In our standalone Flash player context, video capture is not
//! typically used by SWF content — this is primarily for Flash-based webcam
//! applications. We create valid resources so the plugin's interface
//! availability check passes, but `Open` returns `PP_ERROR_NOACCESS` (no
//! camera available), `EnumerateDevices` returns an empty list, and capture
//! operations fail gracefully. All calls are traced.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::c_void;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

pub struct VideoCaptureResource {
    pub instance: PP_Instance,
}

impl Resource for VideoCaptureResource {
    fn resource_type(&self) -> &'static str {
        "PPB_VideoCapture(Dev)"
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

static VTABLE: PPB_VideoCapture_Dev_0_3 = PPB_VideoCapture_Dev_0_3 {
    Create: Some(create),
    IsVideoCapture: Some(is_video_capture),
    EnumerateDevices: Some(enumerate_devices),
    MonitorDeviceChange: Some(monitor_device_change),
    Open: Some(open),
    StartCapture: Some(start_capture),
    ReuseBuffer: Some(reuse_buffer),
    StopCapture: Some(stop_capture),
    Close: Some(close),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_VIDEOCAPTURE_DEV_INTERFACE_0_3, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Interface functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    let host = HOST.get().unwrap();

    if !host.instances.exists(instance) {
        tracing::error!("ppb_video_capture_create: bad instance {}", instance);
        return 0;
    }

    let resource = VideoCaptureResource { instance };
    let id = host.resources.insert(instance, Box::new(resource));
    tracing::trace!("ppb_video_capture_create: instance={} -> resource={}", instance, id);
    id
}

unsafe extern "C" fn is_video_capture(video_capture: PP_Resource) -> PP_Bool {
    let host = HOST.get().unwrap();
    let result = if host.resources.is_type(video_capture, "PPB_VideoCapture(Dev)") {
        PP_TRUE
    } else {
        PP_FALSE
    };
    tracing::trace!(
        "ppb_video_capture_is_video_capture: resource={} -> {}",
        video_capture,
        result
    );
    result
}

unsafe extern "C" fn enumerate_devices(
    video_capture: PP_Resource,
    output: PP_ArrayOutput,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::trace!(
        "ppb_video_capture_enumerate_devices: resource={}",
        video_capture
    );

    let host = HOST.get().unwrap();
    if !host.resources.is_type(video_capture, "PPB_VideoCapture(Dev)") {
        tracing::error!(
            "ppb_video_capture_enumerate_devices: bad resource {}",
            video_capture
        );
        return PP_ERROR_BADRESOURCE;
    }

    // Return an empty device list — no video capture devices available.
    if let Some(get_data_buffer) = output.GetDataBuffer {
        unsafe {
            get_data_buffer(output.user_data, 0, std::mem::size_of::<PP_Resource>() as u32);
        }
    }

    // Fire callback with PP_OK.
    if let Some(func) = callback.func {
        unsafe {
            func(callback.user_data, PP_OK);
        }
    }

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn monitor_device_change(
    video_capture: PP_Resource,
    callback: PP_MonitorDeviceChangeCallback,
    user_data: *mut c_void,
) -> i32 {
    tracing::trace!(
        "ppb_video_capture_monitor_device_change: resource={}, callback={:?}, user_data={:?}",
        video_capture,
        callback.map(|f| f as *const ()),
        user_data
    );

    let host = HOST.get().unwrap();
    if !host.resources.is_type(video_capture, "PPB_VideoCapture(Dev)") {
        return PP_ERROR_BADRESOURCE;
    }

    // No device monitoring — silently succeed.
    PP_OK
}

unsafe extern "C" fn open(
    video_capture: PP_Resource,
    device_ref: PP_Resource,
    requested_info: *const PP_VideoCaptureDeviceInfo_Dev,
    buffer_count: u32,
    callback: PP_CompletionCallback,
) -> i32 {
    let info = if requested_info.is_null() {
        None
    } else {
        Some(unsafe { &*requested_info })
    };
    tracing::trace!(
        "ppb_video_capture_open: resource={}, device_ref={}, info={:?}, buffer_count={}",
        video_capture,
        device_ref,
        info,
        buffer_count
    );

    let host = HOST.get().unwrap();
    if !host.resources.is_type(video_capture, "PPB_VideoCapture(Dev)") {
        tracing::error!("ppb_video_capture_open: bad resource {}", video_capture);
        return PP_ERROR_BADRESOURCE;
    }

    // No video capture hardware — fire callback with PP_ERROR_NOACCESS.
    if let Some(func) = callback.func {
        unsafe {
            func(callback.user_data, PP_ERROR_NOACCESS);
        }
    }

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn start_capture(video_capture: PP_Resource) -> i32 {
    tracing::trace!("ppb_video_capture_start_capture: resource={}", video_capture);

    let host = HOST.get().unwrap();
    if !host.resources.is_type(video_capture, "PPB_VideoCapture(Dev)") {
        return PP_ERROR_BADRESOURCE;
    }

    // Device was never opened successfully.
    PP_ERROR_FAILED
}

unsafe extern "C" fn reuse_buffer(video_capture: PP_Resource, buffer: u32) -> i32 {
    tracing::trace!(
        "ppb_video_capture_reuse_buffer: resource={}, buffer={}",
        video_capture,
        buffer
    );

    let host = HOST.get().unwrap();
    if !host.resources.is_type(video_capture, "PPB_VideoCapture(Dev)") {
        return PP_ERROR_BADRESOURCE;
    }

    PP_ERROR_BADARGUMENT
}

unsafe extern "C" fn stop_capture(video_capture: PP_Resource) -> i32 {
    tracing::trace!("ppb_video_capture_stop_capture: resource={}", video_capture);

    let host = HOST.get().unwrap();
    if !host.resources.is_type(video_capture, "PPB_VideoCapture(Dev)") {
        return PP_ERROR_BADRESOURCE;
    }

    PP_ERROR_FAILED
}

unsafe extern "C" fn close(video_capture: PP_Resource) {
    tracing::trace!("ppb_video_capture_close: resource={}", video_capture);

    let host = HOST.get().unwrap();
    // Release the resource (decrement ref count; drops when it reaches zero).
    host.resources.release(video_capture);
}
