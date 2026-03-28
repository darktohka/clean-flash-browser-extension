//! Stub registrations for OpenGL ES 2.0 Chromium extension interfaces.
//!
//! When the real `opengles2` module is not compiled (no `gl-glow` feature),
//! Flash still queries these interfaces during PPP_InitializeModule.
//! Providing no-op vtables prevents init failure.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::c_void;
use std::ptr;

// ---------------------------------------------------------------------------
// PPB_OpenGLES2ChromiumMapSub stubs
// ---------------------------------------------------------------------------

unsafe extern "C" fn map_buffer_sub_data(
    _context: PP_Resource, _target: GLuint, _offset: GLintptr,
    _size: GLsizeiptr, _access: GLenum,
) -> *mut c_void {
    ptr::null_mut()
}

unsafe extern "C" fn unmap_buffer_sub_data(_context: PP_Resource, _mem: *const c_void) {}

unsafe extern "C" fn map_tex_sub_image_2d(
    _context: PP_Resource, _target: GLenum, _level: GLint,
    _xoffset: GLint, _yoffset: GLint, _width: GLsizei, _height: GLsizei,
    _format: GLenum, _type: GLenum, _access: GLenum,
) -> *mut c_void {
    ptr::null_mut()
}

unsafe extern "C" fn unmap_tex_sub_image_2d(_context: PP_Resource, _mem: *const c_void) {}

static CHROMIUM_MAP_SUB_VTABLE: PPB_OpenGLES2ChromiumMapSub_1_0 =
    PPB_OpenGLES2ChromiumMapSub_1_0 {
        MapBufferSubDataCHROMIUM: Some(map_buffer_sub_data),
        UnmapBufferSubDataCHROMIUM: Some(unmap_buffer_sub_data),
        MapTexSubImage2DCHROMIUM: Some(map_tex_sub_image_2d),
        UnmapTexSubImage2DCHROMIUM: Some(unmap_tex_sub_image_2d),
    };

// ---------------------------------------------------------------------------
// PPB_OpenGLES2ChromiumEnableFeature stub
// ---------------------------------------------------------------------------

unsafe extern "C" fn enable_feature_chromium(
    _context: PP_Resource, _feature: *const std::ffi::c_char,
) -> GLboolean {
    0 // GL_FALSE
}

static CHROMIUM_ENABLE_VTABLE: PPB_OpenGLES2ChromiumEnableFeature_1_0 =
    PPB_OpenGLES2ChromiumEnableFeature_1_0 {
        EnableFeatureCHROMIUM: Some(enable_feature_chromium),
    };

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// PPB_OpenGLES2 stub — all function pointers are None (null).
// Flash stores the vtable pointer during init but won't call GL functions
// until a Graphics3D context is actually bound.
// ---------------------------------------------------------------------------

static GLES2_VTABLE: PPB_OpenGLES2_1_0 = unsafe { std::mem::zeroed() };

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_OPENGLES2_INTERFACE_1_0, &GLES2_VTABLE);
        registry.register(
            PPB_OPENGLES2_CHROMIUMMAPSUB_INTERFACE_1_0,
            &CHROMIUM_MAP_SUB_VTABLE,
        );
        // Dev variant and GLESChromiumTextureMapping alias use the same vtable
        registry.register(
            "PPB_OpenGLES2ChromiumMapSub(Dev);1.0\0",
            &CHROMIUM_MAP_SUB_VTABLE,
        );
        registry.register(
            "PPB_GLESChromiumTextureMapping(Dev);0.1\0",
            &CHROMIUM_MAP_SUB_VTABLE,
        );
        registry.register(
            PPB_OPENGLES2_CHROMIUMENABLEFEATURE_INTERFACE_1_0,
            &CHROMIUM_ENABLE_VTABLE,
        );
    }
}
