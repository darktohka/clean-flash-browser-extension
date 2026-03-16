//! Font rasterization and system font management.
//!
//! Uses `fontdb` for cross-platform system font enumeration and matching,
//! `ttf-parser` for OpenType/TrueType table extraction, and `ab_glyph` for
//! glyph rasterization into BGRA pixel buffers.
//!
//! Used by `PPB_BrowserFont_Trusted`, `PPB_Flash::DrawGlyphs`, and
//! `PPB_Flash_FontFile`.

use ab_glyph::{Font, FontVec, GlyphId, PxScale, ScaleFont, point};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

// ---------------------------------------------------------------------------
// Global font database (fontdb – system font enumeration & matching)
// ---------------------------------------------------------------------------

static FONT_DB: OnceLock<Mutex<fontdb::Database>> = OnceLock::new();

/// Get (or lazily initialise) the global font database with all system fonts.
fn font_db() -> &'static Mutex<fontdb::Database> {
    FONT_DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db.set_sans_serif_family("Arial");
        db.set_serif_family("Times New Roman");
        db.set_monospace_family("Courier New");
        let count = db.faces().count();
        tracing::info!("fontdb: loaded {} system font faces", count);
        Mutex::new(db)
    })
}

// ---------------------------------------------------------------------------
// Global ab_glyph font cache
// ---------------------------------------------------------------------------

static FONT_CACHE: OnceLock<Mutex<FontCache>> = OnceLock::new();

struct FontCache {
    /// Fonts keyed by fontdb face ID.
    by_id: HashMap<fontdb::ID, Arc<FontVec>>,
    /// Fonts keyed by file path (for manually loaded fonts).
    by_path: HashMap<PathBuf, Arc<FontVec>>,
    /// Embedded fallback font (always available).
    fallback: Arc<FontVec>,
}

/// Embedded fallback: Liberation Sans (Apache 2.0 license).
const FALLBACK_FONT_BYTES: &[u8] = include_bytes!("fonts/LiberationSans-Regular.ttf");

fn font_cache() -> &'static Mutex<FontCache> {
    FONT_CACHE.get_or_init(|| {
        let fallback = Arc::new(
            FontVec::try_from_vec(FALLBACK_FONT_BYTES.to_vec())
                .expect("embedded fallback font must be valid"),
        );
        Mutex::new(FontCache {
            by_id: HashMap::new(),
            by_path: HashMap::new(),
            fallback,
        })
    })
}

impl FontCache {
    /// Load a font from fontdb by ID (cached by ID).
    fn get_or_load_from_db(&mut self, db: &fontdb::Database, id: fontdb::ID) -> Arc<FontVec> {
        if let Some(cached) = self.by_id.get(&id) {
            return Arc::clone(cached);
        }
        let result = db.with_face_data(id, |data, face_index| {
            FontVec::try_from_vec_and_index(data.to_vec(), face_index)
        });
        match result {
            Some(Ok(font)) => {
                let arc = Arc::new(font);
                self.by_id.insert(id, Arc::clone(&arc));
                arc
            }
            Some(Err(e)) => {
                tracing::warn!("Failed to load font from fontdb (ID {:?}): {}", id, e);
                Arc::clone(&self.fallback)
            }
            None => Arc::clone(&self.fallback),
        }
    }

