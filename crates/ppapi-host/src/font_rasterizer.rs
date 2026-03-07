//! Font rasterization helpers using `ab_glyph`.
//!
//! Provides a global font cache (keyed by file path) and helpers for
//! rasterizing individual glyphs into an BGRA pixel buffer.
//! Used by `PPB_BrowserFont_Trusted::DrawTextAt` and `PPB_Flash::DrawGlyphs`.

use ab_glyph::{Font, FontVec, GlyphId, PxScale, ScaleFont, point};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Global font cache
// ---------------------------------------------------------------------------

static FONT_CACHE: once_cell::sync::Lazy<Mutex<FontCache>> =
    once_cell::sync::Lazy::new(|| Mutex::new(FontCache::new()));

struct FontCache {
    /// Fonts keyed by their file path.
    by_path: HashMap<PathBuf, Arc<FontVec>>,
    /// Embedded fallback font (always available).
    fallback: Arc<FontVec>,
}

/// Embedded fallback: Liberation Sans (Apache 2.0 license).
/// We embed a minimal sans-serif font so text always renders even when
/// system fonts are unavailable.
///
/// If the binary size is a concern this could be feature-gated, but
/// Liberation Sans Regular is ~130 KB which is negligible.
const FALLBACK_FONT_BYTES: &[u8] = include_bytes!("fonts/LiberationSans-Regular.ttf");

impl FontCache {
    fn new() -> Self {
        let fallback = Arc::new(
            FontVec::try_from_vec(FALLBACK_FONT_BYTES.to_vec())
                .expect("embedded fallback font must be valid"),
        );
        Self {
            by_path: HashMap::new(),
            fallback,
        }
    }

