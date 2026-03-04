//! PPB_BrowserFont_Trusted;1.0 implementation.
//!
//! Provides basic font creation, description, measurement, and text drawing.
//! We use a simple built-in approach: fonts are not actually rasterized with
//! a system font library (that would require fontconfig/freetype integration),
//! but we provide plausible metric values so the plugin can lay out text.
//! DrawTextAt is a no-op — Flash primarily uses this for UI chrome text
//! that we don't need to render in a standalone player.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

pub struct BrowserFontResource {
    pub family: PP_BrowserFont_Trusted_Family,
    pub size: u32,
    pub weight: PP_BrowserFont_Trusted_Weight,
    pub italic: PP_Bool,
    pub small_caps: PP_Bool,
    pub letter_spacing: i32,
    pub word_spacing: i32,
}

impl Resource for BrowserFontResource {
    fn resource_type(&self) -> &'static str {
        "PPB_BrowserFont"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Vtable
// ---------------------------------------------------------------------------

static VTABLE: PPB_BrowserFont_Trusted_1_0 = PPB_BrowserFont_Trusted_1_0 {
    GetFontFamilies: Some(get_font_families),
    Create: Some(create),
    IsFont: Some(is_font),
    Describe: Some(describe),
    DrawTextAt: Some(draw_text_at),
    MeasureText: Some(measure_text),
    CharacterOffsetForPixel: Some(character_offset_for_pixel),
    PixelOffsetForCharacter: Some(pixel_offset_for_character),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_BROWSERFONT_TRUSTED_INTERFACE_1_0, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

unsafe extern "C" fn get_font_families(_instance: PP_Instance) -> PP_Var {
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    // Return a null-separated list of generic font family names.
    // Flash uses this to check font availability. Provide common fallbacks.
    host.vars
        .var_from_str("Sans\0Serif\0Monospace\0Arial\0Times New Roman\0Courier New\0")
}

unsafe extern "C" fn create(
    instance: PP_Instance,
    description: *const PP_BrowserFont_Trusted_Description,
) -> PP_Resource {
    let Some(host) = HOST.get() else {
        return 0;
    };
    if description.is_null() {
        return 0;
    }

    let desc = unsafe { &*description };

    let font = BrowserFontResource {
        family: desc.family,
        size: if desc.size == 0 { 16 } else { desc.size },
        weight: desc.weight,
        italic: desc.italic,
        small_caps: desc.small_caps,
        letter_spacing: desc.letter_spacing,
        word_spacing: desc.word_spacing,
    };

    host.resources.insert(instance, Box::new(font))
}

unsafe extern "C" fn is_font(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_BrowserFont")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn describe(
    font: PP_Resource,
    description: *mut PP_BrowserFont_Trusted_Description,
    metrics: *mut PP_BrowserFont_Trusted_Metrics,
) -> PP_Bool {
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    if description.is_null() || metrics.is_null() {
        return PP_FALSE;
    }

    host.resources
        .with_downcast::<BrowserFontResource, _>(font, |bf| {
            let size = bf.size as i32;
            // Provide plausible font metrics based on the size.
            let ascent = (size as f32 * 0.8) as i32;
            let descent = (size as f32 * 0.2) as i32;

            unsafe {
                let desc = &mut *description;
                desc.face = host.vars.var_from_str("Sans");
                desc.family = bf.family;
                desc.size = bf.size;
                desc.weight = bf.weight;
                desc.italic = bf.italic;
                desc.small_caps = bf.small_caps;
                desc.letter_spacing = bf.letter_spacing;
                desc.word_spacing = bf.word_spacing;
                desc.padding = 0;

                let m = &mut *metrics;
                m.height = size;
                m.ascent = ascent;
                m.descent = descent;
                m.line_spacing = size + 2;
                m.x_height = ascent / 2;
            }
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn draw_text_at(
    font: PP_Resource,
    _image_data: PP_Resource,
    _text: *const PP_BrowserFont_Trusted_TextRun,
    _position: *const PP_Point,
    _color: u32,
    _clip: *const PP_Rect,
    _image_data_is_opaque: PP_Bool,
) -> PP_Bool {
    // We don't have font rasterization — return TRUE so Flash thinks it succeeded.
    // In a standalone player, the Flash content renders via Graphics2D, not browser fonts.
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };
    let is_valid = host.resources.is_type(font, "PPB_BrowserFont");
    pp_from_bool(is_valid)
}

unsafe extern "C" fn measure_text(
    font: PP_Resource,
    text: *const PP_BrowserFont_Trusted_TextRun,
) -> i32 {
    let Some(host) = HOST.get() else {
        return -1;
    };

    // Estimate text width: roughly 0.6 * font_size per character.
    let text_len = if !text.is_null() {
        let run = unsafe { &*text };
        host.vars
            .get_string(run.text)
            .map(|s| s.len())
            .unwrap_or(0)
    } else {
        0
    };

    host.resources
        .with_downcast::<BrowserFontResource, _>(font, |bf| {
            let char_width = (bf.size as f32 * 0.6) as i32;
            (text_len as i32) * char_width
        })
        .unwrap_or(-1)
}

unsafe extern "C" fn character_offset_for_pixel(
    _font: PP_Resource,
    _text: *const PP_BrowserFont_Trusted_TextRun,
    _pixel_position: i32,
) -> u32 {
    // Not implemented — return 0.
    0
}

unsafe extern "C" fn pixel_offset_for_character(
    _font: PP_Resource,
    _text: *const PP_BrowserFont_Trusted_TextRun,
    _char_offset: u32,
) -> i32 {
    // Not implemented — return 0.
    0
}
