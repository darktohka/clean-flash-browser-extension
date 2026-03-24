//! Filesystem providers for Flash file storage (LSOs, settings, temp files).
//!
//! Provides a [`FlashFileSystem`] trait and three implementations:
//!
//! - **`OsFileSystem`** - persists to the real OS filesystem (default).
//!   Uses `std::fs` internally so it works on both Windows and Unix.
//! - **`InMemoryFileSystem`** - stores data in-memory; nothing is written
//!   to disk and all data is lost when the process exits.
//! - **`StubFileSystem`** - every operation returns an error.  Useful for
//!   headless / CI builds where Flash file I/O is irrelevant.
//!
//! # Selecting a provider
//!
//! The default provider is chosen at compile time via crate features and
//! OS target:
//!
//! | Feature       | Effect                                    |
//! |---------------|-------------------------------------------|
//! | `fs-os`       | Use OS filesystem (default, all platforms) |
//! | `fs-memory`   | Use in-memory filesystem                  |
//! | `fs-stub`     | Use stub filesystem (all ops fail)        |
//!
//! If none of these features is enabled the default is `fs-os`.
//!
//! At runtime you can override the provider once before the first
//! filesystem call by using [`set_filesystem`].

use ppapi_sys::*;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Public data types
// ---------------------------------------------------------------------------

/// Metadata returned by file query / stat operations.
#[derive(Debug, Clone)]
pub struct FlashFileInfo {
    pub size: i64,
    pub is_file: bool,
    pub is_dir: bool,
    pub creation_time: f64,
    pub last_access_time: f64,
    pub last_modified_time: f64,
}

/// A single directory entry.
#[derive(Debug, Clone)]
pub struct FlashDirEntry {
    pub name: String,
    pub is_dir: bool,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Abstraction over the filesystem that Flash uses for Local Shared Objects,
/// settings files, and temporary files.
///
/// All paths handed to the trait methods are *already resolved* absolute
/// paths (after the host has mapped the Flash-relative path and checked for
/// directory traversal).  Implementations do **not** need to do any path
/// sanitization.
pub trait FlashFileSystem: Send + Sync {
    /// Open (or create) a file.  `mode` is a bitmask of `PP_FILEOPENFLAG_*`
    /// constants.  Returns a `PP_FileHandle` on success.
    fn open_file(&self, path: &str, mode: i32) -> Result<PP_FileHandle, i32>;

    /// Rename / move a file or directory.
    fn rename_file(&self, from: &str, to: &str) -> Result<(), i32>;

    /// Delete a file or directory.  If `recursive` is true, remove a
    /// directory and all its contents.
    fn delete_file_or_dir(&self, path: &str, recursive: bool) -> Result<(), i32>;

    /// Create a directory (and all missing parents).
    fn create_dir(&self, path: &str) -> Result<(), i32>;

    /// Query metadata for a path.
    fn query_file(&self, path: &str) -> Result<FlashFileInfo, i32>;

    /// List the immediate children of a directory.
    fn read_dir(&self, path: &str) -> Result<Vec<FlashDirEntry>, i32>;

    /// Create a temporary file and return its handle.
    fn create_temp_file(&self, dir: &str) -> Result<PP_FileHandle, i32>;

    /// Close a previously returned file handle.
    fn close_handle(&self, handle: PP_FileHandle);

    /// Query metadata for a file identified by its handle (fstat).
    fn fstat_handle(&self, handle: PP_FileHandle) -> Result<FlashFileInfo, i32>;
}

// ---------------------------------------------------------------------------
// Global provider
// ---------------------------------------------------------------------------

static FILESYSTEM: OnceLock<Box<dyn FlashFileSystem>> = OnceLock::new();

/// Override the filesystem provider.  Must be called before the first
/// filesystem operation; subsequent calls are silently ignored.
pub fn set_filesystem(fs: Box<dyn FlashFileSystem>) {
    let _ = FILESYSTEM.set(fs);
}

/// Obtain a reference to the active filesystem provider.
pub fn get_filesystem() -> &'static dyn FlashFileSystem {
    FILESYSTEM.get_or_init(|| default_filesystem()).as_ref()
}