    /// Load a font from disk (or return cached). Returns the fallback on failure.
    fn load(&mut self, path: &Path) -> Arc<FontVec> {
        if let Some(cached) = self.by_path.get(path) {
            return Arc::clone(cached);
        }
        match std::fs::read(path) {
            Ok(data) => match FontVec::try_from_vec(data) {
                Ok(font) => {
                    let arc = Arc::new(font);
                    self.by_path.insert(path.to_path_buf(), Arc::clone(&arc));
                    arc
                }
                Err(e) => {
                    tracing::warn!("Failed to parse font {}: {}", path.display(), e);
                    Arc::clone(&self.fallback)
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read font {}: {}", path.display(), e);
                Arc::clone(&self.fallback)
            }
        }
    }

    /// Load font from raw bytes (for FlashFontFile resources that already have data).
    fn load_from_bytes(&mut self, key: &Path, data: &[u8]) -> Arc<FontVec> {
        if let Some(cached) = self.by_path.get(key) {
            return Arc::clone(cached);
        }
        match FontVec::try_from_vec(data.to_vec()) {
            Ok(font) => {
                let arc = Arc::new(font);
                self.by_path.insert(key.to_path_buf(), Arc::clone(&arc));
                arc
            }
            Err(e) => {
                tracing::warn!("Failed to parse font bytes: {}", e);
                Arc::clone(&self.fallback)
            }
        }
    }

    fn fallback(&self) -> Arc<FontVec> {
        Arc::clone(&self.fallback)
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Get a font from a file path, loading from disk (cached).
/// Falls back to the embedded font if the path cannot be loaded.
pub fn get_font(path: &Path) -> Arc<FontVec> {
    FONT_CACHE.lock().load(path)
}

/// Get a font from raw bytes, using `key` as a cache key.
pub fn get_font_from_bytes(key: &Path, data: &[u8]) -> Arc<FontVec> {
    FONT_CACHE.lock().load_from_bytes(key, data)
}

/// Get the embedded fallback font.
pub fn get_fallback_font() -> Arc<FontVec> {
    FONT_CACHE.lock().fallback()
}

// ---------------------------------------------------------------------------
// Font resolution (platform-specific)
// ---------------------------------------------------------------------------

/// Resolve a `PP_BrowserFont_Trusted_Description` family to a system font path.
pub fn resolve_system_font(family_name: Option<&str>, family_enum: i32, bold: bool, italic: bool) -> Option<PathBuf> {
    resolve_system_font_impl(family_name, family_enum, bold, italic)
}

#[cfg(windows)]
fn resolve_system_font_impl(
    family_name: Option<&str>,
    family_enum: i32,
    _bold: bool,
    _italic: bool,
) -> Option<PathBuf> {
    let fonts_dir = {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        PathBuf::from(windir).join("Fonts")
    };

    // If a specific face was requested, try it directly.
    if let Some(name) = family_name {
        let lower = name.to_lowercase();
        // Try exact filename first (e.g. "arial" → "arial.ttf")
        let filename = format!("{}.ttf", lower.replace(' ', ""));
        let path = fonts_dir.join(&filename);
        if path.exists() {
            return Some(path);
        }
        // Common name mappings
        let mapped = match lower.as_str() {
            "arial" | "sans-serif" | "sans" | "helvetica" => "arial.ttf",
            "times new roman" | "times" | "serif" => "times.ttf",
            "courier new" | "courier" | "monospace" => "cour.ttf",
            "verdana" => "verdana.ttf",
            "tahoma" => "tahoma.ttf",
            "trebuchet ms" => "trebuc.ttf",
            "georgia" => "georgia.ttf",
            "comic sans ms" => "comic.ttf",
            "impact" => "impact.ttf",
            "lucida console" => "lucon.ttf",
            "consolas" => "consola.ttf",
            "segoe ui" => "segoeui.ttf",
            _ => "",
        };
        if !mapped.is_empty() {
            let path = fonts_dir.join(mapped);
            if path.exists() {
                return Some(path);
            }
        }
    }

    // Fall back based on family enum.
    let fallback = match family_enum {
        1 => "times.ttf",    // serif
        3 => "cour.ttf",     // monospace
        _ => "arial.ttf",    // sans-serif / default
    };
    let path = fonts_dir.join(fallback);
    if path.exists() {
        return Some(path);
    }

    None
}

#[cfg(unix)]
fn resolve_system_font_impl(
    family_name: Option<&str>,
    family_enum: i32,
    bold: bool,
    italic: bool,
) -> Option<PathBuf> {
    let query = match family_name {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => match family_enum {
            1 => "serif".to_string(),
            3 => "monospace".to_string(),
            _ => "sans-serif".to_string(),
        },
    };

    // Append style hints
    let mut fc_query = query;
    if bold {
        fc_query.push_str(":bold");
    }
    if italic {
        fc_query.push_str(":italic");
    }

    let output = std::process::Command::new("fc-match")
        .arg("--format=%{file}")
        .arg(&fc_query)
        .output()
        .ok()?;

    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout);
        let path = PathBuf::from(path_str.trim());
        if path.exists() {
            return Some(path);
        }
    }

    // Fallback paths
    let fallbacks = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    ];
    for p in &fallbacks {
        let path = PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }

    None
}

#[cfg(not(any(unix, windows)))]
fn resolve_system_font_impl(
    _family_name: Option<&str>,
    _family_enum: i32,
    _bold: bool,
    _italic: bool,
) -> Option<PathBuf> {
    None
}

// ---------------------------------------------------------------------------
// Glyph rasterization into BGRA buffer
// ---------------------------------------------------------------------------

