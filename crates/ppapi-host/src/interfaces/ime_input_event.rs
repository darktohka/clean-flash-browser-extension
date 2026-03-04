//! PPB_IMEInputEvent(Dev);0.2 / 0.1 implementation.
//!
//! IME composition events are a specialisation of PPB_InputEvent.
//! They carry composition text, segment information, and selection state.
//! We store IME-specific fields in a dedicated resource type.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use crate::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// IME input event resource — stores composition text, segments, and selection.
pub struct IMEInputEventResource {
    pub instance: PP_Instance,
    pub event_type: PP_InputEvent_Type,
    pub time_stamp: PP_TimeTicks,
    pub text: PP_Var,
    pub segment_offsets: Vec<u32>,
    pub target_segment: i32,
    pub selection_start: u32,
    pub selection_end: u32,
}

impl Resource for IMEInputEventResource {
    fn resource_type(&self) -> &'static str {
        "PPB_IMEInputEvent"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Vtable functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn create(
    instance: PP_Instance,
    type_: PP_InputEvent_Type,
    time_stamp: PP_TimeTicks,
    text: PP_Var,
    segment_number: u32,
    segment_offsets: *const u32,
    target_segment: i32,
    selection_start: u32,
    selection_end: u32,
) -> PP_Resource {
    tracing::trace!(
        "PPB_IMEInputEvent(Dev)::Create(instance={}, type={}, segments={})",
        instance, type_, segment_number
    );
    let Some(host) = HOST.get() else { return 0 };

    // Copy segment offsets. The array has segment_number + 1 entries
    // (one extra for the end offset of the last segment).
    let offsets = if !segment_offsets.is_null() && segment_number > 0 {
        let count = (segment_number + 1) as usize;
        unsafe { std::slice::from_raw_parts(segment_offsets, count) }.to_vec()
    } else {
        Vec::new()
    };

    // Add a reference to the text var if it's a ref-counted type.
    if text.type_ == PP_VARTYPE_STRING {
        host.vars.add_ref(text);
    }

    let res = IMEInputEventResource {
        instance,
        event_type: type_,
        time_stamp,
        text,
        segment_offsets: offsets,
        target_segment,
        selection_start,
        selection_end,
    };
    host.resources.insert(instance, Box::new(res))
}

unsafe extern "C" fn is_ime_input_event(resource: PP_Resource) -> PP_Bool {
    let Some(host) = HOST.get() else { return PP_FALSE };

    // Check our dedicated IME resource type first.
    if host.resources.is_type(resource, "PPB_IMEInputEvent") {
        return PP_TRUE;
    }

    // Also accept regular InputEventResource with an IME event type.
    host.resources
        .with_downcast::<super::input_event::InputEventResource, _>(resource, |e| {
            pp_from_bool(matches!(
                e.event_type,
                PP_INPUTEVENT_TYPE_IME_COMPOSITION_START
                    | PP_INPUTEVENT_TYPE_IME_COMPOSITION_UPDATE
                    | PP_INPUTEVENT_TYPE_IME_COMPOSITION_END
                    | PP_INPUTEVENT_TYPE_IME_TEXT
            ))
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_text(ime_event: PP_Resource) -> PP_Var {
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    host.resources
        .with_downcast::<IMEInputEventResource, _>(ime_event, |e| {
            // Add ref for the returned var.
            if e.text.type_ == PP_VARTYPE_STRING {
                host.vars.add_ref(e.text);
            }
            e.text
        })
        .unwrap_or_else(PP_Var::undefined)
}

unsafe extern "C" fn get_segment_number(ime_event: PP_Resource) -> u32 {
    let Some(host) = HOST.get() else { return 0 };
    host.resources
        .with_downcast::<IMEInputEventResource, _>(ime_event, |e| {
            // segment_offsets has segment_number + 1 entries.
            if e.segment_offsets.is_empty() {
                0
            } else {
                (e.segment_offsets.len() - 1) as u32
            }
        })
        .unwrap_or(0)
}

unsafe extern "C" fn get_segment_offset(ime_event: PP_Resource, index: u32) -> u32 {
    let Some(host) = HOST.get() else { return 0 };
    host.resources
        .with_downcast::<IMEInputEventResource, _>(ime_event, |e| {
            e.segment_offsets
                .get(index as usize)
                .copied()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

unsafe extern "C" fn get_target_segment(ime_event: PP_Resource) -> i32 {
    let Some(host) = HOST.get() else { return -1 };
    host.resources
        .with_downcast::<IMEInputEventResource, _>(ime_event, |e| e.target_segment)
        .unwrap_or(-1)
}

unsafe extern "C" fn get_selection(
    ime_event: PP_Resource,
    start: *mut u32,
    end: *mut u32,
) {
    let Some(host) = HOST.get() else { return };
    host.resources
        .with_downcast::<IMEInputEventResource, _>(ime_event, |e| {
            if !start.is_null() {
                unsafe { *start = e.selection_start };
            }
            if !end.is_null() {
                unsafe { *end = e.selection_end };
            }
        });
}

// ---------------------------------------------------------------------------
// Vtables
// ---------------------------------------------------------------------------

static VTABLE_0_2: PPB_IMEInputEvent_Dev_0_2 = PPB_IMEInputEvent_Dev_0_2 {
    Create: Some(create),
    IsIMEInputEvent: Some(is_ime_input_event),
    GetText: Some(get_text),
    GetSegmentNumber: Some(get_segment_number),
    GetSegmentOffset: Some(get_segment_offset),
    GetTargetSegment: Some(get_target_segment),
    GetSelection: Some(get_selection),
};

static VTABLE_0_1: PPB_IMEInputEvent_Dev_0_1 = PPB_IMEInputEvent_Dev_0_1 {
    IsIMEInputEvent: Some(is_ime_input_event),
    GetText: Some(get_text),
    GetSegmentNumber: Some(get_segment_number),
    GetSegmentOffset: Some(get_segment_offset),
    GetTargetSegment: Some(get_target_segment),
    GetSelection: Some(get_selection),
};

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_IME_INPUT_EVENT_DEV_INTERFACE_0_2, &VTABLE_0_2);
        registry.register(PPB_IME_INPUT_EVENT_DEV_INTERFACE_0_1, &VTABLE_0_1);
    }
}
