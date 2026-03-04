//! PPB_Flash_File_FileRef;2 implementation.
//!
//! Flash uses this interface to open/query files referred to by PPB_FileRef
//! resources (typically files chosen via the file chooser dialog).
//! The two functions are OpenFile (returns a file descriptor) and QueryFile
//! (fills a PP_FileInfo struct).

use crate::interface_registry::InterfaceRegistry;
use super::file_ref::FileRefResource;
use ppapi_sys::*;
use std::ffi::CString;

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

/// Map PPAPI file open flags to libc open() flags.
fn pp_mode_to_open_flags(mode: i32) -> i32 {
    let mut flags = 0i32;

    let read = mode & PP_FILEOPENFLAG_READ != 0;
    let write = mode & PP_FILEOPENFLAG_WRITE != 0;

    if read && write {
        flags |= libc::O_RDWR;
    } else if write {
        flags |= libc::O_WRONLY;
    } else {
        flags |= libc::O_RDONLY;
    }

    if mode & PP_FILEOPENFLAG_CREATE != 0 {
        flags |= libc::O_CREAT;
    }
    if mode & PP_FILEOPENFLAG_TRUNCATE != 0 {
        flags |= libc::O_TRUNC;
    }
    if mode & PP_FILEOPENFLAG_EXCLUSIVE != 0 {
        flags |= libc::O_EXCL;
    }
    if mode & PP_FILEOPENFLAG_APPEND != 0 {
        flags |= libc::O_APPEND;
    }
    flags
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

    let flags = pp_mode_to_open_flags(mode);
    let c_path = match CString::new(path.as_bytes()) {
        Ok(c) => c,
        Err(_) => return PP_ERROR_FAILED,
    };

    let fd = unsafe { libc::open(c_path.as_ptr(), flags, 0o644) };
    if fd < 0 {
        let err = unsafe { *libc::__errno_location() };
        return match err {
            libc::ENOENT => PP_ERROR_FILENOTFOUND,
            libc::EACCES => PP_ERROR_NOACCESS,
            _ => PP_ERROR_FAILED,
        };
    }

    unsafe { *file = fd };
    PP_OK
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

    let mut sb: libc::stat = unsafe { std::mem::zeroed() };
    let ret = if file_type == super::file_ref::FileRefType::Name {
        let path_str = path.as_deref().unwrap_or("");
        let c_path = match CString::new(path_str.as_bytes()) {
            Ok(c) => c,
            Err(_) => return PP_ERROR_FAILED,
        };
        unsafe { libc::stat(c_path.as_ptr(), &mut sb) }
    } else if let Some(fd_val) = fd {
        unsafe { libc::fstat(fd_val, &mut sb) }
    } else {
        return PP_ERROR_FAILED;
    };

    if ret == -1 {
        let err = unsafe { *libc::__errno_location() };
        return match err {
            libc::ENOENT => PP_ERROR_FILENOTFOUND,
            libc::EACCES => PP_ERROR_NOACCESS,
            _ => PP_ERROR_FAILED,
        };
    }

    let pp_type = if (sb.st_mode & libc::S_IFMT) == libc::S_IFREG {
        PP_FILETYPE_REGULAR
    } else if (sb.st_mode & libc::S_IFMT) == libc::S_IFDIR {
        PP_FILETYPE_DIRECTORY
    } else {
        PP_FILETYPE_OTHER
    };

    unsafe {
        *info = PP_FileInfo {
            size: sb.st_size,
            type_: pp_type,
            system_type: PP_FILESYSTEMTYPE_EXTERNAL,
            creation_time: sb.st_ctime as f64,
            last_access_time: sb.st_atime as f64,
            last_modified_time: sb.st_mtime as f64,
        };
    }
    PP_OK
}
