//! PPB_Flash_FontFile;0.1/0.2 implementation.
//!
//! Flash uses this to access font table data (OpenType/TrueType tables) from
//! a font file.  Font resolution uses `fontdb` (via `font_rasterizer`) and
//! table extraction uses `ttf-parser`.
//!
//! Flash primarily queries the `cmap`, `head`, `OS/2`, and other tables
//! needed for text layout.

use crate::font_rasterizer;
use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::c_void;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

pub struct FlashFontFileResource {
    /// Raw font file bytes (eagerly loaded from fontdb during Create).
    pub font_data: Vec<u8>,
    /// Face index within a TrueType Collection (0 for single-face fonts).
    pub face_index: u32,
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
    let family_name = host.vars.get_string(desc.face);
    let bold = desc.weight >= PP_BROWSERFONT_TRUSTED_WEIGHT_BOLD;
    let italic = pp_to_bool(desc.italic);

    let Some((font_data, face_index)) = font_rasterizer::resolve_system_font_data(
        family_name.as_deref(),
        desc.family,
        bold,
        italic,
    ) else {
        tracing::warn!("PPB_Flash_FontFile::Create: could not resolve font");
        return 0;
    };

    tracing::debug!(
        "PPB_Flash_FontFile::Create: resolved font ({} bytes, face_index={})",
        font_data.len(), face_index
    );

    let res = FlashFontFileResource {
        font_data,
        face_index,
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

    // Extract the table using ttf-parser via font_rasterizer.
    let table_data = host.resources.with_downcast::<FlashFontFileResource, _>(font_file, |res| {
        font_rasterizer::extract_sfnt_table(&res.font_data, res.face_index, table)
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
