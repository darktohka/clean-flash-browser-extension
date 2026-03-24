//! PPB_FileRef;1.0 / 1.1 / 1.2 implementation.
//!
//! Provides file reference resources. A FileRef wraps a path (and optionally
//! an fd) so the plugin can query, rename, delete, etc. In our standalone
//! player most filesystem operations return PP_ERROR_FAILED since we don't
//! expose a real PP_FileSystem. The key operations Flash actually uses are
//! `Create`, `IsFileRef`, `GetName`, `GetPath`, and `Query`.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::{c_char, CStr};
use std::path::Path;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Internal type of file reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRefType {
    /// Path-based reference.
    Name,
    /// File-descriptor-based reference.
    Fd,
}

pub struct FileRefResource {
    pub file_type: FileRefType,
    pub path: Option<String>,
    pub fd: Option<PP_FileHandle>,
}

impl Resource for FileRefResource {
    fn resource_type(&self) -> &'static str {
        "PPB_FileRef"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Drop for FileRefResource {
    fn drop(&mut self) {
        if let (FileRefType::Fd, Some(fd)) = (self.file_type, self.fd) {
            crate::filesystem::get_filesystem().close_handle(fd);
        }
    }
}

// ---------------------------------------------------------------------------
// Public helper - create a file ref from an absolute path (for FileChooser)
// ---------------------------------------------------------------------------

/// Create a FileRef resource from an unrestricted absolute path.
/// Used by PPB_FileChooser when the user picks a file.
pub fn create_file_ref_unrestricted(path: &str, instance: PP_Instance) -> PP_Resource {
    let Some(host) = HOST.get() else { return 0 };
    let res = FileRefResource {
        file_type: FileRefType::Name,
        path: Some(path.to_string()),
        fd: None,
    };
    host.resources.insert(instance, Box::new(res))
}

// ---------------------------------------------------------------------------
// Vtables
// ---------------------------------------------------------------------------

static VTABLE_1_2: PPB_FileRef_1_2 = PPB_FileRef_1_2 {
    Create: Some(create),
    IsFileRef: Some(is_file_ref),
    GetFileSystemType: Some(get_file_system_type),
    GetName: Some(get_name),
    GetPath: Some(get_path),
    GetParent: Some(get_parent),
    MakeDirectory: Some(make_directory_1_2),
    Touch: Some(touch),
    Delete: Some(delete),
    Rename: Some(rename),
    Query: Some(query),
    ReadDirectoryEntries: Some(read_directory_entries),
};

static VTABLE_1_1: PPB_FileRef_1_1 = PPB_FileRef_1_1 {
    Create: Some(create),
    IsFileRef: Some(is_file_ref),
    GetFileSystemType: Some(get_file_system_type),
    GetName: Some(get_name),
    GetPath: Some(get_path),
    GetParent: Some(get_parent),
    MakeDirectory: Some(make_directory_1_1),
    Touch: Some(touch),
    Delete: Some(delete),
    Rename: Some(rename),
    Query: Some(query),
    ReadDirectoryEntries: Some(read_directory_entries),
};

static VTABLE_1_0: PPB_FileRef_1_0 = PPB_FileRef_1_0 {
    Create: Some(create),
    IsFileRef: Some(is_file_ref),
    GetFileSystemType: Some(get_file_system_type),
    GetName: Some(get_name),
    GetPath: Some(get_path),
    GetParent: Some(get_parent),
    MakeDirectory: Some(make_directory_1_1),
    Touch: Some(touch),
    Delete: Some(delete),
    Rename: Some(rename),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FILEREF_INTERFACE_1_2, &VTABLE_1_2);
        registry.register(PPB_FILEREF_INTERFACE_1_1, &VTABLE_1_1);
        registry.register(PPB_FILEREF_INTERFACE_1_0, &VTABLE_1_0);
    }
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

unsafe extern "C" fn create(
    _file_system: PP_Resource,
    path: *const c_char,
) -> PP_Resource {
    let path_debug = if path.is_null() { std::borrow::Cow::Borrowed("<null>") } else { CStr::from_ptr(path).to_string_lossy() };
    tracing::debug!("PPB_FileRef::Create(file_system={}, path={:?})",
        _file_system, path_debug);

    let Some(host) = HOST.get() else { return 0 };

    if path.is_null() {
        return 0;
    }
    let path_str = CStr::from_ptr(path).to_str().unwrap_or("");

    // Determine instance from the file_system resource, or use 0.
    let instance = host.resources.get_instance(_file_system).unwrap_or(0);

    let res = FileRefResource {
        file_type: FileRefType::Name,
        path: Some(path_str.to_string()),
        fd: None,
    };
    host.resources.insert(instance, Box::new(res))
}

unsafe extern "C" fn is_file_ref(resource: PP_Resource) -> PP_Bool {
    let Some(host) = HOST.get() else { return PP_FALSE };
    pp_from_bool(host.resources.is_type(resource, "PPB_FileRef"))
}

