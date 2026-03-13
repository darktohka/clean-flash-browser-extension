//! PPB_Flash_FontFile;0.1/0.2 implementation.
//!
//! Flash uses this to access font table data (OpenType/TrueType tables) from
//! a font file. We use fontconfig + freetype2 via the `freetype` crate to
//! resolve font descriptions and extract raw SFNT table data.
//!
//! If freetype is not available, Create returns 0 and GetFontTable returns
//! PP_FALSE, which causes Flash to fall back to its built-in font rendering.
//!
//! For a lightweight standalone player we provide a working implementation
//! that loads system fonts. Flash primarily queries the 'cmap', 'head', 'OS/2',
//! and other tables needed for text layout.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::c_void;
use std::path::PathBuf;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

pub struct FlashFontFileResource {
    /// The path to the font file on disk.
    pub font_path: Option<PathBuf>,
    /// Cached raw font file bytes (loaded lazily).
    pub font_data: Option<Vec<u8>>,
}

impl Resource for FlashFontFileResource {
    fn resource_type(&self) -> &'static str {
        "PPB_Flash_FontFile"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Font resolution helper
// ---------------------------------------------------------------------------

/// Try to find a font file path matching the given description.
/// Uses fontconfig via the `fc-match` command for simplicity.
fn resolve_font_path(description: &PP_BrowserFont_Trusted_Description) -> Option<PathBuf> {
    // Extract family name from the PP_Var
    let family_name = {
        let host = HOST.get()?;
        host.vars.get_string(description.face)
    };

    let family = family_name.unwrap_or_else(|| {
        // Map family enum to generic name
        match description.family {
            1 => "serif".to_string(),
            2 => "sans-serif".to_string(),
            3 => "monospace".to_string(),
            _ => "sans-serif".to_string(),
        }
    });

    // Try fontconfig via command line
    let output = std::process::Command::new("fc-match")
        .arg("--format=%{file}")
        .arg(&family)
        .output()
        .ok()?;

    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout);
        let path = PathBuf::from(path_str.trim());
        if path.exists() {
            return Some(path);
        }
    }

    // Fallback: try common system font paths
    let fallbacks = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    ];
    for path in &fallbacks {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    None
}

/// Extract a specific SFNT table from raw font data.
///
/// Returns the table bytes or None if not found. Uses minimal manual
/// parsing of the OpenType/TrueType file header.
fn extract_sfnt_table(font_data: &[u8], table_tag: u32) -> Option<Vec<u8>> {
    if font_data.len() < 12 {
        return None;
    }

    // Read number of tables from offset 4 (big-endian u16)
    let num_tables = u16::from_be_bytes([font_data[4], font_data[5]]) as usize;

    // Table directory starts at offset 12
    for i in 0..num_tables {
        let offset = 12 + i * 16;
        if offset + 16 > font_data.len() {
            break;
        }
        let tag = u32::from_be_bytes([
            font_data[offset],
            font_data[offset + 1],
            font_data[offset + 2],
            font_data[offset + 3],
        ]);
        if tag == table_tag {
            let table_offset = u32::from_be_bytes([
                font_data[offset + 8],
                font_data[offset + 9],
                font_data[offset + 10],
                font_data[offset + 11],
            ]) as usize;
            let table_length = u32::from_be_bytes([
                font_data[offset + 12],
                font_data[offset + 13],
                font_data[offset + 14],
                font_data[offset + 15],
            ]) as usize;

            if table_offset + table_length <= font_data.len() {
                return Some(font_data[table_offset..table_offset + table_length].to_vec());
            }
        }
    }

    // If table_tag is 0, return the entire font file (Flash sometimes queries
    // the whole file this way).
    if table_tag == 0 {
        return Some(font_data.to_vec());
    }

    None
}

// ---------------------------------------------------------------------------
// Vtables
// ---------------------------------------------------------------------------