fn default_filesystem() -> Box<dyn FlashFileSystem> {
    #[cfg(feature = "fs-stub")]
    {
        return Box::new(StubFileSystem);
    }
    #[cfg(feature = "fs-memory")]
    {
        return Box::new(InMemoryFileSystem::new());
    }
    #[cfg(not(any(feature = "fs-stub", feature = "fs-memory")))]
    {
        return Box::new(OsFileSystem);
    }
}

// ---------------------------------------------------------------------------
// File whitelist manager
// ---------------------------------------------------------------------------

/// Check if a file path is allowed by the file whitelist settings.
///
/// When file whitelisting is disabled, all paths are allowed.
/// When enabled, a path is allowed if:
/// - It matches an entry in the whitelisted files list exactly, OR
/// - It is under any of the whitelisted folders (prefix match on
///   canonicalized path components).
pub fn is_file_path_allowed(path: &str) -> bool {
    let settings = crate::HOST
        .get()
        .and_then(|h| h.get_settings_provider())
        .map(|sp| sp.get_settings());
    let Some(settings) = settings else { return true };

    if !settings.file_whitelist_enabled {
        return true;
    }

    // Normalize the path for comparison
    let normalized = normalize_path(path);

    // Check exact file match
    if settings.whitelisted_files.iter().any(|f| normalize_path(f) == normalized) {
        return true;
    }

    // Check folder prefix match
    for folder in &settings.whitelisted_folders {
        let folder_normalized = normalize_path(folder);
        let prefix = if folder_normalized.ends_with('/') {
            folder_normalized.clone()
        } else {
            format!("{}/", folder_normalized)
        };
        if normalized.starts_with(&prefix) || normalized == folder_normalized {
            return true;
        }
    }

    false
}

/// Normalize a path for comparison: resolve `.` and `..`, normalize
/// separators.
fn normalize_path(path: &str) -> String {
    use std::path::Path;
    let p = Path::new(path);
    // Try to canonicalize (resolve symlinks etc.), fall back to lexical
    // normalization if the path doesn't exist yet.
    match p.canonicalize() {
        Ok(canonical) => canonical.to_string_lossy().to_string(),
        Err(_) => {
            // Lexical normalization: just clean up the path
            let mut components = Vec::new();
            for comp in p.components() {
                match comp {
                    std::path::Component::ParentDir => { components.pop(); }
                    std::path::Component::CurDir => {}
                    _ => components.push(comp),
                }
            }
            let result: std::path::PathBuf = components.iter().collect();
            result.to_string_lossy().to_string()
        }
    }
}

// ===========================================================================
// OS Filesystem
// ===========================================================================

/// Real filesystem backed by the OS.  Works on Windows, Linux, and macOS.
pub struct OsFileSystem;

impl OsFileSystem {
    fn io_err_to_pp(e: &std::io::Error) -> i32 {
        match e.kind() {
            std::io::ErrorKind::NotFound => PP_ERROR_FILENOTFOUND,
            std::io::ErrorKind::PermissionDenied => PP_ERROR_NOACCESS,
            std::io::ErrorKind::AlreadyExists => PP_ERROR_FILEEXISTS,
            _ => PP_ERROR_FAILED,
        }
    }

    fn metadata_to_info(meta: &std::fs::Metadata) -> FlashFileInfo {
        let time = |t: std::io::Result<std::time::SystemTime>| -> f64 {
            t.ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0)
        };
        FlashFileInfo {
            size: meta.len() as i64,
            is_file: meta.is_file(),
            is_dir: meta.is_dir(),
            creation_time: time(meta.created()),
            last_access_time: time(meta.accessed()),
            last_modified_time: time(meta.modified()),
        }
    }
}

/// Convert a `std::fs::File` into a `PP_FileHandle` (i32).
#[cfg(unix)]
fn file_to_handle(file: std::fs::File) -> Result<PP_FileHandle, i32> {
    use std::os::unix::io::IntoRawFd;
    Ok(file.into_raw_fd())
}

