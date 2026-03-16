//! PPB_BrowserFont_Trusted;1.0 implementation.
//!
//! Provides font creation, description, measurement, and text drawing.
//! Text rasterization uses `ab_glyph` via the shared `font_rasterizer` module.

use crate::font_rasterizer;
use crate::interface_registry::InterfaceRegistry;
use crate::interfaces::image_data::ImageDataResource;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::sync::Arc;

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
    /// Resolved system font for rasterization.
    pub font: Arc<ab_glyph::FontVec>,
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
// Helpers
// ---------------------------------------------------------------------------

/// Resolve a font for a `PP_BrowserFont_Trusted_Description`.
fn resolve_font_for_desc(desc: &PP_BrowserFont_Trusted_Description) -> Arc<ab_glyph::FontVec> {
    let host = HOST.get();
    let family_name = host.and_then(|h| h.vars.get_string(desc.face));
    let bold = desc.weight >= PP_BROWSERFONT_TRUSTED_WEIGHT_BOLD;
    let italic = pp_to_bool(desc.italic);

    font_rasterizer::resolve_system_font(
        family_name.as_deref(),
        desc.family,
        bold,
        italic,
    )
}

/// Compute real text width using `ab_glyph` metrics.
fn measure_text_width(font: &ab_glyph::FontVec, text: &str, px_size: f32) -> f32 {
    use ab_glyph::{Font, PxScale, ScaleFont};
    let scale = PxScale::from(px_size);
    let scaled = font.as_scaled(scale);

    let mut width = 0.0f32;
    let mut prev: Option<ab_glyph::GlyphId> = None;
    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        if let Some(p) = prev {
            width += scaled.kern(p, gid);
        }
        width += scaled.h_advance(gid);
        prev = Some(gid);
    }
    width
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

unsafe extern "C" fn get_font_families(_instance: PP_Instance) -> PP_Var {
    tracing::trace!("PPB_BrowserFont_Trusted::GetFontFamilies(instance={})", _instance);
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    // Enumerate real system font families via fontdb.
    let families = font_rasterizer::get_font_families();
    let list = families.join("\0") + "\0";
    host.vars.var_from_str(&list)
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
    let font = resolve_font_for_desc(desc);

    let face_str = host.vars.get_string(desc.face).unwrap_or_default();
    tracing::debug!(
        "PPB_BrowserFont_Trusted::Create(instance={}, face={:?}, size={}, weight={}, italic={})",
        instance, face_str, desc.size, desc.weight, pp_to_bool(desc.italic)
    );

    let bf = BrowserFontResource {
        family: desc.family,
        size: if desc.size == 0 { 16 } else { desc.size },
        weight: desc.weight,
        italic: desc.italic,
        small_caps: desc.small_caps,
        letter_spacing: desc.letter_spacing,
        word_spacing: desc.word_spacing,
        font,
    };

    host.resources.insert(instance, Box::new(bf))
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
            let px_size = bf.size as f32;
            // Compute real metrics from the font.
            use ab_glyph::{Font, PxScale, ScaleFont};
            let scale = PxScale::from(px_size);
            let scaled = bf.font.as_scaled(scale);

            let ascent = scaled.ascent();
            let descent = scaled.descent(); // negative
            let line_gap = scaled.line_gap();

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
                m.ascent = ascent.round() as i32;
                m.descent = (-descent).round() as i32; // PPAPI descent is positive
                m.height = (ascent - descent).round() as i32;
                m.line_spacing = (ascent - descent + line_gap).round() as i32;
                m.x_height = (ascent * 0.53).round() as i32; // approximate x-height
            }
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn draw_text_at(
    font: PP_Resource,
    image_data: PP_Resource,
    text: *const PP_BrowserFont_Trusted_TextRun,
    position: *const PP_Point,
    color: u32,
    clip: *const PP_Rect,
    _image_data_is_opaque: PP_Bool,
) -> PP_Bool {
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    if text.is_null() || position.is_null() {
        return PP_FALSE;
    }

    let text_run = unsafe { &*text };
    let pos = unsafe { &*position };

    let text_str = match host.vars.get_string(text_run.text) {
        Some(s) => s,
        None => return PP_TRUE, // empty text → nothing to draw
    };

    if text_str.is_empty() {
        return PP_TRUE;
    }

    tracing::trace!(
        "PPB_BrowserFont_Trusted::DrawTextAt(font={}, image_data={}, text={:?}, pos=({},{}), color={:#010x})",
        font, image_data, text_str, pos.x, pos.y, color
    );

    // We need to read the font and write the image data.
    // First, get the font info (Arc<FontVec> + size).
    let font_info = host.resources
        .with_downcast::<BrowserFontResource, _>(font, |bf| {
            (Arc::clone(&bf.font), bf.size as f32)
        });

    let Some((font_vec, px_size)) = font_info else {
        return PP_FALSE;
    };

    let clip_rect = if !clip.is_null() {
        let c = unsafe { &*clip };
        Some((c.point.x, c.point.y, c.size.width, c.size.height))
    } else {
        None
    };

    // Now mutably access the image data and draw.
    let result = host.resources
        .with_downcast_mut::<ImageDataResource, _>(image_data, |img| {
            font_rasterizer::draw_text_to_bgra(
                &mut img.pixels,
                img.stride,
                img.size.width,
                img.size.height,
                &font_vec,
                &text_str,
                px_size,
                pos.x as f32,
                pos.y as f32,
                color,
                clip_rect,
            );
        });

    pp_from_bool(result.is_some())
}

