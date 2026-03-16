//! PPB_Memory(Dev);0.1 implementation.
//!
//! Every allocation prepends an 8-byte header that stores the total
//! allocation size (header + payload). This allows `MemFree` to
//! reconstruct the layout and properly deallocate.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::c_void;

/// Size of the header prepended to every allocation (stores total size).
const HEADER_SIZE: usize = std::mem::size_of::<usize>();
const ALIGN: usize = 8;

static VTABLE: PPB_Memory_Dev_0_1 = PPB_Memory_Dev_0_1 {
    MemAlloc: Some(mem_alloc),
    MemFree: Some(mem_free),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_MEMORY_DEV_INTERFACE_0_1, &VTABLE);
    }
}

/// Allocate `size` bytes (zero-initialized) using the PPB_Memory layout.
///
/// The returned pointer can be freed with [`ppb_mem_free`]. This is the
/// same allocator backing `PPB_Memory::MemAlloc`, so the plugin can also
/// free it via `PPB_Memory::MemFree`.
pub fn ppb_mem_alloc(size: usize) -> *mut u8 {
    if size == 0 {
        return std::ptr::null_mut();
    }
    let total = HEADER_SIZE + size;
    let layout = std::alloc::Layout::from_size_align(total, ALIGN)
        .expect("invalid allocation layout");
    let raw = unsafe { std::alloc::alloc_zeroed(layout) };
    if raw.is_null() {
        return std::ptr::null_mut();
    }
    // Store total allocation size in the header.
    unsafe { *(raw as *mut usize) = total };
    unsafe { raw.add(HEADER_SIZE) }
}

/// Free a pointer previously returned by [`ppb_mem_alloc`] or
/// `PPB_Memory::MemAlloc`. Null pointers are ignored.
pub unsafe fn ppb_mem_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    let raw = unsafe { ptr.sub(HEADER_SIZE) };
    let total = unsafe { *(raw as *const usize) };
    let layout = std::alloc::Layout::from_size_align(total, ALIGN)
        .expect("invalid deallocation layout");
    unsafe { std::alloc::dealloc(raw, layout) };
}

unsafe extern "C" fn mem_alloc(num_bytes: u32) -> *mut c_void {
    ppb_mem_alloc(num_bytes as usize) as *mut c_void
}

unsafe extern "C" fn mem_free(ptr: *mut c_void) {
    unsafe { ppb_mem_free(ptr as *mut u8) };
}
