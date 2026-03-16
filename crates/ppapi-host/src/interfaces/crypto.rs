//! PPB_Crypto(Dev);0.1 implementation.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use rand::TryRng;
use rand::rngs::SysRng;
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

    // SAFETY: caller guarantees `buffer` points to a valid allocation of at
    // least `num_bytes` bytes and that no other thread aliases it during this
    // call.
    let slice = unsafe { std::slice::from_raw_parts_mut(buffer as *mut u8, num_bytes as usize) };

    // SysRng delegates to the OS CSPRNG (getrandom(2) / BCryptGenRandom /
    // SecRandomCopyBytes) and is appropriate for security-sensitive use.
    if let Err(e) = SysRng.try_fill_bytes(slice) {
        tracing::error!("PPB_Crypto(Dev)::GetRandomBytes: SysRng failed: {}", e);
    }

    tracing::trace!("PPB_Crypto(Dev)::GetRandomBytes(num_bytes={})", num_bytes);
}
