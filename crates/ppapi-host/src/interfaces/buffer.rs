//! PPB_Buffer(Dev);0.4 implementation.
//!
//! Provides shared-memory-style byte buffers. The plugin can Create a buffer,
//! Map it to get a raw pointer, and Unmap when done. We use a simple Vec<u8>
//! allocation. Map returns a stable pointer (the Vec won't move while mapped).
//! We track a "map count" so that Map/Unmap are balanced via AddRef/Release.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::c_void;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

pub struct BufferResource {
    pub data: Vec<u8>,
    pub len: u32,
    /// Number of outstanding Map calls (each Map adds a ref, Unmap releases).
    pub map_count: i32,
}

impl Resource for BufferResource {
    fn resource_type(&self) -> &'static str {
        "PPB_Buffer"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Vtable
// ---------------------------------------------------------------------------

static VTABLE: PPB_Buffer_Dev_0_4 = PPB_Buffer_Dev_0_4 {
    Create: Some(create),
    IsBuffer: Some(is_buffer),
    Describe: Some(describe),
    Map: Some(map),
    Unmap: Some(unmap),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_BUFFER_DEV_INTERFACE_0_4, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

unsafe extern "C" fn create(instance: PP_Instance, size_in_bytes: u32) -> PP_Resource {
    let Some(host) = HOST.get() else {
        return 0;
    };

    if size_in_bytes == 0 {
        return 0;
    }

    let buf = BufferResource {
        data: vec![0u8; size_in_bytes as usize],
        len: size_in_bytes,
        map_count: 0,
    };

    host.resources.insert(instance, Box::new(buf))
}

unsafe extern "C" fn is_buffer(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_Buffer")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn describe(resource: PP_Resource, size_in_bytes: *mut u32) -> PP_Bool {
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    host.resources
        .with_downcast::<BufferResource, _>(resource, |buf| {
            if !size_in_bytes.is_null() {
                unsafe {
                    *size_in_bytes = buf.len;
                }
            }
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn map(resource: PP_Resource) -> *mut c_void {
    let Some(host) = HOST.get() else {
        return std::ptr::null_mut();
    };

    // Add a reference to keep the buffer alive while mapped.
    host.resources.add_ref(resource);

    host.resources
        .with_downcast_mut::<BufferResource, _>(resource, |buf| {
            buf.map_count += 1;
            buf.data.as_mut_ptr() as *mut c_void
        })
        .unwrap_or_else(|| {
            // Undo the add_ref if we couldn't find the buffer.
            host.resources.release(resource);
            std::ptr::null_mut()
        })
}

unsafe extern "C" fn unmap(resource: PP_Resource) {
    let Some(host) = HOST.get() else {
        return;
    };

    // Only release if this is actually a buffer.
    let is_buffer = host
        .resources
        .with_downcast_mut::<BufferResource, _>(resource, |buf| {
            if buf.map_count > 0 {
                buf.map_count -= 1;
            }
        })
        .is_some();

    if is_buffer {
        host.resources.release(resource);
    }
}