#[cfg(windows)]
fn file_to_handle(file: std::fs::File) -> Result<PP_FileHandle, i32> {
    use std::os::windows::io::IntoRawHandle;
    Ok(file.into_raw_handle() as PP_FileHandle)
}

/// Close a raw `PP_FileHandle`.
#[cfg(unix)]
fn close_raw_handle(handle: PP_FileHandle) {
    unsafe { libc::close(handle) };
}

#[cfg(windows)]
fn close_raw_handle(handle: PP_FileHandle) {
    unsafe { CloseHandle(handle) };
}

/// Stat a file by its `PP_FileHandle`.
#[cfg(unix)]
fn fstat_raw(handle: PP_FileHandle) -> Result<std::fs::Metadata, i32> {
    use std::os::unix::io::FromRawFd;
    // Duplicate the fd so we don't consume it.
    let dup_fd = unsafe { libc::dup(handle) };
    if dup_fd < 0 {
        return Err(PP_ERROR_FAILED);
    }
    let file = unsafe { std::fs::File::from_raw_fd(dup_fd) };
    file.metadata().map_err(|e| OsFileSystem::io_err_to_pp(&e))
}

#[cfg(windows)]
fn fstat_raw(handle: PP_FileHandle) -> Result<std::fs::Metadata, i32> {
    use std::os::windows::io::FromRawHandle;
    // Duplicate so we don't take ownership.
    let mut dup_handle: isize = 0;
    let current_process = unsafe { GetCurrentProcess() };
    let ok = unsafe {
        DuplicateHandle(
            current_process,
            handle,
            current_process,
            &mut dup_handle,
            0,
            0, // bInheritHandle = FALSE
            2, // DUPLICATE_SAME_ACCESS
        )
    };
    if ok == 0 {
        return Err(PP_ERROR_FAILED);
    }
    let file = unsafe { std::fs::File::from_raw_handle(dup_handle as *mut std::ffi::c_void) };
    file.metadata().map_err(|e| OsFileSystem::io_err_to_pp(&e))
}

// Windows kernel32 FFI (only compiled on Windows).

#[cfg(windows)]
extern "system" {
    fn CloseHandle(hObject: isize) -> i32;
    fn GetCurrentProcess() -> isize;
    fn DuplicateHandle(
        hSourceProcessHandle: isize,
        hSourceHandle: isize,
        hTargetProcessHandle: isize,
        lpTargetHandle: *mut isize,
        dwDesiredAccess: u32,
        bInheritHandle: i32,
        dwOptions: u32,
    ) -> i32;
}

impl FlashFileSystem for OsFileSystem {
    fn open_file(&self, path: &str, mode: i32) -> Result<PP_FileHandle, i32> {
        if !is_file_path_allowed(path) {
            tracing::warn!("OsFileSystem::open_file: path '{}' blocked by file whitelist", path);
            return Err(PP_ERROR_NOACCESS);
        }

        let p = std::path::Path::new(path);

        let mut opts = std::fs::OpenOptions::new();
        let read = mode & PP_FILEOPENFLAG_READ != 0;
        let write = mode & PP_FILEOPENFLAG_WRITE != 0;
        if read { opts.read(true); }
        if write { opts.write(true); }
        if mode & PP_FILEOPENFLAG_CREATE != 0 { opts.create(true); }
        if mode & PP_FILEOPENFLAG_TRUNCATE != 0 { opts.truncate(true); }
        if mode & PP_FILEOPENFLAG_EXCLUSIVE != 0 { opts.create_new(true); }
        if mode & PP_FILEOPENFLAG_APPEND != 0 { opts.append(true); }

        // Ensure parent directory exists when creating.
        if mode & PP_FILEOPENFLAG_CREATE != 0 {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
        }

        let file = opts.open(p).map_err(|e| Self::io_err_to_pp(&e))?;
        file_to_handle(file)
    }

