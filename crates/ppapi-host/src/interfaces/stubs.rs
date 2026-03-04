//! Stub implementations for all PPAPI interfaces required by PepperFlash
//! that don't yet have full implementations.
//!
//! These stubs return a valid (non-null) vtable so the plugin passes its
//! init-time interface availability check. Individual functions are no-ops
//! that return 0/null/PP_FALSE. As we implement real functionality, we
//! move interfaces out of this file into dedicated modules.

//use crate::interface_registry::InterfaceRegistry;
//use parking_lot::Mutex;
//use std::collections::HashMap;
//use std::sync::OnceLock;
//
// ---------------------------------------------------------------------------
// Named-stub machinery
//
// Each vtable slot gets a DISTINCT function address by instantiating
// `typed_stub::<N>` with a unique const generic N.  At registration time,
// `register_named_stubs` maps each function pointer → (interface_name, slot)
// so that any call to the stub can log exactly which interface and slot was
// invoked.
// ---------------------------------------------------------------------------

/*
/// Global registry: function-pointer value → (interface_name, slot_index).
static STUB_REGISTRY: OnceLock<Mutex<HashMap<usize, (&'static str, usize)>>> = OnceLock::new();

fn stub_registry() -> &'static Mutex<HashMap<usize, (&'static str, usize)>> {
    STUB_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Called by every typed_stub instantiation.
fn stub_called(fn_ptr: usize) {
    let reg = stub_registry().lock();
    if let Some((iface, slot)) = reg.get(&fn_ptr) {
        tracing::warn!("STUB called: {}[{}] (unimplemented)", iface, slot);
    } else {
        tracing::warn!("STUB called: fn_ptr=0x{:x} (unregistered)", fn_ptr);
    }
}

/// Slot-N stub: each monomorphisation is a distinct symbol with a distinct
/// address, so Flash's vtable lookup can be traced back to interface + slot.
unsafe extern "C" fn typed_stub<const N: usize>() -> usize {
    stub_called(typed_stub::<N> as *const () as usize);
    0
}

/// Populate the registry for a vtable array.
/// Call once per interface name after declaring the vtable static.
fn register_named_stubs(iface: &'static str, ptrs: &[unsafe extern "C" fn() -> usize]) {
    let mut reg = stub_registry().lock();
    for (slot, &ptr) in ptrs.iter().enumerate() {
        reg.insert(ptr as *const () as usize, (iface, slot));
    }
}

// ---------------------------------------------------------------------------
// stub_array!(N) — builds a const array of N distinct typed_stub<i> pointers.
// Add arms here as needed (max slot count across all vtables below).
// ---------------------------------------------------------------------------
macro_rules! stub_array {
    (1)  => { [typed_stub::<0>] };
    (2)  => { [typed_stub::<0>, typed_stub::<1>] };
    (3)  => { [typed_stub::<0>, typed_stub::<1>, typed_stub::<2>] };
    (4)  => { [typed_stub::<0>, typed_stub::<1>, typed_stub::<2>, typed_stub::<3>] };
    (5)  => { [typed_stub::<0>, typed_stub::<1>, typed_stub::<2>, typed_stub::<3>,
               typed_stub::<4>] };
    (6)  => { [typed_stub::<0>, typed_stub::<1>, typed_stub::<2>, typed_stub::<3>,
               typed_stub::<4>, typed_stub::<5>] };
    (7)  => { [typed_stub::<0>, typed_stub::<1>, typed_stub::<2>, typed_stub::<3>,
               typed_stub::<4>, typed_stub::<5>, typed_stub::<6>] };
    (8)  => { [typed_stub::<0>, typed_stub::<1>, typed_stub::<2>, typed_stub::<3>,
               typed_stub::<4>, typed_stub::<5>, typed_stub::<6>, typed_stub::<7>] };
    (9)  => { [typed_stub::<0>, typed_stub::<1>, typed_stub::<2>, typed_stub::<3>,
               typed_stub::<4>, typed_stub::<5>, typed_stub::<6>, typed_stub::<7>,
               typed_stub::<8>] };
    (10) => { [typed_stub::<0>, typed_stub::<1>, typed_stub::<2>, typed_stub::<3>,
               typed_stub::<4>, typed_stub::<5>, typed_stub::<6>, typed_stub::<7>,
               typed_stub::<8>, typed_stub::<9>] };
    (11) => { [typed_stub::<0>, typed_stub::<1>, typed_stub::<2>, typed_stub::<3>,
               typed_stub::<4>, typed_stub::<5>, typed_stub::<6>, typed_stub::<7>,
               typed_stub::<8>, typed_stub::<9>, typed_stub::<10>] };
    (12) => { [typed_stub::<0>, typed_stub::<1>, typed_stub::<2>, typed_stub::<3>,
               typed_stub::<4>, typed_stub::<5>, typed_stub::<6>, typed_stub::<7>,
               typed_stub::<8>, typed_stub::<9>, typed_stub::<10>, typed_stub::<11>] };
    (13) => { [typed_stub::<0>, typed_stub::<1>, typed_stub::<2>, typed_stub::<3>,
               typed_stub::<4>, typed_stub::<5>, typed_stub::<6>, typed_stub::<7>,
               typed_stub::<8>, typed_stub::<9>, typed_stub::<10>, typed_stub::<11>,
               typed_stub::<12>] };
    (16) => { [typed_stub::<0>,  typed_stub::<1>,  typed_stub::<2>,  typed_stub::<3>,
               typed_stub::<4>,  typed_stub::<5>,  typed_stub::<6>,  typed_stub::<7>,
               typed_stub::<8>,  typed_stub::<9>,  typed_stub::<10>, typed_stub::<11>,
               typed_stub::<12>, typed_stub::<13>, typed_stub::<14>, typed_stub::<15>] };
    (32) => { [typed_stub::<0>,  typed_stub::<1>,  typed_stub::<2>,  typed_stub::<3>,
               typed_stub::<4>,  typed_stub::<5>,  typed_stub::<6>,  typed_stub::<7>,
               typed_stub::<8>,  typed_stub::<9>,  typed_stub::<10>, typed_stub::<11>,
               typed_stub::<12>, typed_stub::<13>, typed_stub::<14>, typed_stub::<15>,
               typed_stub::<16>, typed_stub::<17>, typed_stub::<18>, typed_stub::<19>,
               typed_stub::<20>, typed_stub::<21>, typed_stub::<22>, typed_stub::<23>,
               typed_stub::<24>, typed_stub::<25>, typed_stub::<26>, typed_stub::<27>,
               typed_stub::<28>, typed_stub::<29>, typed_stub::<30>, typed_stub::<31>] };
}

// ---------------------------------------------------------------------------
// Macro to declare a static stub vtable of N function pointers and register
// one or more interface name strings to it.
// ---------------------------------------------------------------------------

macro_rules! stub_vtable {
    ($name:ident, $count:tt, [ $($iface:expr),+ $(,)? ]) => {
        static $name: [unsafe extern "C" fn() -> usize; $count] = stub_array!($count);
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

// PPB_FileChooser(Dev) — moved to file_chooser.rs
// PPB_FileChooserTrusted — moved to file_chooser.rs
// PPB_FileRef — moved to file_ref.rs
// PPB_Flash_File_FileRef — moved to flash_file_fileref.rs
// PPB_Flash_FontFile — moved to flash_font_file.rs
// PPB_Flash_Menu — moved to flash_menu.rs

// PPB_Graphics3D;1.0 — moved to graphics3d.rs

// PPB_IMEInputEvent(Dev) — moved to ime_input_event.rs

// PPB_NetAddress_Private — moved to net_address.rs

// PPB_OpenGLES2;1.0 — moved to opengles2.rs

// PPB_OpenGLES2ChromiumMapSub — moved to opengles2.rs (including Dev variant names)

// PPB_TCPSocket_Private — moved to tcp_socket.rs

// PPB_TextInput(Dev) — moved to text_input.rs

// PPB_UDPSocket_Private — moved to udp_socket.rs







// Additional interfaces requested post-init
// PPB_OpenGLES2 extensions and PPB_Printing — moved to opengles2.rs / printing.rs

// ---------------------------------------------------------------------------
// Registration — macro to register all versions of each stub vtable
// ---------------------------------------------------------------------------

macro_rules! register_stub {
    ($registry:expr, $vtable:expr, [ $($iface:expr),+ $(,)? ]) => {
        // Register the first interface name with the name registry so we get
        // a human-readable label in tracing (subsequent aliases share the same
        // vtable array so they'd overwrite with an equally-informative name).
        register_named_stubs(
            // Use the first listed name as the canonical label.
            [$($iface),+][0],
            &$vtable,
        );
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
    // PPB_FileChooser(Dev) — moved to file_chooser.rs
    // PPB_FileChooserTrusted — moved to file_chooser.rs
    // PPB_FileRef — moved to file_ref.rs
    // PPB_Flash_File_FileRef — moved to flash_file_fileref.rs
    // PPB_Flash_FontFile — moved to flash_font_file.rs
    // PPB_Flash_Menu — moved to flash_menu.rs
    // PPB_Graphics3D;1.0 — moved to graphics3d.rs
    // PPB_IMEInputEvent(Dev) — moved to ime_input_event.rs
    // PPB_NetAddress_Private — moved to net_address.rs
    // PPB_OpenGLES2ChromiumMapSub Dev variants — moved to opengles2.rs
    // PPB_TCPSocket_Private — moved to tcp_socket.rs
    // PPB_TextInput(Dev) — moved to text_input.rs
    // PPB_UDPSocket_Private — moved to udp_socket.rs

    // PPB_OpenGLES2 extensions and PPB_Printing are now in dedicated modules
    } // unsafe
}

*/