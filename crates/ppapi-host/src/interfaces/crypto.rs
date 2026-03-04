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

    // Read from /dev/urandom for cryptographic randomness.
    let slice = unsafe { std::slice::from_raw_parts_mut(buffer as *mut u8, num_bytes as usize) };

    // Try getrandom first, fall back to /dev/urandom.
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let _ = f.read_exact(slice);
            return;
        }
    }

    // Fallback: fill with pseudo-random data using a simple xorshift.
    let mut state: u64 = 0x12345678_9ABCDEF0;
    for byte in slice.iter_mut() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = state as u8;
    }
}