    fn rename_file(&self, from: &str, to: &str) -> Result<(), i32> {
        std::fs::rename(from, to).map_err(|e| Self::io_err_to_pp(&e))
    }

    fn delete_file_or_dir(&self, path: &str, recursive: bool) -> Result<(), i32> {
        let p = std::path::Path::new(path);
        let result = if p.is_dir() {
            if recursive {
                std::fs::remove_dir_all(p)
            } else {
                std::fs::remove_dir(p)
            }
        } else {
            std::fs::remove_file(p)
        };
        result.map_err(|e| Self::io_err_to_pp(&e))
    }

    fn create_dir(&self, path: &str) -> Result<(), i32> {
        let p = std::path::Path::new(path);
        if p.is_dir() {
            return Ok(());
        }
        std::fs::create_dir_all(p).map_err(|e| Self::io_err_to_pp(&e))
    }

    fn query_file(&self, path: &str) -> Result<FlashFileInfo, i32> {
        if !is_file_path_allowed(path) {
            tracing::warn!("OsFileSystem::query_file: path '{}' blocked by file whitelist", path);
            return Err(PP_ERROR_NOACCESS);
        }
        let meta = std::fs::metadata(path).map_err(|e| Self::io_err_to_pp(&e))?;
        Ok(Self::metadata_to_info(&meta))
    }

    fn read_dir(&self, path: &str) -> Result<Vec<FlashDirEntry>, i32> {
        if !is_file_path_allowed(path) {
            tracing::warn!("OsFileSystem::read_dir: path '{}' blocked by file whitelist", path);
            return Err(PP_ERROR_NOACCESS);
        }
        let rd = std::fs::read_dir(path).map_err(|e| Self::io_err_to_pp(&e))?;
        let entries = rd
            .filter_map(|e| e.ok())
            .map(|e| FlashDirEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir: e.file_type().map(|t| t.is_dir()).unwrap_or(false),
            })
            .collect();
        Ok(entries)
    }

    fn create_temp_file(&self, dir: &str) -> Result<PP_FileHandle, i32> {
        // Create a unique temp file inside `dir`.
        let _ = std::fs::create_dir_all(dir);
        // Use a simple counter + pid for uniqueness.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path = std::path::Path::new(dir).join(format!("flash_tmp_{pid}_{n}"));

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| Self::io_err_to_pp(&e))?;

        // Delete-on-close semantics: unlink immediately on Unix so the
        // file disappears from the directory when the fd is closed.
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&path);
        }
        // On Windows the file stays until close; that's acceptable for temp files.

        file_to_handle(file)
    }

    fn close_handle(&self, handle: PP_FileHandle) {
        close_raw_handle(handle);
    }

    fn fstat_handle(&self, handle: PP_FileHandle) -> Result<FlashFileInfo, i32> {
        let meta = fstat_raw(handle)?;
        Ok(Self::metadata_to_info(&meta))
    }
}

// ===========================================================================
// In-Memory Filesystem
// ===========================================================================

use std::collections::HashMap;
use std::sync::atomic::{Ordering};
use parking_lot::Mutex;

#[cfg(windows)]
use std::sync::atomic::AtomicIsize;
#[cfg(windows)]
type PpAtomicFileHandle = AtomicIsize;

#[cfg(not(windows))]
use std::sync::atomic::AtomicI32;
#[cfg(not(windows))]
type PpAtomicFileHandle = AtomicI32;

/// In-memory filesystem.  All data lives in process memory and is lost
/// when the process exits.
pub struct InMemoryFileSystem {
    inner: Mutex<MemFsInner>,
}

#[derive(Debug, Clone)]
enum MemFsNode {
    File(Vec<u8>),
    Dir,
}

struct MemFsInner {
    nodes: HashMap<String, MemFsNode>,
    /// Maps synthetic fd → path.
    open_handles: HashMap<PP_FileHandle, String>,
    next_fd: PpAtomicFileHandle,
}

