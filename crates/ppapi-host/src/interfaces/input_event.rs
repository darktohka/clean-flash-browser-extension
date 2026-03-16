//! PPB_InputEvent;1.0, PPB_MouseInputEvent;1.1, PPB_KeyboardInputEvent;1.2,
//! PPB_WheelInputEvent;1.0 implementations.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

/// Input event resource data.
pub struct InputEventResource {
    pub event_type: PP_InputEvent_Type,
    pub time_stamp: PP_TimeTicks,
    pub modifiers: u32,
    // Mouse-specific
    pub mouse_button: PP_InputEvent_MouseButton,
    pub mouse_position: PP_Point,
    pub click_count: i32,
    pub mouse_movement: PP_Point,
    // Wheel-specific
    pub wheel_delta: PP_FloatPoint,
    pub wheel_ticks: PP_FloatPoint,
    pub scroll_by_page: bool,
    // Keyboard-specific
    pub key_code: u32,
    pub character_text: PP_Var,
    pub code: PP_Var,
}

impl Resource for InputEventResource {
    fn resource_type(&self) -> &'static str {
        "PPB_InputEvent"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Drop for InputEventResource {
    fn drop(&mut self) {
        // Release any ref-counted vars owned by this event.
        if let Some(host) = HOST.get() {
            if self.character_text.type_ == PP_VARTYPE_STRING {
                host.vars.release(self.character_text);
            }
            if self.code.type_ == PP_VARTYPE_STRING {
                host.vars.release(self.code);
            }
        }
    }
}

impl InputEventResource {
    pub fn new_mouse(
        event_type: PP_InputEvent_Type,
        time_stamp: PP_TimeTicks,
        modifiers: u32,
        button: PP_InputEvent_MouseButton,
        position: PP_Point,
        click_count: i32,
        movement: PP_Point,
    ) -> Self {
        Self {
            event_type,
            time_stamp,
            modifiers,
            mouse_button: button,
            mouse_position: position,
            click_count,
            mouse_movement: movement,
            wheel_delta: PP_FloatPoint::default(),
            wheel_ticks: PP_FloatPoint::default(),
            scroll_by_page: false,
            key_code: 0,
            character_text: PP_Var::undefined(),
            code: PP_Var::undefined(),
        }
    }

    pub fn new_keyboard(
        event_type: PP_InputEvent_Type,
        time_stamp: PP_TimeTicks,
        modifiers: u32,
        key_code: u32,
        character_text: PP_Var,
        code: PP_Var,
    ) -> Self {
        Self {
            event_type,
            time_stamp,
            modifiers,
            mouse_button: PP_INPUTEVENT_MOUSEBUTTON_NONE,
            mouse_position: PP_Point::default(),
            click_count: 0,
            mouse_movement: PP_Point::default(),
            wheel_delta: PP_FloatPoint::default(),
            wheel_ticks: PP_FloatPoint::default(),
            scroll_by_page: false,
            key_code,
            character_text,
            code,
        }
    }

    pub fn new_wheel(
        time_stamp: PP_TimeTicks,
        modifiers: u32,
        delta: PP_FloatPoint,
        ticks: PP_FloatPoint,
        scroll_by_page: bool,
    ) -> Self {
        Self {
            event_type: PP_INPUTEVENT_TYPE_WHEEL,
            time_stamp,
            modifiers,
            mouse_button: PP_INPUTEVENT_MOUSEBUTTON_NONE,
            mouse_position: PP_Point::default(),
            click_count: 0,
            mouse_movement: PP_Point::default(),
            wheel_delta: delta,
            wheel_ticks: ticks,
            scroll_by_page,
            key_code: 0,
            character_text: PP_Var::undefined(),
            code: PP_Var::undefined(),
        }
    }
}

// ---------------------------------------------------------------------------
// PPB_InputEvent;1.0
// ---------------------------------------------------------------------------

static INPUT_EVENT_VTABLE: PPB_InputEvent_1_0 = PPB_InputEvent_1_0 {
    RequestInputEvents: Some(request_input_events),
    RequestFilteringInputEvents: Some(request_filtering_input_events),
    ClearInputEventRequest: Some(clear_input_event_request),
    IsInputEvent: Some(is_input_event),
    GetType: Some(get_type),
    GetTimeStamp: Some(get_time_stamp),
    GetModifiers: Some(get_modifiers),
};

static MOUSE_VTABLE: PPB_MouseInputEvent_1_1 = PPB_MouseInputEvent_1_1 {
    Create: Some(create_mouse),
    IsMouseInputEvent: Some(is_mouse_input_event),
    GetButton: Some(get_button),
    GetPosition: Some(get_position),
    GetClickCount: Some(get_click_count),
    GetMovement: Some(get_movement),
};

static KEYBOARD_VTABLE: PPB_KeyboardInputEvent_1_2 = PPB_KeyboardInputEvent_1_2 {
    Create: Some(create_keyboard),
    IsKeyboardInputEvent: Some(is_keyboard_input_event),
    GetKeyCode: Some(get_key_code),
    GetCharacterText: Some(get_character_text),
    GetCode: Some(get_code),
};

static WHEEL_VTABLE: PPB_WheelInputEvent_1_0 = PPB_WheelInputEvent_1_0 {
    Create: Some(create_wheel),
    IsWheelInputEvent: Some(is_wheel_input_event),
    GetDelta: Some(get_delta),
    GetTicks: Some(get_ticks),
    GetScrollByPage: Some(get_scroll_by_page),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_INPUTEVENT_INTERFACE_1_0, &INPUT_EVENT_VTABLE);
        registry.register(PPB_MOUSEINPUTEVENT_INTERFACE_1_1, &MOUSE_VTABLE);
        registry.register(PPB_KEYBOARDINPUTEVENT_INTERFACE_1_2, &KEYBOARD_VTABLE);
        registry.register(PPB_WHEELINPUTEVENT_INTERFACE_1_0, &WHEEL_VTABLE);
    }
}

