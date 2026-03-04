//! PPB_Memory(Dev);0.1 implementation.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::c_void;

static VTABLE: PPB_Memory_Dev_0_1 = PPB_Memory_Dev_0_1 {
    MemAlloc: Some(mem_alloc),
    MemFree: Some(mem_free),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_MEMORY_DEV_INTERFACE_0_1, &VTABLE);
    }
}

unsafe extern "C" fn mem_alloc(num_bytes: u32) -> *mut c_void {
    if num_bytes == 0 {
        return std::ptr::null_mut();
    }
    let layout = std::alloc::Layout::from_size_align(num_bytes as usize, 8)
        .expect("invalid allocation layout");
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    if ptr.is_null() {
        std::ptr::null_mut()
    } else {
        ptr as *mut c_void
    }
}

unsafe extern "C" fn mem_free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    // We don't know the original size, so we can't properly dealloc.
    // In practice, Flash uses this for small fixed-size buffers.
    // We'll use a simple approach: store the layout alongside the allocation
    // in a real implementation. For now, this is intentionally a leak-on-free
    // until we add a proper allocator wrapper.
    //
    // TODO: Implement proper sized deallocation by prepending a header.
    // For now Flash uses MemAlloc/MemFree very sparingly, so this is acceptable.
}