unsafe extern "C" fn measure_text(
    font: PP_Resource,
    text: *const PP_BrowserFont_Trusted_TextRun,
) -> i32 {
    let Some(host) = HOST.get() else {
        return -1;
    };

    let text_len = if !text.is_null() {
        let run = unsafe { &*text };
        host.vars
            .get_string(run.text)
            .unwrap_or_default()
    } else {
        return 0;
    };

    if text_len.is_empty() {
        return 0;
    }

    host.resources
        .with_downcast::<BrowserFontResource, _>(font, |bf| {
            let width = measure_text_width(&bf.font, &text_len, bf.size as f32);
            width.round() as i32
        })
        .unwrap_or(-1)
}

unsafe extern "C" fn character_offset_for_pixel(
    font: PP_Resource,
    text: *const PP_BrowserFont_Trusted_TextRun,
    pixel_position: i32,
) -> u32 {
    let Some(host) = HOST.get() else { return 0 };

    if text.is_null() { return 0; }
    let run = unsafe { &*text };
    let text_str = host.vars.get_string(run.text).unwrap_or_default();
    if text_str.is_empty() { return 0; }

    host.resources
        .with_downcast::<BrowserFontResource, _>(font, |bf| {
            use ab_glyph::{Font, PxScale, ScaleFont};
            let scale = PxScale::from(bf.size as f32);
            let scaled = bf.font.as_scaled(scale);

            let mut x = 0.0f32;
            let mut prev: Option<ab_glyph::GlyphId> = None;
            for (i, ch) in text_str.chars().enumerate() {
                let gid = bf.font.glyph_id(ch);
                if let Some(p) = prev {
                    x += scaled.kern(p, gid);
                }
                let adv = scaled.h_advance(gid);
                // If pixel falls within this character, return its index.
                if x + adv * 0.5 >= pixel_position as f32 {
                    return i as u32;
                }
                x += adv;
                prev = Some(gid);
            }
            text_str.chars().count() as u32
        })
        .unwrap_or(0)
}

unsafe extern "C" fn pixel_offset_for_character(
    font: PP_Resource,
    text: *const PP_BrowserFont_Trusted_TextRun,
    char_offset: u32,
) -> i32 {
    let Some(host) = HOST.get() else { return 0 };

    if text.is_null() { return 0; }
    let run = unsafe { &*text };
    let text_str = host.vars.get_string(run.text).unwrap_or_default();
    if text_str.is_empty() { return 0; }

    host.resources
        .with_downcast::<BrowserFontResource, _>(font, |bf| {
            use ab_glyph::{Font, PxScale, ScaleFont};
            let scale = PxScale::from(bf.size as f32);
            let scaled = bf.font.as_scaled(scale);

            let mut x = 0.0f32;
            let mut prev: Option<ab_glyph::GlyphId> = None;
            for (i, ch) in text_str.chars().enumerate() {
                if i as u32 >= char_offset {
                    break;
                }
                let gid = bf.font.glyph_id(ch);
                if let Some(p) = prev {
                    x += scaled.kern(p, gid);
                }
                x += scaled.h_advance(gid);
                prev = Some(gid);
            }
            x.round() as i32
        })
        .unwrap_or(0)
}
