//! PPB_Crypto(Dev);0.1 implementation.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::c_char;

static VTABLE: PPB_Crypto_Dev_0_1 = PPB_Crypto_Dev_0_1 {
    GetRandomBytes: Some(get_random_bytes),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_CRYPTO_DEV_INTERFACE_0_1, &VTABLE);
    }
}

unsafe extern "C" fn get_random_bytes(buffer: *mut c_char, num_bytes: u32) {
    if buffer.is_null() || num_bytes == 0 {
        return;
    }

    // Fill the buffer with random bytes.
    let slice = unsafe { std::slice::from_raw_parts_mut(buffer as *mut u8, num_bytes as usize) };

    // Use RtlGenRandom on Windows, and getrandom() on Unix.
    #[cfg(windows)]
    {
        #[link(name = "advapi32")]
        extern "system" {
            fn SystemFunction036(buffer: *mut u8, len: u32) -> u8;
        }

        let result = unsafe { SystemFunction036(slice.as_mut_ptr(), num_bytes) };
        if result == 0 {
            tracing::error!("RtlGenRandom failed");
        }
    }

    #[cfg(unix)]
    {
        use getrandom::getrandom;
        if let Err(e) = getrandom(slice) {
            tracing::error!("Failed to get random bytes: {}", e);
        }
    }

    tracing::trace!("PPB_Crypto(Dev)::GetRandomBytes(num_bytes={})", num_bytes);
}