/// Draw a single rasterized glyph onto a BGRA premultiplied pixel buffer.
///
/// - `pixels`: the target BGRA buffer (row-major)
/// - `stride`: bytes per row
/// - `img_w`, `img_h`: image dimensions
/// - `font`: the ab_glyph font
/// - `glyph_id`: which glyph to rasterize
/// - `px_size`: font size in pixels
/// - `x`, `y`: baseline position
/// - `argb_color`: packed ARGB colour
/// - `clip`: optional clip rect (x, y, w, h)
///
/// Returns the glyph's advance width.
pub fn draw_glyph_to_bgra(
    pixels: &mut [u8],
    stride: i32,
    img_w: i32,
    img_h: i32,
    font: &FontVec,
    glyph_id: GlyphId,
    px_size: f32,
    x: f32,
    y: f32,
    argb_color: u32,
    clip: Option<(i32, i32, i32, i32)>,
) -> f32 {
    let scale = PxScale::from(px_size);
    let scaled = font.as_scaled(scale);

    let glyph = glyph_id.with_scale_and_position(scale, point(x, y));

    let advance = scaled.h_advance(glyph_id);

    if let Some(outlined) = font.outline_glyph(glyph) {
        let bounds = outlined.px_bounds();

        // Extract color channels from ARGB
        let a_src = ((argb_color >> 24) & 0xFF) as f32 / 255.0;
        let r_src = ((argb_color >> 16) & 0xFF) as f32 / 255.0;
        let g_src = ((argb_color >> 8) & 0xFF) as f32 / 255.0;
        let b_src = (argb_color & 0xFF) as f32 / 255.0;

        outlined.draw(|px, py, coverage| {
            let gx = bounds.min.x as i32 + px as i32;
            let gy = bounds.min.y as i32 + py as i32;

            // Bounds check
            if gx < 0 || gy < 0 || gx >= img_w || gy >= img_h {
                return;
            }

            // Clip check
            if let Some((cx, cy, cw, ch)) = clip {
                if gx < cx || gy < cy || gx >= cx + cw || gy >= cy + ch {
                    return;
                }
            }

            let alpha = coverage * a_src;
            if alpha < 1.0 / 255.0 {
                return;
            }

            let offset = (gy as usize) * (stride as usize) + (gx as usize) * 4;
            if offset + 3 >= pixels.len() {
                return;
            }

            // Read destination (BGRA premultiplied)
            let dst_b = pixels[offset] as f32 / 255.0;
            let dst_g = pixels[offset + 1] as f32 / 255.0;
            let dst_r = pixels[offset + 2] as f32 / 255.0;
            let dst_a = pixels[offset + 3] as f32 / 255.0;

            // Source-over compositing (premultiplied alpha)
            let src_r_pre = r_src * alpha;
            let src_g_pre = g_src * alpha;
            let src_b_pre = b_src * alpha;

            let out_r = src_r_pre + dst_r * (1.0 - alpha);
            let out_g = src_g_pre + dst_g * (1.0 - alpha);
            let out_b = src_b_pre + dst_b * (1.0 - alpha);
            let out_a = alpha + dst_a * (1.0 - alpha);

            // Write back as BGRA
            pixels[offset]     = (out_b * 255.0 + 0.5) as u8;
            pixels[offset + 1] = (out_g * 255.0 + 0.5) as u8;
            pixels[offset + 2] = (out_r * 255.0 + 0.5) as u8;
            pixels[offset + 3] = (out_a * 255.0 + 0.5) as u8;
        });
    }

    advance
}

/// Draw a UTF-8 text string onto a BGRA premultiplied pixel buffer.
///
/// - `position`: baseline of the left edge
/// - Returns the total advance width.
pub fn draw_text_to_bgra(
    pixels: &mut [u8],
    stride: i32,
    img_w: i32,
    img_h: i32,
    font: &FontVec,
    text: &str,
    px_size: f32,
    x: f32,
    y: f32,
    argb_color: u32,
    clip: Option<(i32, i32, i32, i32)>,
) -> f32 {
    let scale = PxScale::from(px_size);
    let scaled = font.as_scaled(scale);

    let mut cursor_x = x;
    let mut prev_glyph: Option<GlyphId> = None;

    for ch in text.chars() {
        let glyph_id = font.glyph_id(ch);

        // Apply kerning
        if let Some(prev) = prev_glyph {
            cursor_x += scaled.kern(prev, glyph_id);
        }

        let advance = draw_glyph_to_bgra(
            pixels, stride, img_w, img_h,
            font, glyph_id, px_size,
            cursor_x, y,
            argb_color, clip,
        );

        cursor_x += advance;
        prev_glyph = Some(glyph_id);
    }

    cursor_x - x
}