// --- PPB_InputEvent ---

unsafe extern "C" fn request_input_events(instance: PP_Instance, event_classes: u32) -> i32 {
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };
    host.instances.with_instance_mut(instance, |inst| {
        inst.requested_input_events |= event_classes;
    });
    PP_OK
}

unsafe extern "C" fn request_filtering_input_events(instance: PP_Instance, event_classes: u32) -> i32 {
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };
    host.instances.with_instance_mut(instance, |inst| {
        inst.filtering_input_events |= event_classes;
    });
    PP_OK
}

unsafe extern "C" fn clear_input_event_request(instance: PP_Instance, event_classes: u32) {
    if let Some(host) = HOST.get() {
        host.instances.with_instance_mut(instance, |inst| {
            inst.requested_input_events &= !event_classes;
            inst.filtering_input_events &= !event_classes;
        });
    }
}

unsafe extern "C" fn is_input_event(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_InputEvent")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_type(event: PP_Resource) -> PP_InputEvent_Type {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(event, |e| e.event_type)
        })
        .unwrap_or(PP_INPUTEVENT_TYPE_UNDEFINED)
}

unsafe extern "C" fn get_time_stamp(event: PP_Resource) -> PP_TimeTicks {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(event, |e| e.time_stamp)
        })
        .unwrap_or(0.0)
}

unsafe extern "C" fn get_modifiers(event: PP_Resource) -> u32 {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(event, |e| e.modifiers)
        })
        .unwrap_or(0)
}

// --- PPB_MouseInputEvent ---

unsafe extern "C" fn create_mouse(
    instance: PP_Instance,
    type_: PP_InputEvent_Type,
    time_stamp: PP_TimeTicks,
    modifiers: u32,
    mouse_button: PP_InputEvent_MouseButton,
    mouse_position: *const PP_Point,
    click_count: i32,
    mouse_movement: *const PP_Point,
) -> PP_Resource {
    let Some(host) = HOST.get() else {
        return 0;
    };
    let pos = if mouse_position.is_null() {
        PP_Point::default()
    } else {
        unsafe { *mouse_position }
    };
    let mov = if mouse_movement.is_null() {
        PP_Point::default()
    } else {
        unsafe { *mouse_movement }
    };
    let ev = InputEventResource::new_mouse(type_, time_stamp, modifiers, mouse_button, pos, click_count, mov);
    host.resources.insert(instance, Box::new(ev))
}

unsafe extern "C" fn is_mouse_input_event(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(resource, |e| {
                    pp_from_bool(matches!(
                        e.event_type,
                        PP_INPUTEVENT_TYPE_MOUSEDOWN
                            | PP_INPUTEVENT_TYPE_MOUSEUP
                            | PP_INPUTEVENT_TYPE_MOUSEMOVE
                            | PP_INPUTEVENT_TYPE_MOUSEENTER
                            | PP_INPUTEVENT_TYPE_MOUSELEAVE
                            | PP_INPUTEVENT_TYPE_CONTEXTMENU
                    ))
                })
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_button(mouse_event: PP_Resource) -> PP_InputEvent_MouseButton {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(mouse_event, |e| e.mouse_button)
        })
        .unwrap_or(PP_INPUTEVENT_MOUSEBUTTON_NONE)
}