impl std::fmt::Debug for MemFsInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemFsInner")
            .field("nodes", &self.nodes.len())
            .field("open_handles", &self.open_handles.len())
            .finish()
    }
}

impl InMemoryFileSystem {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MemFsInner {
                nodes: HashMap::new(),
                open_handles: HashMap::new(),
                next_fd: PpAtomicFileHandle::new(1000), // Start high to avoid collisions.
            }),
        }
    }

    fn normalize(path: &str) -> String {
        path.replace('\\', "/")
            .trim_end_matches('/')
            .to_string()
    }
}

impl FlashFileSystem for InMemoryFileSystem {
    fn open_file(&self, path: &str, mode: i32) -> Result<PP_FileHandle, i32> {
        let key = Self::normalize(path);
        let mut inner = self.inner.lock();

        let exists = inner.nodes.contains_key(&key);
        let create = mode & PP_FILEOPENFLAG_CREATE != 0;
        let exclusive = mode & PP_FILEOPENFLAG_EXCLUSIVE != 0;
        let truncate = mode & PP_FILEOPENFLAG_TRUNCATE != 0;

        if exclusive && exists {
            return Err(PP_ERROR_FILEEXISTS);
        }
        if !exists && !create {
            return Err(PP_ERROR_FILENOTFOUND);
        }

        // Ensure parent dirs exist.
        if create {
            if let Some(idx) = key.rfind('/') {
                let parent = &key[..idx];
                if !parent.is_empty() && !inner.nodes.contains_key(parent) {
                    // Insert parent chain.
                    let mut cur = String::new();
                    for part in parent.split('/') {
                        if !cur.is_empty() || parent.starts_with('/') {
                            cur.push('/');
                        }
                        cur.push_str(part);
                        if !cur.is_empty() {
                            inner.nodes.entry(cur.clone()).or_insert(MemFsNode::Dir);
                        }
                    }
                }
            }
        }

        if !exists || truncate {
            inner.nodes.insert(key.clone(), MemFsNode::File(Vec::new()));
        }

        let fd = inner.next_fd.fetch_add(1, Ordering::Relaxed);
        inner.open_handles.insert(fd, key);
        Ok(fd)
    }

    fn rename_file(&self, from: &str, to: &str) -> Result<(), i32> {
        let from_key = Self::normalize(from);
        let to_key = Self::normalize(to);
        let mut inner = self.inner.lock();
        let node = inner.nodes.remove(&from_key).ok_or(PP_ERROR_FILENOTFOUND)?;
        inner.nodes.insert(to_key, node);
        Ok(())
    }

    fn delete_file_or_dir(&self, path: &str, recursive: bool) -> Result<(), i32> {
        let key = Self::normalize(path);
        let mut inner = self.inner.lock();
        if !inner.nodes.contains_key(&key) {
            return Err(PP_ERROR_FILENOTFOUND);
        }
        if recursive {
            let prefix = format!("{}/", key);
            inner.nodes.retain(|k, _| !k.starts_with(&prefix));
        }
        inner.nodes.remove(&key);
        Ok(())
    }

    fn create_dir(&self, path: &str) -> Result<(), i32> {
        let key = Self::normalize(path);
        let mut inner = self.inner.lock();
        inner.nodes.entry(key).or_insert(MemFsNode::Dir);
        Ok(())
    }

    fn query_file(&self, path: &str) -> Result<FlashFileInfo, i32> {
        let key = Self::normalize(path);
        let inner = self.inner.lock();
        match inner.nodes.get(&key) {
            Some(MemFsNode::File(data)) => Ok(FlashFileInfo {
                size: data.len() as i64,
                is_file: true,
                is_dir: false,
                creation_time: 0.0,
                last_access_time: 0.0,
                last_modified_time: 0.0,
            }),
            Some(MemFsNode::Dir) => Ok(FlashFileInfo {
                size: 0,
                is_file: false,
                is_dir: true,
                creation_time: 0.0,
                last_access_time: 0.0,
                last_modified_time: 0.0,
            }),
            None => Err(PP_ERROR_FILENOTFOUND),
        }
    }