    /// Load a font from disk (or return cached).
    fn load_from_path(&mut self, path: &Path) -> Arc<FontVec> {
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

    /// Load font from raw bytes.
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
// Public API - font resolution
// ---------------------------------------------------------------------------

/// Map a PPAPI family enum value to a `fontdb::Family`.
fn ppapi_family_to_fontdb(family_enum: i32) -> fontdb::Family<'static> {
    match family_enum {
        1 => fontdb::Family::Serif,
        3 => fontdb::Family::Monospace,
        _ => fontdb::Family::SansSerif, // 0 = default, 2 = sans-serif
    }
}

/// Resolve a system font matching the given criteria.
///
/// Uses `fontdb` for cross-platform font matching (fontconfig on Linux,
/// DirectWrite on Windows, CoreText on macOS under the hood).
///
/// Always returns a valid `FontVec` – falls back to the embedded
/// Liberation Sans if no system font matches.
pub fn resolve_system_font(
    family_name: Option<&str>,
    family_enum: i32,
    bold: bool,
    italic: bool,
) -> Arc<FontVec> {
    let db = font_db().lock();

    let mut families = Vec::new();
    if let Some(name) = family_name {
        if !name.is_empty() {
            families.push(fontdb::Family::Name(name));
        }
    }
    families.push(ppapi_family_to_fontdb(family_enum));

    let query = fontdb::Query {
        families: &families,
        weight: if bold { fontdb::Weight::BOLD } else { fontdb::Weight::NORMAL },
        stretch: fontdb::Stretch::Normal,
        style: if italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
    };

    if let Some(id) = db.query(&query) {
        font_cache().lock().get_or_load_from_db(&db, id)
    } else {
        tracing::debug!(
            "fontdb: no match for family={:?} enum={} bold={} italic={}",
            family_name, family_enum, bold, italic
        );
        font_cache().lock().fallback()
    }
}

/// Resolve a system font and return its raw binary data and face index.
///
/// Used by `PPB_Flash_FontFile` to provide SFNT table data to Flash.
pub fn resolve_system_font_data(
    family_name: Option<&str>,
    family_enum: i32,
    bold: bool,
    italic: bool,
) -> Option<(Vec<u8>, u32)> {
    let db = font_db().lock();

    let mut families = Vec::new();
    if let Some(name) = family_name {
        if !name.is_empty() {
            families.push(fontdb::Family::Name(name));
        }
    }
    families.push(ppapi_family_to_fontdb(family_enum));

    let query = fontdb::Query {
        families: &families,
        weight: if bold { fontdb::Weight::BOLD } else { fontdb::Weight::NORMAL },
        stretch: fontdb::Stretch::Normal,
        style: if italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
    };

    db.query(&query).and_then(|id| {
        db.with_face_data(id, |data, face_index| (data.to_vec(), face_index))
    })
}

/// Enumerate all unique font family names from the system.
pub fn get_font_families() -> Vec<String> {
    let db = font_db().lock();
    let mut families = std::collections::BTreeSet::new();
    for face in db.faces() {
        for (name, _) in &face.families {
            families.insert(name.clone());
        }
    }
    families.into_iter().collect()
}

/// Get a font from a file path, loading from disk (cached).
pub fn get_font(path: &Path) -> Arc<FontVec> {
    font_cache().lock().load_from_path(path)
}

/// Get a font from raw bytes, using `key` as a cache key.
pub fn get_font_from_bytes(key: &Path, data: &[u8]) -> Arc<FontVec> {
    font_cache().lock().load_from_bytes(key, data)
}

/// Get the embedded fallback font.
pub fn get_fallback_font() -> Arc<FontVec> {
    font_cache().lock().fallback()
}

// ---------------------------------------------------------------------------
// SFNT table extraction (via ttf-parser)
// ---------------------------------------------------------------------------

/// Extract an SFNT table from raw font data.
///
/// - `font_data`: raw TrueType/OpenType/TTC file bytes
/// - `face_index`: face index within a font collection (0 for single fonts)
/// - `table_tag`: 4-byte SFNT table tag as a big-endian u32
///   (e.g. `0x636D6170` for `cmap`), or `0` to retrieve the entire file.
pub fn extract_sfnt_table(font_data: &[u8], face_index: u32, table_tag: u32) -> Option<Vec<u8>> {
    if table_tag == 0 {
        return Some(font_data.to_vec());
    }

    let tag = ttf_parser::Tag::from_bytes(&table_tag.to_be_bytes());
    let raw = ttf_parser::RawFace::parse(font_data, face_index).ok()?;
    raw.table(tag).map(|t| t.to_vec())
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
