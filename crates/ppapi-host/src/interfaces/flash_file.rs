//! PPB_Flash_File_ModuleLocal;3 implementation.
//!
//! Flash uses this to store Local Shared Objects (LSOs/cookies) and settings.
//! We map everything under a data directory.
//!
//! File I/O is delegated to the active [`FlashFileSystem`] provider
//! (see [`crate::filesystem`]).

use crate::filesystem;
use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

static VTABLE: PPB_Flash_File_ModuleLocal_3 = PPB_Flash_File_ModuleLocal_3 {
    CreateThreadAdapterForInstance: Some(create_thread_adapter),
    ClearThreadAdapterForInstance: Some(clear_thread_adapter),
    OpenFile: Some(open_file),
    RenameFile: Some(rename_file),
    DeleteFileOrDir: Some(delete_file_or_dir),
    CreateDir: Some(create_dir),
    QueryFile: Some(query_file),
    GetDirContents: Some(get_dir_contents),
    FreeDirContents: Some(free_dir_contents),
    CreateTemporaryFile: Some(create_temporary_file),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASH_FILE_MODULELOCAL_INTERFACE_3, &VTABLE);
    }
}

/// Get the Flash data directory.
fn data_dir() -> PathBuf {
    #[cfg(unix)]
    let base = {
        std::env::var("XDG_DATA_HOME")
            .or_else(|_| std::env::var("HOME").map(|h| format!("{}/.local/share", h)))
            .unwrap_or_else(|_| "/tmp".to_string())
    };
    #[cfg(windows)]
    let base = {
        std::env::var("APPDATA")
            .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned())
    };
    #[cfg(not(any(unix, windows)))]
    let base = std::env::temp_dir().to_string_lossy().into_owned();

    let dir = PathBuf::from(base).join("flash-player").join("PepperFlash");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Resolve a Flash-relative path to an absolute path, preventing traversal.
fn resolve_path(rel: &str) -> Option<PathBuf> {
    let root = data_dir();
    // Strip leading '/' and sanitize
    let rel = rel.trim_start_matches('/');
    let full = root.join(rel);
    // Canonicalize to prevent ".." traversal
    // If the path doesn't exist yet, check the parent
    let canonical = if full.exists() {
        full.canonicalize().ok()?
    } else {
        let parent = full.parent()?;
        let _ = std::fs::create_dir_all(parent);
        if parent.exists() {
            let canon_parent = parent.canonicalize().ok()?;
            canon_parent.join(full.file_name()?)
        } else {
            return None;
        }
    };
    let root_canon = root.canonicalize().unwrap_or(root);
    if canonical.starts_with(&root_canon) {
        Some(canonical)
    } else {
        None
    }
}

unsafe extern "C" fn create_thread_adapter(_instance: PP_Instance) -> bool {
    tracing::debug!("PPB_Flash_File_ModuleLocal::CreateThreadAdapterForInstance");
    true
}

unsafe extern "C" fn clear_thread_adapter(_instance: PP_Instance) {
    tracing::debug!("PPB_Flash_File_ModuleLocal::ClearThreadAdapterForInstance");
}

unsafe extern "C" fn open_file(
    _instance: PP_Instance,
    path: *const c_char,
    mode: i32,
    file: *mut PP_FileHandle,
) -> i32 {
    tracing::debug!(
        "PPB_Flash_File_ModuleLocal::OpenFile(path={:?}, mode={})",
        unsafe { CStr::from_ptr(path).to_string_lossy() },
        mode
    );
    if path.is_null() || file.is_null() {
        return PP_ERROR_BADARGUMENT;
    }
    let path_str = unsafe { CStr::from_ptr(path) }.to_str().unwrap_or("");
    let Some(abs_path) = resolve_path(path_str) else {
        return PP_ERROR_NOACCESS;
    };

    let fs = filesystem::get_filesystem();
    match fs.open_file(&abs_path.to_string_lossy(), mode) {
        Ok(handle) => {
            unsafe { *file = handle };
            PP_OK
        }
        Err(e) => e,
    }
}