unsafe extern "C" fn get_position(mouse_event: PP_Resource) -> PP_Point {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(mouse_event, |e| e.mouse_position)
        })
        .unwrap_or_default()
}

unsafe extern "C" fn get_click_count(mouse_event: PP_Resource) -> i32 {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(mouse_event, |e| e.click_count)
        })
        .unwrap_or(0)
}

unsafe extern "C" fn get_movement(mouse_event: PP_Resource) -> PP_Point {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(mouse_event, |e| e.mouse_movement)
        })
        .unwrap_or_default()
}

// --- PPB_KeyboardInputEvent ---

unsafe extern "C" fn create_keyboard(
    instance: PP_Instance,
    type_: PP_InputEvent_Type,
    time_stamp: PP_TimeTicks,
    modifiers: u32,
    key_code: u32,
    character_text: PP_Var,
    code: PP_Var,
) -> PP_Resource {
    let Some(host) = HOST.get() else {
        return 0;
    };
    // AddRef the incoming vars - the caller retains its own reference.
    if character_text.type_ == PP_VARTYPE_STRING {
        host.vars.add_ref(character_text);
    }
    if code.type_ == PP_VARTYPE_STRING {
        host.vars.add_ref(code);
    }
    let ev = InputEventResource::new_keyboard(type_, time_stamp, modifiers, key_code, character_text, code);
    host.resources.insert(instance, Box::new(ev))
}

unsafe extern "C" fn is_keyboard_input_event(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(resource, |e| {
                    pp_from_bool(matches!(
                        e.event_type,
                        PP_INPUTEVENT_TYPE_RAWKEYDOWN
                            | PP_INPUTEVENT_TYPE_KEYDOWN
                            | PP_INPUTEVENT_TYPE_KEYUP
                            | PP_INPUTEVENT_TYPE_CHAR
                    ))
                })
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_key_code(key_event: PP_Resource) -> u32 {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(key_event, |e| e.key_code)
        })
        .unwrap_or(0)
}

unsafe extern "C" fn get_character_text(character_event: PP_Resource) -> PP_Var {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(character_event, |e| {
                    // AddRef for the caller per PPAPI convention.
                    if e.character_text.type_ == PP_VARTYPE_STRING {
                        h.vars.add_ref(e.character_text);
                    }
                    e.character_text
                })
        })
        .unwrap_or_else(PP_Var::undefined)
}

unsafe extern "C" fn get_code(key_event: PP_Resource) -> PP_Var {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(key_event, |e| {
                    // AddRef for the caller per PPAPI convention.
                    if e.code.type_ == PP_VARTYPE_STRING {
                        h.vars.add_ref(e.code);
                    }
                    e.code
                })
        })
        .unwrap_or_else(PP_Var::undefined)
}

// --- PPB_WheelInputEvent ---

unsafe extern "C" fn create_wheel(
    instance: PP_Instance,
    time_stamp: PP_TimeTicks,
    modifiers: u32,
    wheel_delta: *const PP_FloatPoint,
    wheel_ticks: *const PP_FloatPoint,
    scroll_by_page: PP_Bool,
) -> PP_Resource {
    let Some(host) = HOST.get() else {
        return 0;
    };
    let delta = if wheel_delta.is_null() { PP_FloatPoint::default() } else { unsafe { *wheel_delta } };
    let ticks = if wheel_ticks.is_null() { PP_FloatPoint::default() } else { unsafe { *wheel_ticks } };
    let ev = InputEventResource::new_wheel(time_stamp, modifiers, delta, ticks, pp_to_bool(scroll_by_page));
    host.resources.insert(instance, Box::new(ev))
}

unsafe extern "C" fn is_wheel_input_event(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(resource, |e| {
                    pp_from_bool(e.event_type == PP_INPUTEVENT_TYPE_WHEEL)
                })
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_delta(wheel_event: PP_Resource) -> PP_FloatPoint {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(wheel_event, |e| e.wheel_delta)
        })
        .unwrap_or_default()
}

unsafe extern "C" fn get_ticks(wheel_event: PP_Resource) -> PP_FloatPoint {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(wheel_event, |e| e.wheel_ticks)
        })
        .unwrap_or_default()
}

unsafe extern "C" fn get_scroll_by_page(wheel_event: PP_Resource) -> PP_Bool {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<InputEventResource, _>(wheel_event, |e| pp_from_bool(e.scroll_by_page))
        })
        .unwrap_or(PP_FALSE)
}