unsafe extern "C" fn get_file_system_type(_file_ref: PP_Resource) -> PP_FileSystemType {
    tracing::debug!("PPB_FileRef::GetFileSystemType({})", _file_ref);
    // In a standalone player without a real filesystem, report external.
    PP_FILESYSTEMTYPE_EXTERNAL
}

unsafe extern "C" fn get_name(file_ref: PP_Resource) -> PP_Var {
    tracing::debug!("PPB_FileRef::GetName({})", file_ref);
    let Some(host) = HOST.get() else { return PP_Var::undefined() };

    let name = host.resources.with_downcast::<FileRefResource, _>(file_ref, |fr| {
        fr.path.as_deref().map(|p| {
            Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        })
    }).flatten();

    match name {
        Some(n) => host.vars.var_from_str(&n),
        None => PP_Var::undefined(),
    }
}

unsafe extern "C" fn get_path(file_ref: PP_Resource) -> PP_Var {
    tracing::debug!("PPB_FileRef::GetPath({})", file_ref);
    let Some(host) = HOST.get() else { return PP_Var::undefined() };

    let path = host.resources.with_downcast::<FileRefResource, _>(file_ref, |fr| {
        fr.path.clone()
    }).flatten();

    match path {
        Some(p) => host.vars.var_from_str(&p),
        None => PP_Var::undefined(),
    }
}

unsafe extern "C" fn get_parent(file_ref: PP_Resource) -> PP_Resource {
    tracing::debug!("PPB_FileRef::GetParent({})", file_ref);
    let Some(host) = HOST.get() else { return 0 };

    let parent_path = host.resources.with_downcast::<FileRefResource, _>(file_ref, |fr| {
        fr.path.as_deref().and_then(|p| {
            Path::new(p).parent().map(|pp| pp.to_string_lossy().to_string())
        })
    }).flatten();

    let instance = host.resources.get_instance(file_ref).unwrap_or(0);

    match parent_path {
        Some(p) if !p.is_empty() => {
            let res = FileRefResource {
                file_type: FileRefType::Name,
                path: Some(p),
                fd: None,
            };
            host.resources.insert(instance, Box::new(res))
        }
        _ => 0,
    }
}

unsafe extern "C" fn make_directory_1_2(
    _directory_ref: PP_Resource,
    _make_directory_flags: i32,
    _callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("PPB_FileRef::MakeDirectory(1.2) - not supported");
    PP_ERROR_FAILED
}

unsafe extern "C" fn make_directory_1_1(
    _directory_ref: PP_Resource,
    _make_ancestors: PP_Bool,
    _callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("PPB_FileRef::MakeDirectory(1.1) - not supported");
    PP_ERROR_FAILED
}

unsafe extern "C" fn touch(
    _file_ref: PP_Resource,
    _last_access_time: PP_Time,
    _last_modified_time: PP_Time,
    _callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("PPB_FileRef::Touch - not supported");
    PP_ERROR_FAILED
}

unsafe extern "C" fn delete(
    _file_ref: PP_Resource,
    _callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("PPB_FileRef::Delete - not supported");
    PP_ERROR_FAILED
}

unsafe extern "C" fn rename(
    _file_ref: PP_Resource,
    _new_file_ref: PP_Resource,
    _callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("PPB_FileRef::Rename - not supported");
    PP_ERROR_FAILED
}

unsafe extern "C" fn query(
    file_ref: PP_Resource,
    info: *mut PP_FileInfo,
    _callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("PPB_FileRef::Query({})", file_ref);
    let Some(host) = HOST.get() else { return PP_ERROR_FAILED };

    if info.is_null() {
        return PP_ERROR_BADARGUMENT;
    }

    let path = host.resources.with_downcast::<FileRefResource, _>(file_ref, |fr| {
        fr.path.clone()
    }).flatten();

    let Some(path) = path else {
        return PP_ERROR_BADRESOURCE;
    };

    if !crate::filesystem::is_file_path_allowed(&path) {
        tracing::warn!("PPB_FileRef::Query: path '{}' blocked by file whitelist", path);
        return PP_ERROR_NOACCESS;
    }

    let fs = crate::filesystem::get_filesystem();
    let fi = match fs.query_file(&path) {
        Ok(fi) => fi,
        Err(e) => return e,
    };

    let file_type = if fi.is_dir {
        PP_FILETYPE_DIRECTORY
    } else if fi.is_file {
        PP_FILETYPE_REGULAR
    } else {
        PP_FILETYPE_OTHER
    };

    unsafe {
        *info = PP_FileInfo {
            size: fi.size,
            type_: file_type,
            system_type: PP_FILESYSTEMTYPE_EXTERNAL,
            creation_time: fi.creation_time,
            last_access_time: fi.last_access_time,
            last_modified_time: fi.last_modified_time,
        };
    }
    PP_OK
}

unsafe extern "C" fn read_directory_entries(
    _file_ref: PP_Resource,
    _output: PP_ArrayOutput,
    _callback: PP_CompletionCallback,
) -> i32 {
    tracing::debug!("PPB_FileRef::ReadDirectoryEntries - not supported");
    PP_ERROR_FAILED
}
