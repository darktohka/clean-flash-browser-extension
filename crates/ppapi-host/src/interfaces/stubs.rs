//! Stub implementations for all PPAPI interfaces required by PepperFlash
//! that don't yet have full implementations.
//!
//! These stubs return a valid (non-null) vtable so the plugin passes its
//! init-time interface availability check. Individual functions are no-ops
//! that return 0/null/PP_FALSE. As we implement real functionality, we
//! move interfaces out of this file into dedicated modules.

use crate::interface_registry::InterfaceRegistry;
use std::ffi::c_void;

// ---------------------------------------------------------------------------
// Generic stub function — returns 0 which maps to:
//   PP_FALSE, null pointer, PP_Resource(0), PP_ERROR_FAILED, etc.
// On x86_64 SysV ABI, extra arguments are harmlessly ignored.
// ---------------------------------------------------------------------------

unsafe extern "C" fn stub() -> usize {
    tracing::trace!("STUB function called!");
    0
}

/// A stub function pointer, as a const for array init.
const STUB_FN: unsafe extern "C" fn() -> usize = stub;

// ---------------------------------------------------------------------------
// Macro to declare a static stub vtable of N function pointers and register
// one or more interface name strings to it.
// ---------------------------------------------------------------------------

macro_rules! stub_vtable {
    ($name:ident, $count:expr, [ $($iface:expr),+ $(,)? ]) => {
        static $name: [unsafe extern "C" fn() -> usize; $count] = [STUB_FN; $count];
    };
}

// ---------------------------------------------------------------------------
// Declare all stub vtables
// ---------------------------------------------------------------------------

// PPB_AudioConfig, PPB_Audio, PPB_AudioInput — moved to dedicated modules

// PPB_BrowserFont_Trusted — moved to browser_font.rs
// PPB_Buffer(Dev) — moved to buffer.rs
// PPB_CharSet(Dev) — moved to char_set.rs
// PPB_CursorControl(Dev) — moved to cursor_control.rs

// PPB_FileChooser(Dev);0.6 — 3 functions
stub_vtable!(FILE_CHOOSER_STUB, 3, ["PPB_FileChooser(Dev);0.6", "PPB_FileChooser(Dev);0.5"]);

// PPB_FileChooserTrusted;0.6 — 1 function
stub_vtable!(FILE_CHOOSER_TRUSTED_STUB, 1, ["PPB_FileChooserTrusted;0.6", "PPB_FileChooserTrusted;0.5"]);

// PPB_FileRef;1.2 / 1.1 / 1.0 — 12 functions
stub_vtable!(FILE_REF_STUB, 12, ["PPB_FileRef;1.2", "PPB_FileRef;1.1", "PPB_FileRef;1.0"]);

// PPB_Flash_File_FileRef;2 — 2 functions
stub_vtable!(FLASH_FILE_FILEREF_STUB, 2, ["PPB_Flash_File_FileRef;2"]);

// PPB_Flash_FontFile;0.2 / 0.1 — 4 functions
stub_vtable!(FLASH_FONT_FILE_STUB, 4, ["PPB_Flash_FontFile;0.2", "PPB_Flash_FontFile;0.1"]);

// PPB_Flash_Menu;0.2 — 3 functions
stub_vtable!(FLASH_MENU_STUB, 3, ["PPB_Flash_Menu;0.2"]);

// PPB_Graphics3D;1.0 — 8 functions
stub_vtable!(GRAPHICS3D_STUB, 8, ["PPB_Graphics3D;1.0"]);

// PPB_IMEInputEvent(Dev);0.2 / 0.1 — 7 functions
stub_vtable!(IME_INPUT_EVENT_STUB, 7, ["PPB_IMEInputEvent(Dev);0.2", "PPB_IMEInputEvent(Dev);0.1"]);

// PPB_NetAddress_Private;1.1 / 1.0 / 0.1 — 11 functions
stub_vtable!(NET_ADDRESS_STUB, 11, ["PPB_NetAddress_Private;1.1", "PPB_NetAddress_Private;1.0", "PPB_NetAddress_Private;0.1"]);

// PPB_OpenGLES2;1.0 — moved to opengles2.rs

// PPB_OpenGLES2ChromiumMapSub;1.0 — moved to opengles2.rs
// Also register the legacy Dev variant names
stub_vtable!(OPENGLES2_CHROMIUM_DEV_STUB, 4, ["PPB_OpenGLES2ChromiumMapSub(Dev);1.0", "PPB_GLESChromiumTextureMapping(Dev);0.1"]);

// PPB_TCPSocket_Private;0.5 / 0.4 / 0.3 — 13 functions
stub_vtable!(TCP_SOCKET_STUB, 13, ["PPB_TCPSocket_Private;0.5", "PPB_TCPSocket_Private;0.4", "PPB_TCPSocket_Private;0.3"]);

// PPB_TextInput(Dev);0.2 / 0.1 — 5 functions
stub_vtable!(TEXT_INPUT_STUB, 5, ["PPB_TextInput(Dev);0.2", "PPB_TextInput(Dev);0.1"]);

// PPB_UDPSocket_Private;0.4 / 0.3 — 9 functions
stub_vtable!(UDP_SOCKET_STUB, 9, ["PPB_UDPSocket_Private;0.4", "PPB_UDPSocket_Private;0.3"]);