    fn read_dir(&self, path: &str) -> Result<Vec<FlashDirEntry>, i32> {
        let key = Self::normalize(path);
        let inner = self.inner.lock();
        if !matches!(inner.nodes.get(&key), Some(MemFsNode::Dir)) {
            // Root ("/") may not be explicitly stored.
            if key.is_empty() || key == "/" {
                // Allow listing root even if not stored.
            } else {
                return Err(PP_ERROR_FILENOTFOUND);
            }
        }
        let prefix = if key.is_empty() || key == "/" {
            String::new()
        } else {
            format!("{}/", key)
        };
        let mut entries = Vec::new();
        for (k, node) in &inner.nodes {
            if let Some(rest) = k.strip_prefix(&prefix) {
                if !rest.contains('/') && !rest.is_empty() {
                    entries.push(FlashDirEntry {
                        name: rest.to_string(),
                        is_dir: matches!(node, MemFsNode::Dir),
                    });
                }
            }
        }
        Ok(entries)
    }

    fn create_temp_file(&self, _dir: &str) -> Result<PP_FileHandle, i32> {
        let mut inner = self.inner.lock();
        let fd = inner.next_fd.fetch_add(1, Ordering::Relaxed);
        let key = format!("/__tmp_{}", fd);
        inner.nodes.insert(key.clone(), MemFsNode::File(Vec::new()));
        inner.open_handles.insert(fd, key);
        Ok(fd)
    }

    fn close_handle(&self, handle: PP_FileHandle) {
        let mut inner = self.inner.lock();
        inner.open_handles.remove(&handle);
    }

    fn fstat_handle(&self, handle: PP_FileHandle) -> Result<FlashFileInfo, i32> {
        let inner = self.inner.lock();
        let path = inner.open_handles.get(&handle).ok_or(PP_ERROR_BADRESOURCE)?;
        match inner.nodes.get(path) {
            Some(MemFsNode::File(data)) => Ok(FlashFileInfo {
                size: data.len() as i64,
                is_file: true,
                is_dir: false,
                creation_time: 0.0,
                last_access_time: 0.0,
                last_modified_time: 0.0,
            }),
            Some(MemFsNode::Dir) => Ok(FlashFileInfo {
                size: 0,
                is_file: false,
                is_dir: true,
                creation_time: 0.0,
                last_access_time: 0.0,
                last_modified_time: 0.0,
            }),
            None => Err(PP_ERROR_FILENOTFOUND),
        }
    }
}

// ===========================================================================
// Stub Filesystem
// ===========================================================================

/// Stub filesystem that rejects every operation.
pub struct StubFileSystem;

impl FlashFileSystem for StubFileSystem {
    fn open_file(&self, _path: &str, _mode: i32) -> Result<PP_FileHandle, i32> {
        Err(PP_ERROR_FAILED)
    }
    fn rename_file(&self, _from: &str, _to: &str) -> Result<(), i32> {
        Err(PP_ERROR_FAILED)
    }
    fn delete_file_or_dir(&self, _path: &str, _recursive: bool) -> Result<(), i32> {
        Err(PP_ERROR_FAILED)
    }
    fn create_dir(&self, _path: &str) -> Result<(), i32> {
        Err(PP_ERROR_FAILED)
    }
    fn query_file(&self, _path: &str) -> Result<FlashFileInfo, i32> {
        Err(PP_ERROR_FAILED)
    }
    fn read_dir(&self, _path: &str) -> Result<Vec<FlashDirEntry>, i32> {
        Err(PP_ERROR_FAILED)
    }
    fn create_temp_file(&self, _dir: &str) -> Result<PP_FileHandle, i32> {
        Err(PP_ERROR_FAILED)
    }
    fn close_handle(&self, _handle: PP_FileHandle) {
        // No-op.
    }
    fn fstat_handle(&self, _handle: PP_FileHandle) -> Result<FlashFileInfo, i32> {
        Err(PP_ERROR_FAILED)
    }
}
