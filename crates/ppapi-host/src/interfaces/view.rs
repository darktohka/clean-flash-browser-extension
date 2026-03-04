//! PPB_View;1.2 implementation.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

/// View resource data.
pub struct ViewResource {
    pub rect: PP_Rect,
    pub clip_rect: PP_Rect,
    pub is_fullscreen: bool,
    pub is_visible: bool,
    pub is_page_visible: bool,
    pub device_scale: f32,
    pub css_scale: f32,
    pub scroll_offset: PP_Point,
}

impl Resource for ViewResource {
    fn resource_type(&self) -> &'static str {
        "PPB_View"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl ViewResource {
    pub fn new(rect: PP_Rect) -> Self {
        Self {
            clip_rect: rect,
            rect,
            is_fullscreen: false,
            is_visible: true,
            is_page_visible: true,
            device_scale: 1.0,
            css_scale: 1.0,
            scroll_offset: PP_Point::default(),
        }
    }
}

static VTABLE: PPB_View_1_2 = PPB_View_1_2 {
    IsView: Some(is_view),
    GetRect: Some(get_rect),
    IsFullscreen: Some(is_fullscreen),
    IsVisible: Some(is_visible),
    IsPageVisible: Some(is_page_visible),
    GetClipRect: Some(get_clip_rect),
    GetDeviceScale: Some(get_device_scale),
    GetCSSScale: Some(get_css_scale),
    GetScrollOffset: Some(get_scroll_offset),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_VIEW_INTERFACE_1_2, &VTABLE);
        registry.register(PPB_VIEW_INTERFACE_1_1, &VTABLE);
        registry.register(PPB_VIEW_INTERFACE_1_0, &VTABLE);
    }
}

unsafe extern "C" fn is_view(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_View")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_rect(resource: PP_Resource, rect: *mut PP_Rect) -> PP_Bool {
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    host.resources
        .with_downcast::<ViewResource, _>(resource, |view| {
            if !rect.is_null() {
                unsafe { *rect = view.rect };
            }
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn is_fullscreen(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<ViewResource, _>(resource, |v| pp_from_bool(v.is_fullscreen))
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn is_visible(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<ViewResource, _>(resource, |v| pp_from_bool(v.is_visible))
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn is_page_visible(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<ViewResource, _>(resource, |v| pp_from_bool(v.is_page_visible))
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_clip_rect(resource: PP_Resource, clip: *mut PP_Rect) -> PP_Bool {
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    host.resources
        .with_downcast::<ViewResource, _>(resource, |view| {
            if !clip.is_null() {
                unsafe { *clip = view.clip_rect };
            }
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_device_scale(resource: PP_Resource) -> f32 {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<ViewResource, _>(resource, |v| v.device_scale)
        })
        .unwrap_or(1.0)
}

unsafe extern "C" fn get_css_scale(resource: PP_Resource) -> f32 {
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<ViewResource, _>(resource, |v| v.css_scale)
        })
        .unwrap_or(1.0)
}

unsafe extern "C" fn get_scroll_offset(resource: PP_Resource, offset: *mut PP_Point) -> PP_Bool {
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    host.resources
        .with_downcast::<ViewResource, _>(resource, |view| {
            if !offset.is_null() {
                unsafe { *offset = view.scroll_offset };
            }
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}
