//! PPB_TextInput(Dev);0.2 / 0.1 implementation.
//!
//! These are informational callbacks from the plugin to the host about
//! the current text input state (IME context, caret position, surrounding
//! text). In a standalone player without an OS IME integration layer,
//! the functions are accepted but have no effect.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::c_char;

// ---------------------------------------------------------------------------
// Vtable functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn set_text_input_type(
    instance: PP_Instance,
    type_: PP_TextInput_Type_Dev,
) {
    tracing::trace!(
        "PPB_TextInput(Dev)::SetTextInputType(instance={}, type={})",
        instance, type_
    );
    // In a standalone player we don't manage an OS IME context,
    // so this is a no-op.
}

unsafe extern "C" fn update_caret_position(
    instance: PP_Instance,
    caret: *const PP_Rect,
    _bounding_box: *const PP_Rect,
) {
    if !caret.is_null() {
        let c = unsafe { &*caret };
        tracing::trace!(
            "PPB_TextInput(Dev)::UpdateCaretPosition(instance={}, caret=({},{} {}x{}))",
            instance, c.point.x, c.point.y, c.size.width, c.size.height
        );
    }
}

unsafe extern "C" fn cancel_composition_text(instance: PP_Instance) {
    tracing::trace!(
        "PPB_TextInput(Dev)::CancelCompositionText(instance={})",
        instance
    );
}

unsafe extern "C" fn update_surrounding_text(
    instance: PP_Instance,
    _text: *const c_char,
    _caret: u32,
    _anchor: u32,
) {
    tracing::trace!(
        "PPB_TextInput(Dev)::UpdateSurroundingText(instance={})",
        instance
    );
}

unsafe extern "C" fn selection_changed(instance: PP_Instance) {
    tracing::trace!(
        "PPB_TextInput(Dev)::SelectionChanged(instance={})",
        instance
    );
}

// ---------------------------------------------------------------------------
// Vtables
// ---------------------------------------------------------------------------

static VTABLE_0_2: PPB_TextInput_Dev_0_2 = PPB_TextInput_Dev_0_2 {
    SetTextInputType: Some(set_text_input_type),
    UpdateCaretPosition: Some(update_caret_position),
    CancelCompositionText: Some(cancel_composition_text),
    UpdateSurroundingText: Some(update_surrounding_text),
    SelectionChanged: Some(selection_changed),
};

static VTABLE_0_1: PPB_TextInput_Dev_0_1 = PPB_TextInput_Dev_0_1 {
    SetTextInputType: Some(set_text_input_type),
    UpdateCaretPosition: Some(update_caret_position),
    CancelCompositionText: Some(cancel_composition_text),
};

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_TEXTINPUT_DEV_INTERFACE_0_2, &VTABLE_0_2);
        registry.register(PPB_TEXTINPUT_DEV_INTERFACE_0_1, &VTABLE_0_1);
    }
}
