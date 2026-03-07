//! PPB_Crypto(Dev);0.1 implementation.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use rand::rngs::SysRng;
use rand::TryRng;
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

    let slice = unsafe { std::slice::from_raw_parts_mut(buffer as *mut u8, num_bytes as usize) };

    let mut rng = SysRng;
    if let Err(err) = rng.try_fill_bytes(slice) {
        tracing::error!(?err, "PPB_Crypto::GetRandomBytes failed");
        slice.fill(0);
    }
}