static VTABLE_0_2: PPB_Flash_FontFile_0_2 = PPB_Flash_FontFile_0_2 {
    Create: Some(create),
    IsFlashFontFile: Some(is_flash_font_file),
    GetFontTable: Some(get_font_table),
    IsSupportedForWindows: Some(is_supported_for_windows),
};

static VTABLE_0_1: PPB_Flash_FontFile_0_1 = PPB_Flash_FontFile_0_1 {
    Create: Some(create),
    IsFlashFontFile: Some(is_flash_font_file),
    GetFontTable: Some(get_font_table),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASH_FONTFILE_INTERFACE_0_2, &VTABLE_0_2);
        registry.register(PPB_FLASH_FONTFILE_INTERFACE_0_1, &VTABLE_0_1);
    }
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

unsafe extern "C" fn create(
    instance: PP_Instance,
    description: *const PP_BrowserFont_Trusted_Description,
    _charset: PP_PrivateFontCharset,
) -> PP_Resource {
    tracing::debug!("PPB_Flash_FontFile::Create(instance={})", instance);

    let Some(host) = HOST.get() else { return 0 };

    if description.is_null() {
        return 0;
    }

    let desc = unsafe { &*description };
    let font_path = resolve_font_path(desc);

    if font_path.is_none() {
        tracing::warn!("PPB_Flash_FontFile::Create: could not resolve font");
    }

    let res = FlashFontFileResource {
        font_path,
        font_data: None,
    };
    host.resources.insert(instance, Box::new(res))
}

unsafe extern "C" fn is_flash_font_file(resource: PP_Resource) -> PP_Bool {
    tracing::trace!("PPB_Flash_FontFile::IsFlashFontFile(resource={})", resource);
    let Some(host) = HOST.get() else { return PP_FALSE };
    pp_from_bool(host.resources.is_type(resource, "PPB_Flash_FontFile"))
}

unsafe extern "C" fn get_font_table(
    font_file: PP_Resource,
    table: u32,
    output: *mut c_void,
    output_length: *mut u32,
) -> PP_Bool {
    tracing::debug!(
        "PPB_Flash_FontFile::GetFontTable(font_file={}, table=0x{:08x})",
        font_file, table
    );

    let Some(host) = HOST.get() else { return PP_FALSE };

    // Ensure font data is loaded
    let loaded = host.resources.with_downcast_mut::<FlashFontFileResource, _>(font_file, |res| {
        if res.font_data.is_none() {
            if let Some(ref path) = res.font_path {
                match std::fs::read(path) {
                    Ok(data) => {
                        tracing::debug!("Loaded font file: {} ({} bytes)", path.display(), data.len());
                        res.font_data = Some(data);
                    }
                    Err(e) => {
                        tracing::error!("Failed to read font file {}: {}", path.display(), e);
                    }
                }
            }
        }
        res.font_data.is_some()
    });

    if loaded != Some(true) {
        return PP_FALSE;
    }

    // Extract the table
    let table_data = host.resources.with_downcast::<FlashFontFileResource, _>(font_file, |res| {
        res.font_data.as_ref().and_then(|data| extract_sfnt_table(data, table))
    }).flatten();

    let Some(table_data) = table_data else {
        return PP_FALSE;
    };

    if output.is_null() {
        // Query mode: just return the size
        if !output_length.is_null() {
            unsafe { *output_length = table_data.len() as u32 };
        }
        return PP_TRUE;
    }

    // Copy data to output buffer
    if !output_length.is_null() {
        let avail = unsafe { *output_length } as usize;
        let copy_len = avail.min(table_data.len());
        unsafe {
            std::ptr::copy_nonoverlapping(table_data.as_ptr(), output as *mut u8, copy_len);
            *output_length = table_data.len() as u32;
        }
    }

    PP_TRUE
}

unsafe extern "C" fn is_supported_for_windows() -> PP_Bool {
    tracing::trace!("PPB_Flash_FontFile::IsSupportedForWindows()");
    // We're on Linux, but Flash checks this. Return TRUE so it proceeds.
    PP_TRUE
}