unsafe extern "C" fn rename_file(
    _instance: PP_Instance,
    path_from: *const c_char,
    path_to: *const c_char,
) -> i32 {
    tracing::debug!(
        "PPB_Flash_File_ModuleLocal::RenameFile(path_from={:?}, path_to={:?})",
        unsafe { CStr::from_ptr(path_from).to_string_lossy() },
        unsafe { CStr::from_ptr(path_to).to_string_lossy() }
    );
    if path_from.is_null() || path_to.is_null() {
        return PP_ERROR_BADARGUMENT;
    }
    let from = unsafe { CStr::from_ptr(path_from) }.to_str().unwrap_or("");
    let to = unsafe { CStr::from_ptr(path_to) }.to_str().unwrap_or("");
    let Some(abs_from) = resolve_path(from) else { return PP_ERROR_NOACCESS };
    let Some(abs_to) = resolve_path(to) else { return PP_ERROR_NOACCESS };

    match std::fs::rename(&abs_from, &abs_to) {
        Ok(()) => PP_OK,
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => PP_ERROR_FILENOTFOUND,
            std::io::ErrorKind::PermissionDenied => PP_ERROR_NOACCESS,
            _ => PP_ERROR_FAILED,
        },
    }
}

unsafe extern "C" fn delete_file_or_dir(
    _instance: PP_Instance,
    path: *const c_char,
    recursive: PP_Bool,
) -> i32 {
    tracing::debug!(
        "PPB_Flash_File_ModuleLocal::DeleteFileOrDir(path={:?}, recursive={})",
        unsafe { CStr::from_ptr(path).to_string_lossy() },
        pp_to_bool(recursive)
    );
    if path.is_null() {
        return PP_ERROR_BADARGUMENT;
    }
    let path_str = unsafe { CStr::from_ptr(path) }.to_str().unwrap_or("");
    let Some(abs_path) = resolve_path(path_str) else { return PP_ERROR_NOACCESS };

    let result = if abs_path.is_dir() {
        if pp_to_bool(recursive) {
            std::fs::remove_dir_all(&abs_path)
        } else {
            std::fs::remove_dir(&abs_path)
        }
    } else {
        std::fs::remove_file(&abs_path)
    };

    match result {
        Ok(()) => PP_OK,
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => PP_ERROR_FILENOTFOUND,
            std::io::ErrorKind::PermissionDenied => PP_ERROR_NOACCESS,
            _ => PP_ERROR_FAILED,
        },
    }
}

unsafe extern "C" fn create_dir(
    _instance: PP_Instance,
    path: *const c_char,
) -> i32 {
    tracing::debug!(
        "PPB_Flash_File_ModuleLocal::CreateDir(path={:?})",
        unsafe { CStr::from_ptr(path).to_string_lossy() },
    );

    if path.is_null() {
        return PP_ERROR_BADARGUMENT;
    }
    let path_str = unsafe { CStr::from_ptr(path) }.to_str().unwrap_or("");
    let Some(abs_path) = resolve_path(path_str) else { return PP_ERROR_NOACCESS };

    // Check if directory already exists; if so, return OK (idempotent)
    if abs_path.is_dir() {
        return PP_OK;
    }

    match std::fs::create_dir_all(&abs_path) {
        Ok(()) => PP_OK,
        Err(e) => match e.kind() {
            std::io::ErrorKind::PermissionDenied => PP_ERROR_NOACCESS,
            _ => PP_ERROR_FAILED,
        },
    }
}

unsafe extern "C" fn query_file(
    _instance: PP_Instance,
    path: *const c_char,
    info: *mut PP_FileInfo,
) -> i32 {
    tracing::debug!(
        "PPB_Flash_File_ModuleLocal::QueryFile(path={:?})",
        unsafe { CStr::from_ptr(path).to_string_lossy() },
    );
    if path.is_null() || info.is_null() {
        return PP_ERROR_BADARGUMENT;
    }
    let path_str = unsafe { CStr::from_ptr(path) }.to_str().unwrap_or("");
    let Some(abs_path) = resolve_path(path_str) else { return PP_ERROR_FILENOTFOUND };

    let meta = match std::fs::metadata(&abs_path) {
        Ok(m) => m,
        Err(_) => return PP_ERROR_FILENOTFOUND,
    };

    let file_type = if meta.is_dir() {
        PP_FILETYPE_DIRECTORY
    } else if meta.is_file() {
        PP_FILETYPE_REGULAR
    } else {
        PP_FILETYPE_OTHER
    };
    
    // Get file times; convert SystemTime to seconds since UNIX_EPOCH
    let creation_time = meta.created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let last_access_time = meta.accessed()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let last_modified_time = meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    unsafe {
        *info = PP_FileInfo {
            size: meta.len() as i64,
            type_: file_type,
            system_type: PP_FILESYSTEMTYPE_ISOLATED,
            creation_time,
            last_access_time,
            last_modified_time,
        };
    }
    PP_OK
}

