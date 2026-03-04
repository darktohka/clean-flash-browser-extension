//! PPB_Flash_File_ModuleLocal;3 implementation.
//!
//! Flash uses this to store Local Shared Objects (LSOs/cookies) and settings.
//! We map everything under a data directory.

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
    let base = std::env::var("XDG_DATA_HOME")
        .or_else(|_| std::env::var("HOME").map(|h| format!("{}/.local/share", h)))
        .unwrap_or_else(|_| "/tmp".to_string());
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

unsafe extern "C" fn create_thread_adapter(_instance: PP_Instance) -> PP_Bool {
    tracing::debug!("PPB_Flash_File_ModuleLocal::CreateThreadAdapterForInstance");
    PP_TRUE
}

unsafe extern "C" fn clear_thread_adapter(_instance: PP_Instance) {
    tracing::debug!("PPB_Flash_File_ModuleLocal::ClearThreadAdapterForInstance");
}

unsafe extern "C" fn open_file(
    _instance: PP_Instance,
    path: *const c_char,
    mode: i32,
    file: *mut i32,
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

    // Map mode flags to libc flags
    let mut flags = 0i32;
    if mode & 1 != 0 { flags |= libc::O_RDONLY; }
    if mode & 2 != 0 { flags |= libc::O_WRONLY; }
    if mode & 4 != 0 { flags |= libc::O_RDWR; }
    if mode & 8 != 0 { flags |= libc::O_CREAT; }
    if mode & 16 != 0 { flags |= libc::O_TRUNC; }
    // Ensure parent dir exists
    if let Some(parent) = abs_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let c_path = match CString::new(abs_path.to_string_lossy().as_bytes()) {
        Ok(c) => c,
        Err(_) => return PP_ERROR_FAILED,
    };

    let fd = unsafe { libc::open(c_path.as_ptr(), flags, 0o644) };
    if fd < 0 {
        return PP_ERROR_FAILED;
    }
    unsafe { *file = fd };
    PP_OK
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
        Err(_) => PP_ERROR_FAILED,
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
        Err(_) => PP_ERROR_FAILED,
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

    match std::fs::create_dir_all(&abs_path) {
        Ok(()) => PP_OK,
        Err(_) => PP_ERROR_FAILED,
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

    unsafe {
        *info = PP_FileInfo {
            size: meta.len() as i64,
            type_: file_type,
            system_type: 0,
            creation_time: 0.0,
            last_access_time: 0.0,
            last_modified_time: 0.0,
        };
    }
    PP_OK
}

unsafe extern "C" fn get_dir_contents(
    _instance: PP_Instance,
    path: *const c_char,
    contents: *mut PP_DirContents_Dev,
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
    let layout = std::alloc::Layout::array::<PP_DirEntry_Dev>(entries.len()).unwrap();
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) as *mut PP_DirEntry_Dev };

    for (i, entry) in entries.iter().enumerate() {
        let name = entry.file_name();
        let name_c = CString::new(name.to_string_lossy().as_bytes()).unwrap_or_default();
        let name_ptr = name_c.into_raw(); // leak, freed in FreeDirContents
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        unsafe {
            *ptr.add(i) = PP_DirEntry_Dev {
                name: name_ptr,
                is_dir: pp_from_bool(is_dir),
            };
        }
    }

    unsafe {
        *contents = PP_DirContents_Dev {
            count,
            entries: ptr,
        };
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
    let c = unsafe { &*contents };
    for i in 0..c.count as usize {
        let entry = unsafe { &*c.entries.add(i) };
        if !entry.name.is_null() {
            // Reclaim the CString we leaked in get_dir_contents
            let _ = unsafe { CString::from_raw(entry.name as *mut c_char) };
        }
    }
    if !c.entries.is_null() && c.count > 0 {
        let layout = std::alloc::Layout::array::<PP_DirEntry_Dev>(c.count as usize).unwrap();
        unsafe { std::alloc::dealloc(c.entries as *mut u8, layout) };
    }
}

unsafe extern "C" fn create_temporary_file(
    _instance: PP_Instance,
    file: *mut i32,
) -> i32 {
    tracing::debug!(
        "PPB_Flash_File_ModuleLocal::CreateTemporaryFile(file={:?})",
        file
    );
    if file.is_null() {
        return PP_ERROR_BADARGUMENT;
    }
    let dir = data_dir();
    let template = format!("{}/tmpXXXXXX", dir.to_string_lossy());
    let c_template = match CString::new(template.as_bytes()) {
        Ok(c) => c,
        Err(_) => return PP_ERROR_FAILED,
    };
    let mut buf = c_template.into_bytes_with_nul();
    let fd = unsafe { libc::mkstemp(buf.as_mut_ptr() as *mut c_char) };
    if fd < 0 {
        return PP_ERROR_FAILED;
    }
    unsafe { *file = fd };
    PP_OK
}
