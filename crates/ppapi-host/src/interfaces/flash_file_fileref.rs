//! PPB_Flash_File_FileRef;2 implementation.
//!
//! Flash uses this interface to open/query files referred to by PPB_FileRef
//! resources (typically files chosen via the file chooser dialog).
//! The two functions are OpenFile (returns a file handle) and QueryFile
//! (fills a PP_FileInfo struct).
//!
//! File I/O is delegated to the active [`FlashFileSystem`] provider
//! (see [`crate::filesystem`]).

use crate::filesystem;
use crate::interface_registry::InterfaceRegistry;
use super::file_ref::FileRefResource;
use ppapi_sys::*;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Vtable
// ---------------------------------------------------------------------------

static VTABLE: PPB_Flash_File_FileRef = PPB_Flash_File_FileRef {
    OpenFile: Some(open_file),
    QueryFile: Some(query_file),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASH_FILE_FILEREF_INTERFACE_2, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

/// Map PPAPI file open flags to the mode integer expected by the filesystem
/// provider.  (The provider accepts PP_FILEOPENFLAG_* bits directly.)
fn pp_mode_to_provider_flags(mode: i32) -> i32 {
    // The filesystem provider already understands PP_FILEOPENFLAG_* bits,
    // so pass them through unchanged.
    mode
}

unsafe extern "C" fn open_file(
    file_ref_id: PP_Resource,
    mode: i32,
    file: *mut PP_FileHandle,
) -> i32 {
    tracing::debug!(
        "PPB_Flash_File_FileRef::OpenFile(file_ref={}, mode=0x{:x})",
        file_ref_id, mode
    );

    if file.is_null() {
        return PP_ERROR_BADARGUMENT;
    }

    let Some(host) = HOST.get() else { return PP_ERROR_FAILED };

    let path = host.resources.with_downcast::<FileRefResource, _>(file_ref_id, |fr| {
        fr.path.clone()
    }).flatten();

    let Some(path) = path else {
        tracing::error!("PPB_Flash_File_FileRef::OpenFile: bad resource or no path");
        return PP_ERROR_BADRESOURCE;
    };

    let _flags = pp_mode_to_provider_flags(mode);
    let fs = filesystem::get_filesystem();
    tracing::trace!("PPB_Flash_File_FileRef::OpenFile: opening path '{}' with mode 0x{:x}", path, mode);
    match fs.open_file(&path, mode) {
        Ok(handle) => {
            unsafe { *file = handle };
            PP_OK
        }
        Err(e) => e,
    }
}

unsafe extern "C" fn query_file(
    file_ref_id: PP_Resource,
    info: *mut PP_FileInfo,
) -> i32 {
    tracing::debug!(
        "PPB_Flash_File_FileRef::QueryFile(file_ref={})",
        file_ref_id
    );

    if info.is_null() {
        return PP_ERROR_BADARGUMENT;
    }

    let Some(host) = HOST.get() else { return PP_ERROR_FAILED };

    // Get the file ref's path and/or fd
    let ref_data = host.resources.with_downcast::<FileRefResource, _>(file_ref_id, |fr| {
        (fr.file_type, fr.path.clone(), fr.fd)
    });

    let Some((file_type, path, fd)) = ref_data else {
        tracing::error!("PPB_Flash_File_FileRef::QueryFile: bad resource");
        return PP_ERROR_BADRESOURCE;
    };

    let fs = filesystem::get_filesystem();

    let fi = if file_type == super::file_ref::FileRefType::Name {
        let path_str = path.as_deref().unwrap_or("");
        tracing::trace!("PPB_Flash_File_FileRef::QueryFile: querying path '{}'", path_str);
        match fs.query_file(path_str) {
            Ok(fi) => fi,
            Err(e) => return e,
        }
    } else if let Some(fd_val) = fd {
        tracing::trace!("PPB_Flash_File_FileRef::QueryFile: querying fd {}", fd_val);
        match fs.fstat_handle(fd_val) {
            Ok(fi) => fi,
            Err(e) => return e,
        }
    } else {
        return PP_ERROR_FAILED;
    };

    let pp_type = if fi.is_file {
        PP_FILETYPE_REGULAR
    } else if fi.is_dir {
        PP_FILETYPE_DIRECTORY
    } else {
        PP_FILETYPE_OTHER
    };

    unsafe {
        *info = PP_FileInfo {
            size: fi.size,
            type_: pp_type,
            system_type: PP_FILESYSTEMTYPE_EXTERNAL,
            creation_time: fi.creation_time,
            last_access_time: fi.last_access_time,
            last_modified_time: fi.last_modified_time,
        };
    }
    PP_OK
}
