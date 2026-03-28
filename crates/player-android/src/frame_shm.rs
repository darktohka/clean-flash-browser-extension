//! Shared memory framebuffer via mmap'd file.
//!
//! The host creates a file at a well-known path inside the PRoot chroot,
//! mmaps it, and writes pixel data directly.  The Android app mmaps the
//! same file on the real filesystem (via the rootfs path) and feeds the
//! direct `ByteBuffer` straight to `glTexSubImage2D` — eliminating all
//! socket I/O for pixel data.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;

/// Well-known guest path for the shared framebuffer file.
pub const SHM_GUEST_PATH: &str = "/tmp/flash/framebuffer.raw";

/// A shared framebuffer backed by a memory-mapped file.
pub struct FrameShm {
    ptr: *mut u8,
    size: usize,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    _file: File,
}

// Safety: the mmap is process-wide shared memory; we synchronize access
// through the main-loop structure (write in on_flush, read in send_dirty_frame).
unsafe impl Send for FrameShm {}
unsafe impl Sync for FrameShm {}

impl FrameShm {
    /// Create a new shared framebuffer file at `path` with the given dimensions.
    pub fn create(path: &Path, width: u32, height: u32) -> io::Result<Self> {
        let stride = width * 4;
        let size = (stride * height) as usize;
        if size == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "zero-size framebuffer"));
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        file.set_len(size as u64)?;

        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }

        // Zero-fill so the file pages are faulted in.
        unsafe { std::ptr::write_bytes(ptr as *mut u8, 0, size) };

        tracing::info!(
            "FrameShm: created {}x{} ({} bytes) at {:?}",
            width, height, size, path
        );

        Ok(Self {
            ptr: ptr as *mut u8,
            size,
            width,
            height,
            stride,
            _file: file,
        })
    }

    /// Copy a rectangular dirty region from `src` (the full SharedFrameBuffer
    /// pixel array with stride `src_stride`) into the mmap'd region.
    pub fn write_region(
        &self,
        src: &[u8],
        src_stride: u32,
        dx: u32,
        dy: u32,
        dw: u32,
        dh: u32,
    ) {
        let dst_stride = self.stride as usize;
        let src_stride = src_stride as usize;
        let bytes_per_row = (dw * 4) as usize;

        for row in 0..dh {
            let y = (dy + row) as usize;
            let src_off = y * src_stride + (dx as usize) * 4;
            let dst_off = y * dst_stride + (dx as usize) * 4;

            if src_off + bytes_per_row <= src.len() && dst_off + bytes_per_row <= self.size {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        src.as_ptr().add(src_off),
                        self.ptr.add(dst_off),
                        bytes_per_row,
                    );
                }
            }
        }
    }

    /// Resize the shared framebuffer (unmap, truncate, remap).
    pub fn resize(&mut self, width: u32, height: u32) -> io::Result<()> {
        unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.size) };

        let stride = width * 4;
        let size = (stride * height) as usize;
        if size == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "zero-size framebuffer"));
        }

        self._file.set_len(size as u64)?;

        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                self._file.as_raw_fd(),
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }

        unsafe { std::ptr::write_bytes(ptr as *mut u8, 0, size) };

        self.ptr = ptr as *mut u8;
        self.size = size;
        self.width = width;
        self.height = height;
        self.stride = stride;

        tracing::info!("FrameShm: resized to {}x{} ({} bytes)", width, height, size);
        Ok(())
    }
}

impl Drop for FrameShm {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.size) };
    }
}