// PPB_VideoCapture(Dev);0.3 — 9 functions
stub_vtable!(VIDEO_CAPTURE_STUB, 9, ["PPB_VideoCapture(Dev);0.3"]);

// PPB_PDF;1 — fallback for font file (32 functions, generous)
stub_vtable!(PDF_STUB, 32, ["PPB_PDF;1"]);

// Additional interfaces requested post-init
// PPB_OpenGLES2 extensions and PPB_Printing — moved to opengles2.rs / printing.rs
stub_vtable!(BROKER_TRUSTED_STUB, 4, ["PPB_BrokerTrusted;0.3"]);
stub_vtable!(VAR_DEPRECATED_STUB, 16, ["PPB_Var(Deprecated);0.3"]);
stub_vtable!(AUDIO_OUTPUT_STUB, 9, ["PPB_AudioOutput(Dev);0.1"]);
stub_vtable!(NETWORK_MONITOR_STUB, 4, ["PPB_NetworkMonitor;1.0"]);
stub_vtable!(INSTANCE_PRIVATE_STUB, 4, ["PPB_Instance_Private;0.1"]);

// ---------------------------------------------------------------------------
// Registration — macro to register all versions of each stub vtable
// ---------------------------------------------------------------------------

macro_rules! register_stub {
    ($registry:expr, $vtable:expr, [ $($iface:expr),+ $(,)? ]) => {
        $(
            $registry.register_raw($iface, $vtable.as_ptr() as *const c_void);
        )+
    };
}

/// Register all stub interfaces.
pub fn register(registry: &mut InterfaceRegistry) {
    unsafe {
    // PPB_AudioConfig, PPB_Audio, PPB_AudioInput — moved to dedicated modules
    // PPB_BrowserFont_Trusted — moved to browser_font.rs
    // PPB_Buffer(Dev) — moved to buffer.rs
    // PPB_CharSet(Dev) — moved to char_set.rs
    // PPB_CursorControl(Dev) — moved to cursor_control.rs
    register_stub!(registry, FILE_CHOOSER_STUB, [
        "PPB_FileChooser(Dev);0.6", "PPB_FileChooser(Dev);0.5"
    ]);
    register_stub!(registry, FILE_CHOOSER_TRUSTED_STUB, [
        "PPB_FileChooserTrusted;0.6", "PPB_FileChooserTrusted;0.5"
    ]);
    register_stub!(registry, FILE_REF_STUB, [
        "PPB_FileRef;1.2", "PPB_FileRef;1.1", "PPB_FileRef;1.0"
    ]);
    register_stub!(registry, FLASH_FILE_FILEREF_STUB, [
        "PPB_Flash_File_FileRef;2"
    ]);
    register_stub!(registry, FLASH_FONT_FILE_STUB, [
        "PPB_Flash_FontFile;0.2", "PPB_Flash_FontFile;0.1"
    ]);
    register_stub!(registry, FLASH_MENU_STUB, [
        "PPB_Flash_Menu;0.2"
    ]);
    register_stub!(registry, GRAPHICS3D_STUB, [
        "PPB_Graphics3D;1.0"
    ]);
    register_stub!(registry, IME_INPUT_EVENT_STUB, [
        "PPB_IMEInputEvent(Dev);0.2", "PPB_IMEInputEvent(Dev);0.1"
    ]);
    register_stub!(registry, NET_ADDRESS_STUB, [
        "PPB_NetAddress_Private;1.1", "PPB_NetAddress_Private;1.0", "PPB_NetAddress_Private;0.1"
    ]);
    register_stub!(registry, OPENGLES2_CHROMIUM_DEV_STUB, [
        "PPB_OpenGLES2ChromiumMapSub(Dev);1.0",
        "PPB_GLESChromiumTextureMapping(Dev);0.1"
    ]);
    register_stub!(registry, TCP_SOCKET_STUB, [
        "PPB_TCPSocket_Private;0.5", "PPB_TCPSocket_Private;0.4", "PPB_TCPSocket_Private;0.3"
    ]);
    register_stub!(registry, TEXT_INPUT_STUB, [
        "PPB_TextInput(Dev);0.2", "PPB_TextInput(Dev);0.1"
    ]);
    register_stub!(registry, UDP_SOCKET_STUB, [
        "PPB_UDPSocket_Private;0.4", "PPB_UDPSocket_Private;0.3"
    ]);

    register_stub!(registry, VIDEO_CAPTURE_STUB, [
        "PPB_VideoCapture(Dev);0.3"
    ]);
    register_stub!(registry, PDF_STUB, [
        "PPB_PDF;1"
    ]);
    // PPB_OpenGLES2 extensions and PPB_Printing are now in dedicated modules
    register_stub!(registry, BROKER_TRUSTED_STUB, [
        "PPB_BrokerTrusted;0.3"
    ]);
    register_stub!(registry, VAR_DEPRECATED_STUB, [
        "PPB_Var(Deprecated);0.3"
    ]);
    register_stub!(registry, AUDIO_OUTPUT_STUB, [
        "PPB_AudioOutput(Dev);0.1"
    ]);
    register_stub!(registry, NETWORK_MONITOR_STUB, [
        "PPB_NetworkMonitor;1.0"
    ]);
    register_stub!(registry, INSTANCE_PRIVATE_STUB, [
        "PPB_Instance_Private;0.1"
    ]);
    } // unsafe
}