unsafe extern "C" fn get_dir_contents(
    _instance: PP_Instance,
    path: *const c_char,
    contents: *mut *mut PP_DirContents_Dev,
) -> i32 {
    tracing::debug!(
        "PPB_Flash_File_ModuleLocal::GetDirContents(path={:?})",
        unsafe { CStr::from_ptr(path).to_string_lossy() },
    );
    if path.is_null() || contents.is_null() {
        return PP_ERROR_BADARGUMENT;
    }
    let path_str = unsafe { CStr::from_ptr(path) }.to_str().unwrap_or("");
    let Some(abs_path) = resolve_path(path_str) else { return PP_ERROR_FILENOTFOUND };

    let entries: Vec<_> = match std::fs::read_dir(&abs_path) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return PP_ERROR_FAILED,
    };

    let count = entries.len() as i32;

    let mut dir_contents = Box::new(PP_DirContents_Dev {
        count,
        entries: std::ptr::null_mut(),
    });

    // Allocate entries array only if needed. For empty directories, keep
    // entries=NULL and count=0 exactly like C implementations typically do.
    let entries_ptr = if entries.is_empty() {
        std::ptr::null_mut()
    } else {
        let layout = std::alloc::Layout::array::<PP_DirEntry_Dev>(entries.len()).unwrap();
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) as *mut PP_DirEntry_Dev };
        if ptr.is_null() {
            return PP_ERROR_FAILED;
        }
        ptr
    };

    for (i, entry) in entries.iter().enumerate() {
        let name = entry.file_name();
        let name_c = CString::new(name.to_string_lossy().as_bytes()).unwrap_or_default();
        let name_ptr = name_c.into_raw(); // reclaimed in FreeDirContents
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        unsafe {
            *entries_ptr.add(i) = PP_DirEntry_Dev {
                name: name_ptr,
                is_dir: pp_from_bool(is_dir),
            };
        }
    }

    dir_contents.entries = entries_ptr;

    tracing::trace!("PPB_Flash_File_ModuleLocal::GetDirContents: found {} entries", count);

    unsafe {
        *contents = Box::into_raw(dir_contents);
    }
    PP_OK
}

unsafe extern "C" fn free_dir_contents(
    _instance: PP_Instance,
    contents: *mut PP_DirContents_Dev,
) {
    tracing::debug!(
        "PPB_Flash_File_ModuleLocal::FreeDirContents(contents={:?})",
        contents
    );
    if contents.is_null() {
        return;
    }

    let contents_box = unsafe { Box::from_raw(contents) };

    for i in 0..contents_box.count as usize {
        let entry = unsafe { &*contents_box.entries.add(i) };
        if !entry.name.is_null() {
            let _ = unsafe { CString::from_raw(entry.name as *mut c_char) };
        }
    }

    if !contents_box.entries.is_null() && contents_box.count > 0 {
        let layout =
            std::alloc::Layout::array::<PP_DirEntry_Dev>(contents_box.count as usize).unwrap();
        unsafe { std::alloc::dealloc(contents_box.entries as *mut u8, layout) };
    }
}

unsafe extern "C" fn create_temporary_file(
    _instance: PP_Instance,
    file: *mut PP_FileHandle,
) -> i32 {
    tracing::debug!(
        "PPB_Flash_File_ModuleLocal::CreateTemporaryFile(file={:?})",
        file
    );
    if file.is_null() {
        return PP_ERROR_BADARGUMENT;
    }
    let dir = data_dir();
    let fs = filesystem::get_filesystem();
    match fs.create_temp_file(&dir.to_string_lossy()) {
        Ok(handle) => {
            unsafe { *file = handle };
            PP_OK
        }
        Err(e) => e,
    }
}
