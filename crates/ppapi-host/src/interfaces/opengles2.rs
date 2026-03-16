//! PPB_OpenGLES2;1.0 and extension interfaces implementation.
//!
//! These provide the GL ES 2.0 function vtables that Flash queries at init.
//! When a real EGL/GLES2 context is available (via `gl_context`), all functions
//! dispatch to the real GPU driver via `glow`.  Otherwise they fall back to
//! safe defaults.

use crate::gl_context;
use crate::interface_registry::InterfaceRegistry;
use glow::HasContext;
use ppapi_sys::*;
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::num::NonZeroU32;
use std::ptr;
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Raw GL function loaders for functions glow doesn't expose or wraps unsafely
// ---------------------------------------------------------------------------

type GlGetIntegervFn = unsafe extern "system" fn(u32, *mut i32);
type GlGetFloatvFn = unsafe extern "system" fn(u32, *mut f32);
type GlGetBooleanvFn = unsafe extern "system" fn(u32, *mut u8);

static RAW_GET_INTEGERV: OnceLock<Option<GlGetIntegervFn>> = OnceLock::new();
static RAW_GET_FLOATV: OnceLock<Option<GlGetFloatvFn>> = OnceLock::new();
static RAW_GET_BOOLEANV: OnceLock<Option<GlGetBooleanvFn>> = OnceLock::new();

unsafe fn load_raw_gl<T>(name: &std::ffi::CStr) -> Option<T> {
    let p = gl_context::gl_proc_address(name);
    if p.is_null() { None } else { Some(std::mem::transmute_copy(&p)) }
}

/// Compute byte size per pixel for a given GL format + type.
fn pixel_byte_size(fmt: GLenum, type_: GLenum) -> usize {
    // Packed types are one unit per pixel regardless of channels
    match type_ {
        glow::UNSIGNED_SHORT_5_6_5
        | glow::UNSIGNED_SHORT_4_4_4_4
        | glow::UNSIGNED_SHORT_5_5_5_1 => return 2,
        _ => {}
    }
    let channels = match fmt {
        glow::ALPHA | glow::LUMINANCE | glow::DEPTH_COMPONENT => 1,
        glow::LUMINANCE_ALPHA => 2,
        glow::RGB => 3,
        _ /* RGBA and others */ => 4,
    };
    let type_size = match type_ {
        glow::UNSIGNED_BYTE => 1,
        glow::FLOAT => 4,
        glow::UNSIGNED_SHORT => 2,
        glow::UNSIGNED_INT => 4,
        _ => 1,
    };
    channels * type_size
}

// ---------------------------------------------------------------------------
// Helper: ensure context is current, then get glow context
// ---------------------------------------------------------------------------

#[inline]
fn with_gl(ctx: PP_Resource) -> Option<&'static glow::Context> {
    if gl_context::ensure_context_current(ctx) {
        gl_context::gl_functions()
    } else {
        None
    }
}

/// Convert a raw GLuint framebuffer id to glow's Option<Framebuffer>.
#[inline]
fn to_fbo(id: GLuint) -> Option<glow::Framebuffer> {
    NonZeroU32::new(id).map(glow::NativeFramebuffer)
}

/// Convert a raw GLuint renderbuffer id to glow's Option<Renderbuffer>.
#[inline]
fn to_rbo(id: GLuint) -> Option<glow::Renderbuffer> {
    NonZeroU32::new(id).map(glow::NativeRenderbuffer)
}

/// Convert a raw GLuint buffer id to glow's Option<Buffer>.
#[inline]
fn to_buf(id: GLuint) -> Option<glow::Buffer> {
    NonZeroU32::new(id).map(glow::NativeBuffer)
}

/// Convert a raw GLuint texture id to glow's Option<Texture>.
#[inline]
fn to_tex(id: GLuint) -> Option<glow::Texture> {
    NonZeroU32::new(id).map(glow::NativeTexture)
}

/// Convert a raw GLuint to a glow Shader.
#[inline]
fn to_shader(id: GLuint) -> glow::Shader {
    glow::NativeShader(NonZeroU32::new(id).unwrap_or(NonZeroU32::new(u32::MAX).unwrap()))
}

/// Convert a raw GLuint to a glow Program.
#[inline]
fn to_program(id: GLuint) -> glow::Program {
    glow::NativeProgram(NonZeroU32::new(id).unwrap_or(NonZeroU32::new(u32::MAX).unwrap()))
}

/// Convert a raw GLint uniform location to glow's UniformLocation.
#[inline]
fn to_uniform(loc: GLint) -> Option<glow::UniformLocation> {
    if loc < 0 { None } else { Some(glow::NativeUniformLocation(loc as u32)) }
}

// ---------------------------------------------------------------------------
// PPB_OpenGLES2;1.0 - GL ES 2.0 functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn active_texture(ctx: PP_Resource, texture: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.active_texture(texture); }
}
unsafe extern "C" fn attach_shader(ctx: PP_Resource, program: GLuint, shader: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.attach_shader(to_program(program), to_shader(shader)); }
}
unsafe extern "C" fn bind_attrib_location(ctx: PP_Resource, program: GLuint, index: GLuint, name: *const c_char) {
    if let Some(gl) = with_gl(ctx) {
        let s = CStr::from_ptr(name).to_str().unwrap_or("");
        gl.bind_attrib_location(to_program(program), index, s);
    }
}
unsafe extern "C" fn bind_buffer(ctx: PP_Resource, target: GLenum, buffer: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.bind_buffer(target, to_buf(buffer)); }
}
unsafe extern "C" fn bind_framebuffer(ctx: PP_Resource, target: GLenum, fb: GLuint) {
    if let Some(gl) = with_gl(ctx) {
        // Redirect FBO 0 to our offscreen FBO when using FBO-based rendering.
        let actual = if fb == 0 {
            gl_context::get_offscreen_fbo(ctx)
        } else {
            to_fbo(fb)
        };
        gl.bind_framebuffer(target, actual);
    }
}
unsafe extern "C" fn bind_renderbuffer(ctx: PP_Resource, target: GLenum, rb: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.bind_renderbuffer(target, to_rbo(rb)); }
}
unsafe extern "C" fn bind_texture(ctx: PP_Resource, target: GLenum, texture: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.bind_texture(target, to_tex(texture)); }
}
unsafe extern "C" fn blend_color(ctx: PP_Resource, r: GLclampf, g: GLclampf, b: GLclampf, a: GLclampf) {
    if let Some(gl) = with_gl(ctx) { gl.blend_color(r, g, b, a); }
}
unsafe extern "C" fn blend_equation(ctx: PP_Resource, mode: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.blend_equation(mode); }
}
unsafe extern "C" fn blend_equation_separate(ctx: PP_Resource, rgb: GLenum, alpha: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.blend_equation_separate(rgb, alpha); }
}
unsafe extern "C" fn blend_func(ctx: PP_Resource, sf: GLenum, df: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.blend_func(sf, df); }
}
unsafe extern "C" fn blend_func_separate(ctx: PP_Resource, sr: GLenum, dr: GLenum, sa: GLenum, da: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.blend_func_separate(sr, dr, sa, da); }
}
unsafe extern "C" fn buffer_data(ctx: PP_Resource, target: GLenum, size: GLsizeiptr, data: *const c_void, usage: GLenum) {
    if let Some(gl) = with_gl(ctx) {
        if data.is_null() {
            // Allocate buffer without data (glBufferData with NULL pointer).
            gl.buffer_data_size(target, size as i32, usage);
        } else {
            let slice = std::slice::from_raw_parts(data as *const u8, size as usize);
            gl.buffer_data_u8_slice(target, slice, usage);
        }
    }
}
unsafe extern "C" fn buffer_sub_data(ctx: PP_Resource, target: GLenum, offset: GLintptr, size: GLsizeiptr, data: *const c_void) {
    if let Some(gl) = with_gl(ctx) {
        let slice = if data.is_null() { &[] } else {
            std::slice::from_raw_parts(data as *const u8, size as usize)
        };
        gl.buffer_sub_data_u8_slice(target, offset as i32, slice);
    }
}
unsafe extern "C" fn check_framebuffer_status(ctx: PP_Resource, target: GLenum) -> GLenum {
    with_gl(ctx).map_or(0x8CD5, |gl| gl.check_framebuffer_status(target))
}
unsafe extern "C" fn clear(ctx: PP_Resource, mask: GLbitfield) {
    if let Some(gl) = with_gl(ctx) { gl.clear(mask); }
}
unsafe extern "C" fn clear_color(ctx: PP_Resource, r: GLclampf, g: GLclampf, b: GLclampf, a: GLclampf) {
    if let Some(gl) = with_gl(ctx) { gl.clear_color(r, g, b, a); }
}
unsafe extern "C" fn clear_depthf(ctx: PP_Resource, depth: GLclampf) {
    if let Some(gl) = with_gl(ctx) { gl.clear_depth_f32(depth); }
}
unsafe extern "C" fn clear_stencil(ctx: PP_Resource, s: GLint) {
    if let Some(gl) = with_gl(ctx) { gl.clear_stencil(s); }
}
unsafe extern "C" fn color_mask(ctx: PP_Resource, r: GLboolean, g: GLboolean, b: GLboolean, a: GLboolean) {
    if let Some(gl) = with_gl(ctx) { gl.color_mask(r != 0, g != 0, b != 0, a != 0); }
}
unsafe extern "C" fn compile_shader(ctx: PP_Resource, shader: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.compile_shader(to_shader(shader)); }
}
unsafe extern "C" fn compressed_tex_image_2d(ctx: PP_Resource, target: GLenum, level: GLint, fmt: GLenum, w: GLsizei, h: GLsizei, border: GLint, size: GLsizei, data: *const c_void) {
    if let Some(gl) = with_gl(ctx) {
        let slice = if data.is_null() { &[] } else {
            std::slice::from_raw_parts(data as *const u8, size as usize)
        };
        gl.compressed_tex_image_2d(target, level, fmt as i32, w, h, border, size, slice);
    }
}
unsafe extern "C" fn compressed_tex_sub_image_2d(ctx: PP_Resource, target: GLenum, level: GLint, x: GLint, y: GLint, w: GLsizei, h: GLsizei, fmt: GLenum, size: GLsizei, data: *const c_void) {
    if let Some(gl) = with_gl(ctx) {
        if data.is_null() { return; }
        let slice = std::slice::from_raw_parts(data as *const u8, size as usize);
        gl.compressed_tex_sub_image_2d(target, level, x, y, w, h, fmt, glow::CompressedPixelUnpackData::Slice(slice));
    }
}
unsafe extern "C" fn copy_tex_image_2d(ctx: PP_Resource, target: GLenum, level: GLint, fmt: GLenum, x: GLint, y: GLint, w: GLsizei, h: GLsizei, border: GLint) {
    if let Some(gl) = with_gl(ctx) { gl.copy_tex_image_2d(target, level, fmt, x, y, w, h, border); }
}
unsafe extern "C" fn copy_tex_sub_image_2d(ctx: PP_Resource, target: GLenum, level: GLint, xoff: GLint, yoff: GLint, x: GLint, y: GLint, w: GLsizei, h: GLsizei) {
    if let Some(gl) = with_gl(ctx) { gl.copy_tex_sub_image_2d(target, level, xoff, yoff, x, y, w, h); }
}
unsafe extern "C" fn create_program(ctx: PP_Resource) -> GLuint {
    with_gl(ctx).map_or(0, |gl| gl.create_program().map_or(0, |p| p.0.get()))
}
unsafe extern "C" fn create_shader(ctx: PP_Resource, type_: GLenum) -> GLuint {
    with_gl(ctx).map_or(0, |gl| gl.create_shader(type_).map_or(0, |s| s.0.get()))
}
unsafe extern "C" fn cull_face(ctx: PP_Resource, mode: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.cull_face(mode); }
}
unsafe extern "C" fn delete_buffers(ctx: PP_Resource, n: GLsizei, bufs: *const GLuint) {
    if let Some(gl) = with_gl(ctx) {
        let ids = std::slice::from_raw_parts(bufs, n as usize);
        for &id in ids { if let Some(b) = to_buf(id) { gl.delete_buffer(b); } }
    }
}
unsafe extern "C" fn delete_framebuffers(ctx: PP_Resource, n: GLsizei, fbs: *const GLuint) {
    if let Some(gl) = with_gl(ctx) {
        let ids = std::slice::from_raw_parts(fbs, n as usize);
        for &id in ids { if let Some(f) = to_fbo(id) { gl.delete_framebuffer(f); } }
    }
}
unsafe extern "C" fn delete_program(ctx: PP_Resource, prog: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.delete_program(to_program(prog)); }
}
unsafe extern "C" fn delete_renderbuffers(ctx: PP_Resource, n: GLsizei, rbs: *const GLuint) {
    if let Some(gl) = with_gl(ctx) {
        let ids = std::slice::from_raw_parts(rbs, n as usize);
        for &id in ids { if let Some(r) = to_rbo(id) { gl.delete_renderbuffer(r); } }
    }
}
unsafe extern "C" fn delete_shader(ctx: PP_Resource, shader: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.delete_shader(to_shader(shader)); }
}
unsafe extern "C" fn delete_textures(ctx: PP_Resource, n: GLsizei, texs: *const GLuint) {
    if let Some(gl) = with_gl(ctx) {
        let ids = std::slice::from_raw_parts(texs, n as usize);
        for &id in ids { if let Some(t) = to_tex(id) { gl.delete_texture(t); } }
    }
}
unsafe extern "C" fn depth_func(ctx: PP_Resource, func: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.depth_func(func); }
}
unsafe extern "C" fn depth_mask(ctx: PP_Resource, flag: GLboolean) {
    if let Some(gl) = with_gl(ctx) { gl.depth_mask(flag != 0); }
}
unsafe extern "C" fn depth_rangef(ctx: PP_Resource, near: GLclampf, far: GLclampf) {
    if let Some(gl) = with_gl(ctx) { gl.depth_range_f32(near, far); }
}
unsafe extern "C" fn detach_shader(ctx: PP_Resource, prog: GLuint, shader: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.detach_shader(to_program(prog), to_shader(shader)); }
}
unsafe extern "C" fn disable(ctx: PP_Resource, cap: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.disable(cap); }
}
unsafe extern "C" fn disable_vertex_attrib_array(ctx: PP_Resource, idx: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.disable_vertex_attrib_array(idx); }
}
unsafe extern "C" fn draw_arrays(ctx: PP_Resource, mode: GLenum, first: GLint, count: GLsizei) {
    if let Some(gl) = with_gl(ctx) { gl.draw_arrays(mode, first, count); }
}
unsafe extern "C" fn draw_elements(ctx: PP_Resource, mode: GLenum, count: GLsizei, type_: GLenum, indices: *const c_void) {
    if let Some(gl) = with_gl(ctx) { gl.draw_elements(mode, count, type_, indices as i32); }
}
unsafe extern "C" fn enable(ctx: PP_Resource, cap: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.enable(cap); }
}
unsafe extern "C" fn enable_vertex_attrib_array(ctx: PP_Resource, idx: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.enable_vertex_attrib_array(idx); }
}
unsafe extern "C" fn finish(ctx: PP_Resource) {
    if let Some(gl) = with_gl(ctx) { gl.finish(); }
}
unsafe extern "C" fn flush(ctx: PP_Resource) {
    if let Some(gl) = with_gl(ctx) { gl.flush(); }
}
unsafe extern "C" fn framebuffer_renderbuffer(ctx: PP_Resource, target: GLenum, attachment: GLenum, rbtarget: GLenum, rb: GLuint) {
    let _ = rbtarget;
    if let Some(gl) = with_gl(ctx) { gl.framebuffer_renderbuffer(target, attachment, glow::RENDERBUFFER, to_rbo(rb)); }
}
unsafe extern "C" fn framebuffer_texture_2d(ctx: PP_Resource, target: GLenum, attachment: GLenum, textarget: GLenum, texture: GLuint, level: GLint) {
    if let Some(gl) = with_gl(ctx) { gl.framebuffer_texture_2d(target, attachment, textarget, to_tex(texture), level); }
}
unsafe extern "C" fn front_face(ctx: PP_Resource, mode: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.front_face(mode); }
}
unsafe extern "C" fn gen_buffers(ctx: PP_Resource, n: GLsizei, bufs: *mut GLuint) {
    if let Some(gl) = with_gl(ctx) {
        let ids = std::slice::from_raw_parts_mut(bufs, n as usize);
        for id in ids.iter_mut() {
            *id = gl.create_buffer().map_or(0, |b| b.0.get());
        }
    }
}
unsafe extern "C" fn generate_mipmap(ctx: PP_Resource, target: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.generate_mipmap(target); }
}
unsafe extern "C" fn gen_framebuffers(ctx: PP_Resource, n: GLsizei, fbs: *mut GLuint) {
    if let Some(gl) = with_gl(ctx) {
        let ids = std::slice::from_raw_parts_mut(fbs, n as usize);
        for id in ids.iter_mut() {
            *id = gl.create_framebuffer().map_or(0, |f| f.0.get());
        }
    }
}
unsafe extern "C" fn gen_renderbuffers(ctx: PP_Resource, n: GLsizei, rbs: *mut GLuint) {
    if let Some(gl) = with_gl(ctx) {
        let ids = std::slice::from_raw_parts_mut(rbs, n as usize);
        for id in ids.iter_mut() {
            *id = gl.create_renderbuffer().map_or(0, |r| r.0.get());
        }
    }
}
unsafe extern "C" fn gen_textures(ctx: PP_Resource, n: GLsizei, texs: *mut GLuint) {
    if let Some(gl) = with_gl(ctx) {
        let ids = std::slice::from_raw_parts_mut(texs, n as usize);
        for id in ids.iter_mut() {
            *id = gl.create_texture().map_or(0, |t| t.0.get());
        }
    }
}
unsafe extern "C" fn get_active_attrib(ctx: PP_Resource, prog: GLuint, idx: GLuint, bufsize: GLsizei, length: *mut GLsizei, size: *mut GLint, type_: *mut GLenum, name: *mut c_char) {
    if let Some(gl) = with_gl(ctx) {
        let aattr = gl.get_active_attribute(to_program(prog), idx);
        if let Some(a) = aattr {
            *size = a.size;
            *type_ = a.atype;
            let bytes = a.name.as_bytes();
            let copy_len = bytes.len().min((bufsize as usize).saturating_sub(1));
            ptr::copy_nonoverlapping(bytes.as_ptr(), name as *mut u8, copy_len);
            *name.add(copy_len) = 0;
            if !length.is_null() { *length = copy_len as GLsizei; }
        }
    }
}
unsafe extern "C" fn get_active_uniform(ctx: PP_Resource, prog: GLuint, idx: GLuint, bufsize: GLsizei, length: *mut GLsizei, size: *mut GLint, type_: *mut GLenum, name: *mut c_char) {
    if let Some(gl) = with_gl(ctx) {
        let u = gl.get_active_uniform(to_program(prog), idx);
        if let Some(u) = u {
            *size = u.size;
            *type_ = u.utype;
            let bytes = u.name.as_bytes();
            let copy_len = bytes.len().min((bufsize as usize).saturating_sub(1));
            ptr::copy_nonoverlapping(bytes.as_ptr(), name as *mut u8, copy_len);
            *name.add(copy_len) = 0;
            if !length.is_null() { *length = copy_len as GLsizei; }
        }
    }
}
unsafe extern "C" fn get_attached_shaders(ctx: PP_Resource, prog: GLuint, max: GLsizei, count: *mut GLsizei, shaders: *mut GLuint) {
    // glow doesn't expose glGetAttachedShaders. Use raw GL.
    if with_gl(ctx).is_some() {
        type GetAttachedShadersFn = unsafe extern "system" fn(u32, i32, *mut i32, *mut u32);
        static RAW: OnceLock<Option<GetAttachedShadersFn>> = OnceLock::new();
        let f = RAW.get_or_init(|| load_raw_gl(c"glGetAttachedShaders"));
        if let Some(func) = f {
            func(prog, max, count, shaders);
        } else {
            if !count.is_null() { *count = 0; }
        }
    }
}
unsafe extern "C" fn get_attrib_location(ctx: PP_Resource, prog: GLuint, name: *const c_char) -> GLint {
    with_gl(ctx).map_or(-1, |gl| {
        let s = CStr::from_ptr(name).to_str().unwrap_or("");
        gl.get_attrib_location(to_program(prog), s).map_or(-1, |l| l as GLint)
    })
}
unsafe extern "C" fn get_booleanv(ctx: PP_Resource, pname: GLenum, params: *mut GLboolean) {
    // Must use raw glGetBooleanv - glow's get_parameter_bool only reads one
    // value, but pnames like GL_COLOR_WRITEMASK write 4 booleans.
    if with_gl(ctx).is_some() {
        let f = RAW_GET_BOOLEANV.get_or_init(|| load_raw_gl(c"glGetBooleanv"));
        if let Some(func) = f { func(pname, params); }
    }
}
unsafe extern "C" fn get_buffer_parameteriv(ctx: PP_Resource, target: GLenum, pname: GLenum, params: *mut GLint) {
    if let Some(gl) = with_gl(ctx) { *params = gl.get_buffer_parameter_i32(target, pname); }
}
unsafe extern "C" fn get_error(ctx: PP_Resource) -> GLenum {
    with_gl(ctx).map_or(0, |gl| gl.get_error())
}
unsafe extern "C" fn get_floatv(ctx: PP_Resource, pname: GLenum, params: *mut GLfloat) {
    // Must use raw glGetFloatv - glow's get_parameter_f32 only reads one
    // value, but pnames like GL_DEPTH_RANGE / GL_BLEND_COLOR write 2-4 floats.
    if with_gl(ctx).is_some() {
        let f = RAW_GET_FLOATV.get_or_init(|| load_raw_gl(c"glGetFloatv"));
        if let Some(func) = f { func(pname, params); }
    }
}
unsafe extern "C" fn get_framebuffer_attachment_parameteriv(ctx: PP_Resource, target: GLenum, attachment: GLenum, pname: GLenum, params: *mut GLint) {
    if let Some(gl) = with_gl(ctx) {
        *params = gl.get_framebuffer_attachment_parameter_i32(target, attachment, pname);
    }
}
unsafe extern "C" fn get_integerv(ctx: PP_Resource, pname: GLenum, params: *mut GLint) {
    // Must use raw glGetIntegerv - glow's get_parameter_i32 only reads one
    // value, but pnames like GL_VIEWPORT / GL_SCISSOR_BOX write 4 ints,
    // which overflows glow's single-value stack variable and corrupts the stack.
    if with_gl(ctx).is_some() {
        let f = RAW_GET_INTEGERV.get_or_init(|| load_raw_gl(c"glGetIntegerv"));
        if let Some(func) = f { func(pname, params); }
    }
}
unsafe extern "C" fn get_programiv(ctx: PP_Resource, prog: GLuint, pname: GLenum, params: *mut GLint) {
    if let Some(gl) = with_gl(ctx) { *params = gl.get_program_parameter_i32(to_program(prog), pname); }
}
unsafe extern "C" fn get_program_info_log(ctx: PP_Resource, prog: GLuint, bufsize: GLsizei, length: *mut GLsizei, infolog: *mut c_char) {
    if let Some(gl) = with_gl(ctx) {
        let log = gl.get_program_info_log(to_program(prog));
        let bytes = log.as_bytes();
        let copy_len = bytes.len().min((bufsize as usize).saturating_sub(1));
        ptr::copy_nonoverlapping(bytes.as_ptr(), infolog as *mut u8, copy_len);
        *infolog.add(copy_len) = 0;
        if !length.is_null() { *length = copy_len as GLsizei; }
    }
}
unsafe extern "C" fn get_renderbuffer_parameteriv(ctx: PP_Resource, target: GLenum, pname: GLenum, params: *mut GLint) {
    if let Some(gl) = with_gl(ctx) {
        *params = gl.get_renderbuffer_parameter_i32(target, pname);
    }
}
unsafe extern "C" fn get_shaderiv(ctx: PP_Resource, shader: GLuint, pname: GLenum, params: *mut GLint) {
    if let Some(gl) = with_gl(ctx) {
        let s = to_shader(shader);
        *params = match pname {
            glow::COMPILE_STATUS => if gl.get_shader_compile_status(s) { 1 } else { 0 },
            glow::DELETE_STATUS | glow::SHADER_TYPE => {
                // Use raw glGetShaderiv for pnames glow doesn't wrap.
                type GetShaderivFn = unsafe extern "system" fn(u32, u32, *mut i32);
                static GET_SHADERIV: std::sync::OnceLock<Option<GetShaderivFn>> = std::sync::OnceLock::new();
                let f = GET_SHADERIV.get_or_init(|| {
                    let p = gl_context::gl_proc_address(c"glGetShaderiv");
                    if p.is_null() { None } else { Some(std::mem::transmute(p)) }
                });
                if let Some(func) = f {
                    let mut v = 0i32;
                    func(shader, pname, &mut v);
                    v
                } else { 0 }
            }
            glow::INFO_LOG_LENGTH => gl.get_shader_info_log(s).len() as i32 + 1,
            glow::SHADER_SOURCE_LENGTH => {
                // glow doesn't expose get_shader_source; return 0.
                0
            }
            _ => 0,
        };
    }
}
unsafe extern "C" fn get_shader_info_log(ctx: PP_Resource, shader: GLuint, bufsize: GLsizei, length: *mut GLsizei, infolog: *mut c_char) {
    if let Some(gl) = with_gl(ctx) {
        let log = gl.get_shader_info_log(to_shader(shader));
        let bytes = log.as_bytes();
        let copy_len = bytes.len().min((bufsize as usize).saturating_sub(1));
        ptr::copy_nonoverlapping(bytes.as_ptr(), infolog as *mut u8, copy_len);
        *infolog.add(copy_len) = 0;
        if !length.is_null() { *length = copy_len as GLsizei; }
    }
}
unsafe extern "C" fn get_shader_precision_format(_ctx: PP_Resource, _shadertype: GLenum, _precisiontype: GLenum, range: *mut GLint, precision: *mut GLint) {
    // glow doesn't expose this. Provide reasonable defaults.
    if !range.is_null() { *range = 127; *range.add(1) = 127; }
    if !precision.is_null() { *precision = 23; }
}
unsafe extern "C" fn get_shader_source(ctx: PP_Resource, shader: GLuint, bufsize: GLsizei, length: *mut GLsizei, source: *mut c_char) {
    // glow doesn't expose glGetShaderSource. Use raw GL.
    if with_gl(ctx).is_some() {
        type GetShaderSourceFn = unsafe extern "system" fn(u32, i32, *mut i32, *mut c_char);
        static GET_SHADER_SOURCE: std::sync::OnceLock<Option<GetShaderSourceFn>> = std::sync::OnceLock::new();
        let f = GET_SHADER_SOURCE.get_or_init(|| {
            let p = gl_context::gl_proc_address(c"glGetShaderSource");
            if p.is_null() { None } else { Some(std::mem::transmute(p)) }
        });
        if let Some(func) = f {
            func(shader, bufsize, length, source);
        } else {
            if bufsize > 0 { *source = 0; }
            if !length.is_null() { *length = 0; }
        }
    }
}
unsafe extern "C" fn get_string(ctx: PP_Resource, name: GLenum) -> *const GLubyte {
    // glGetString returns a null-terminated C string.  glow returns a Rust
    // String (no null terminator), so we must append one before leaking.
    with_gl(ctx).map_or(b"\0".as_ptr(), |gl| {
        let s = gl.get_parameter_string(name);
        let mut bytes = s.into_bytes();
        bytes.push(0); // null terminator
        let leaked = Box::leak(bytes.into_boxed_slice());
        leaked.as_ptr()
    })
}
unsafe extern "C" fn get_tex_parameterfv(ctx: PP_Resource, target: GLenum, pname: GLenum, params: *mut GLfloat) {
    if let Some(gl) = with_gl(ctx) { *params = gl.get_tex_parameter_f32(target, pname); }
}
unsafe extern "C" fn get_tex_parameteriv(ctx: PP_Resource, target: GLenum, pname: GLenum, params: *mut GLint) {
    if let Some(gl) = with_gl(ctx) { *params = gl.get_tex_parameter_i32(target, pname); }
}
unsafe extern "C" fn get_uniformfv(ctx: PP_Resource, prog: GLuint, loc: GLint, params: *mut GLfloat) {
    if let Some(gl) = with_gl(ctx) {
        if let Some(ul) = to_uniform(loc) {
            // glow writes all components into the slice; copy the full
            // result back to the plugin's buffer.  16 floats covers mat4.
            let mut buf = [0f32; 16];
            gl.get_uniform_f32(to_program(prog), &ul, &mut buf);
            ptr::copy_nonoverlapping(buf.as_ptr(), params, 16);
        }
    }
}
unsafe extern "C" fn get_uniformiv(ctx: PP_Resource, prog: GLuint, loc: GLint, params: *mut GLint) {
    if let Some(gl) = with_gl(ctx) {
        if let Some(ul) = to_uniform(loc) {
            let mut buf = [0i32; 16];
            gl.get_uniform_i32(to_program(prog), &ul, &mut buf);
            ptr::copy_nonoverlapping(buf.as_ptr(), params, 16);
        }
    }
}
unsafe extern "C" fn get_uniform_location(ctx: PP_Resource, prog: GLuint, name: *const c_char) -> GLint {
    with_gl(ctx).map_or(-1, |gl| {
        let s = CStr::from_ptr(name).to_str().unwrap_or("");
        gl.get_uniform_location(to_program(prog), s).map_or(-1, |l| l.0 as GLint)
    })
}
unsafe extern "C" fn get_vertex_attribfv(ctx: PP_Resource, idx: GLuint, pname: GLenum, params: *mut GLfloat) {
    if let Some(gl) = with_gl(ctx) {
        let mut buf = [0f32; 4];
        gl.get_vertex_attrib_parameter_f32_slice(idx, pname, &mut buf);
        // GL_CURRENT_VERTEX_ATTRIB returns 4 floats; others return 1.
        let count = if pname == glow::CURRENT_VERTEX_ATTRIB { 4 } else { 1 };
        ptr::copy_nonoverlapping(buf.as_ptr(), params, count);
    }
}
unsafe extern "C" fn get_vertex_attribiv(ctx: PP_Resource, idx: GLuint, pname: GLenum, params: *mut GLint) {
    // glow doesn't expose get_vertex_attrib_parameter_i32. Use raw GL.
    if with_gl(ctx).is_some() {
        type GetVertexAttribivFn = unsafe extern "system" fn(u32, u32, *mut i32);
        static GET_VERTEX_ATTRIBIV: std::sync::OnceLock<Option<GetVertexAttribivFn>> = std::sync::OnceLock::new();
        let f = GET_VERTEX_ATTRIBIV.get_or_init(|| {
            let p = gl_context::gl_proc_address(c"glGetVertexAttribiv");
            if p.is_null() { None } else { Some(std::mem::transmute(p)) }
        });
        if let Some(func) = f {
            func(idx, pname, params);
        }
    }
}
unsafe extern "C" fn get_vertex_attrib_pointerv(_ctx: PP_Resource, _idx: GLuint, _pname: GLenum, _ptr_out: *mut *mut c_void) {
    // glow doesn't expose this. Leave as no-op.
}
unsafe extern "C" fn hint(ctx: PP_Resource, target: GLenum, mode: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.hint(target, mode); }
}
unsafe extern "C" fn is_buffer(ctx: PP_Resource, buffer: GLuint) -> GLboolean {
    with_gl(ctx).map_or(0, |gl| if gl.is_buffer(to_buf(buffer).unwrap_or(glow::NativeBuffer(NonZeroU32::new(u32::MAX).unwrap()))) { 1 } else { 0 })
}
unsafe extern "C" fn is_enabled(ctx: PP_Resource, cap: GLenum) -> GLboolean {
    with_gl(ctx).map_or(0, |gl| if gl.is_enabled(cap) { 1 } else { 0 })
}
unsafe extern "C" fn is_framebuffer(ctx: PP_Resource, fb: GLuint) -> GLboolean {
    with_gl(ctx).map_or(0, |gl| if gl.is_framebuffer(to_fbo(fb).unwrap_or(glow::NativeFramebuffer(NonZeroU32::new(u32::MAX).unwrap()))) { 1 } else { 0 })
}
unsafe extern "C" fn is_program(ctx: PP_Resource, prog: GLuint) -> GLboolean {
    with_gl(ctx).map_or(0, |gl| if gl.is_program(to_program(prog)) { 1 } else { 0 })
}
unsafe extern "C" fn is_renderbuffer(ctx: PP_Resource, rb: GLuint) -> GLboolean {
    with_gl(ctx).map_or(0, |gl| if gl.is_renderbuffer(to_rbo(rb).unwrap_or(glow::NativeRenderbuffer(NonZeroU32::new(u32::MAX).unwrap()))) { 1 } else { 0 })
}
unsafe extern "C" fn is_shader(ctx: PP_Resource, shader: GLuint) -> GLboolean {
    with_gl(ctx).map_or(0, |gl| if gl.is_shader(to_shader(shader)) { 1 } else { 0 })
}
unsafe extern "C" fn is_texture(ctx: PP_Resource, tex: GLuint) -> GLboolean {
    with_gl(ctx).map_or(0, |gl| if gl.is_texture(to_tex(tex).unwrap_or(glow::NativeTexture(NonZeroU32::new(u32::MAX).unwrap()))) { 1 } else { 0 })
}
unsafe extern "C" fn line_width(ctx: PP_Resource, width: GLfloat) {
    if let Some(gl) = with_gl(ctx) { gl.line_width(width); }
}
unsafe extern "C" fn link_program(ctx: PP_Resource, prog: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.link_program(to_program(prog)); }
}
unsafe extern "C" fn pixel_storei(ctx: PP_Resource, pname: GLenum, param: GLint) {
    if let Some(gl) = with_gl(ctx) { gl.pixel_store_i32(pname, param); }
}
unsafe extern "C" fn polygon_offset(ctx: PP_Resource, factor: GLfloat, units: GLfloat) {
    if let Some(gl) = with_gl(ctx) { gl.polygon_offset(factor, units); }
}
unsafe extern "C" fn read_pixels(ctx: PP_Resource, x: GLint, y: GLint, w: GLsizei, h: GLsizei, fmt: GLenum, type_: GLenum, pixels: *mut c_void) {
    if let Some(gl) = with_gl(ctx) {
        let size = (w as usize) * (h as usize) * pixel_byte_size(fmt, type_);
        let slice = std::slice::from_raw_parts_mut(pixels as *mut u8, size);
        gl.read_pixels(x, y, w, h, fmt, type_, glow::PixelPackData::Slice(Some(slice)));
    }
}
unsafe extern "C" fn release_shader_compiler(_ctx: PP_Resource) {
    // No-op in glow / most GLES2 implementations.
}
unsafe extern "C" fn renderbuffer_storage(ctx: PP_Resource, target: GLenum, fmt: GLenum, w: GLsizei, h: GLsizei) {
    if let Some(gl) = with_gl(ctx) { gl.renderbuffer_storage(target, fmt, w, h); }
}
unsafe extern "C" fn sample_coverage(ctx: PP_Resource, value: GLclampf, invert: GLboolean) {
    if let Some(gl) = with_gl(ctx) { gl.sample_coverage(value, invert != 0); }
}
unsafe extern "C" fn scissor(ctx: PP_Resource, x: GLint, y: GLint, w: GLsizei, h: GLsizei) {
    if let Some(gl) = with_gl(ctx) { gl.scissor(x, y, w, h); }
}
unsafe extern "C" fn shader_binary(_ctx: PP_Resource, _n: GLsizei, _shaders: *const GLuint, _fmt: GLenum, _binary: *const c_void, _length: GLsizei) {
    // Not commonly supported / not exposed by glow. No-op.
}
unsafe extern "C" fn shader_source(ctx: PP_Resource, shader: GLuint, count: GLsizei, str_: *const *const c_char, length: *const GLint) {
    if let Some(gl) = with_gl(ctx) {
        // Concatenate all source strings into one.
        let mut src = String::new();
        let strs = std::slice::from_raw_parts(str_, count as usize);
        let lens = if length.is_null() { &[] as &[GLint] } else {
            std::slice::from_raw_parts(length, count as usize)
        };
        for (i, &s) in strs.iter().enumerate() {
            if s.is_null() { continue; }
            let part = if i < lens.len() && lens[i] >= 0 {
                let sl = std::slice::from_raw_parts(s as *const u8, lens[i] as usize);
                std::str::from_utf8_unchecked(sl)
            } else {
                CStr::from_ptr(s).to_str().unwrap_or("")
            };
            src.push_str(part);
        }
        gl.shader_source(to_shader(shader), &src);
    }
}
unsafe extern "C" fn stencil_func(ctx: PP_Resource, func: GLenum, ref_: GLint, mask: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.stencil_func(func, ref_, mask); }
}
unsafe extern "C" fn stencil_func_separate(ctx: PP_Resource, face: GLenum, func: GLenum, ref_: GLint, mask: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.stencil_func_separate(face, func, ref_, mask); }
}
unsafe extern "C" fn stencil_mask(ctx: PP_Resource, mask: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.stencil_mask(mask); }
}
unsafe extern "C" fn stencil_mask_separate(ctx: PP_Resource, face: GLenum, mask: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.stencil_mask_separate(face, mask); }
}
unsafe extern "C" fn stencil_op(ctx: PP_Resource, fail: GLenum, zfail: GLenum, zpass: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.stencil_op(fail, zfail, zpass); }
}
unsafe extern "C" fn stencil_op_separate(ctx: PP_Resource, face: GLenum, fail: GLenum, zfail: GLenum, zpass: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.stencil_op_separate(face, fail, zfail, zpass); }
}
unsafe extern "C" fn tex_image_2d(ctx: PP_Resource, target: GLenum, level: GLint, ifmt: GLint, w: GLsizei, h: GLsizei, border: GLint, fmt: GLenum, type_: GLenum, pixels: *const c_void) {
    if let Some(gl) = with_gl(ctx) {
        let data = if pixels.is_null() {
            glow::PixelUnpackData::Slice(None)
        } else {
            let size = (w as usize) * (h as usize) * pixel_byte_size(fmt, type_);
            glow::PixelUnpackData::Slice(Some(std::slice::from_raw_parts(pixels as *const u8, size)))
        };
        gl.tex_image_2d(target, level, ifmt, w, h, border, fmt, type_, data);
    }
}
unsafe extern "C" fn tex_parameterf(ctx: PP_Resource, target: GLenum, pname: GLenum, param: GLfloat) {
    if let Some(gl) = with_gl(ctx) { gl.tex_parameter_f32(target, pname, param); }
}
unsafe extern "C" fn tex_parameterfv(ctx: PP_Resource, target: GLenum, pname: GLenum, params: *const GLfloat) {
    if let Some(gl) = with_gl(ctx) { gl.tex_parameter_f32(target, pname, *params); }
}
unsafe extern "C" fn tex_parameteri(ctx: PP_Resource, target: GLenum, pname: GLenum, param: GLint) {
    if let Some(gl) = with_gl(ctx) { gl.tex_parameter_i32(target, pname, param); }
}
unsafe extern "C" fn tex_parameteriv(ctx: PP_Resource, target: GLenum, pname: GLenum, params: *const GLint) {
    if let Some(gl) = with_gl(ctx) { gl.tex_parameter_i32(target, pname, *params); }
}
unsafe extern "C" fn tex_sub_image_2d(ctx: PP_Resource, target: GLenum, level: GLint, x: GLint, y: GLint, w: GLsizei, h: GLsizei, fmt: GLenum, type_: GLenum, pixels: *const c_void) {
    if let Some(gl) = with_gl(ctx) {
        let size = (w as usize) * (h as usize) * pixel_byte_size(fmt, type_);
        let data = glow::PixelUnpackData::Slice(if pixels.is_null() { None } else {
            Some(std::slice::from_raw_parts(pixels as *const u8, size))
        });
        gl.tex_sub_image_2d(target, level, x, y, w, h, fmt, type_, data);
    }
}
unsafe extern "C" fn uniform1f(ctx: PP_Resource, loc: GLint, x: GLfloat) { if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { gl.uniform_1_f32(Some(&u), x); } } }
unsafe extern "C" fn uniform1fv(ctx: PP_Resource, loc: GLint, count: GLsizei, v: *const GLfloat) {
    if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { let s = std::slice::from_raw_parts(v, count as usize); gl.uniform_1_f32_slice(Some(&u), s); } }
}
unsafe extern "C" fn uniform1i(ctx: PP_Resource, loc: GLint, x: GLint) { if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { gl.uniform_1_i32(Some(&u), x); } } }
unsafe extern "C" fn uniform1iv(ctx: PP_Resource, loc: GLint, count: GLsizei, v: *const GLint) {
    if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { let s = std::slice::from_raw_parts(v, count as usize); gl.uniform_1_i32_slice(Some(&u), s); } }
}
unsafe extern "C" fn uniform2f(ctx: PP_Resource, loc: GLint, x: GLfloat, y: GLfloat) { if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { gl.uniform_2_f32(Some(&u), x, y); } } }
unsafe extern "C" fn uniform2fv(ctx: PP_Resource, loc: GLint, count: GLsizei, v: *const GLfloat) {
    if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { let s = std::slice::from_raw_parts(v, (count * 2) as usize); gl.uniform_2_f32_slice(Some(&u), s); } }
}
unsafe extern "C" fn uniform2i(ctx: PP_Resource, loc: GLint, x: GLint, y: GLint) { if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { gl.uniform_2_i32(Some(&u), x, y); } } }
unsafe extern "C" fn uniform2iv(ctx: PP_Resource, loc: GLint, count: GLsizei, v: *const GLint) {
    if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { let s = std::slice::from_raw_parts(v, (count * 2) as usize); gl.uniform_2_i32_slice(Some(&u), s); } }
}
unsafe extern "C" fn uniform3f(ctx: PP_Resource, loc: GLint, x: GLfloat, y: GLfloat, z: GLfloat) { if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { gl.uniform_3_f32(Some(&u), x, y, z); } } }
unsafe extern "C" fn uniform3fv(ctx: PP_Resource, loc: GLint, count: GLsizei, v: *const GLfloat) {
    if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { let s = std::slice::from_raw_parts(v, (count * 3) as usize); gl.uniform_3_f32_slice(Some(&u), s); } }
}
unsafe extern "C" fn uniform3i(ctx: PP_Resource, loc: GLint, x: GLint, y: GLint, z: GLint) { if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { gl.uniform_3_i32(Some(&u), x, y, z); } } }
unsafe extern "C" fn uniform3iv(ctx: PP_Resource, loc: GLint, count: GLsizei, v: *const GLint) {
    if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { let s = std::slice::from_raw_parts(v, (count * 3) as usize); gl.uniform_3_i32_slice(Some(&u), s); } }
}
unsafe extern "C" fn uniform4f(ctx: PP_Resource, loc: GLint, x: GLfloat, y: GLfloat, z: GLfloat, w: GLfloat) { if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { gl.uniform_4_f32(Some(&u), x, y, z, w); } } }
unsafe extern "C" fn uniform4fv(ctx: PP_Resource, loc: GLint, count: GLsizei, v: *const GLfloat) {
    if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { let s = std::slice::from_raw_parts(v, (count * 4) as usize); gl.uniform_4_f32_slice(Some(&u), s); } }
}
unsafe extern "C" fn uniform4i(ctx: PP_Resource, loc: GLint, x: GLint, y: GLint, z: GLint, w: GLint) { if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { gl.uniform_4_i32(Some(&u), x, y, z, w); } } }
unsafe extern "C" fn uniform4iv(ctx: PP_Resource, loc: GLint, count: GLsizei, v: *const GLint) {
    if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { let s = std::slice::from_raw_parts(v, (count * 4) as usize); gl.uniform_4_i32_slice(Some(&u), s); } }
}
unsafe extern "C" fn uniform_matrix2fv(ctx: PP_Resource, loc: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat) {
    if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { let s = std::slice::from_raw_parts(value, (count * 4) as usize); gl.uniform_matrix_2_f32_slice(Some(&u), transpose != 0, s); } }
}
unsafe extern "C" fn uniform_matrix3fv(ctx: PP_Resource, loc: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat) {
    if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { let s = std::slice::from_raw_parts(value, (count * 9) as usize); gl.uniform_matrix_3_f32_slice(Some(&u), transpose != 0, s); } }
}
unsafe extern "C" fn uniform_matrix4fv(ctx: PP_Resource, loc: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat) {
    if let Some(gl) = with_gl(ctx) { if let Some(u) = to_uniform(loc) { let s = std::slice::from_raw_parts(value, (count * 16) as usize); gl.uniform_matrix_4_f32_slice(Some(&u), transpose != 0, s); } }
}
unsafe extern "C" fn use_program(ctx: PP_Resource, prog: GLuint) { if let Some(gl) = with_gl(ctx) { gl.use_program(if prog == 0 { None } else { Some(to_program(prog)) }); } }
unsafe extern "C" fn validate_program(ctx: PP_Resource, prog: GLuint) { if let Some(gl) = with_gl(ctx) { gl.validate_program(to_program(prog)); } }
unsafe extern "C" fn vertex_attrib1f(ctx: PP_Resource, idx: GLuint, x: GLfloat) { if let Some(gl) = with_gl(ctx) { gl.vertex_attrib_1_f32(idx, x); } }
unsafe extern "C" fn vertex_attrib1fv(ctx: PP_Resource, idx: GLuint, v: *const GLfloat) { if let Some(gl) = with_gl(ctx) { gl.vertex_attrib_1_f32(idx, *v); } }
unsafe extern "C" fn vertex_attrib2f(ctx: PP_Resource, idx: GLuint, x: GLfloat, y: GLfloat) { if let Some(gl) = with_gl(ctx) { gl.vertex_attrib_2_f32(idx, x, y); } }
unsafe extern "C" fn vertex_attrib2fv(ctx: PP_Resource, idx: GLuint, v: *const GLfloat) { if let Some(gl) = with_gl(ctx) { let s = std::slice::from_raw_parts(v, 2); gl.vertex_attrib_2_f32(idx, s[0], s[1]); } }
unsafe extern "C" fn vertex_attrib3f(ctx: PP_Resource, idx: GLuint, x: GLfloat, y: GLfloat, z: GLfloat) { if let Some(gl) = with_gl(ctx) { gl.vertex_attrib_3_f32(idx, x, y, z); } }
unsafe extern "C" fn vertex_attrib3fv(ctx: PP_Resource, idx: GLuint, v: *const GLfloat) { if let Some(gl) = with_gl(ctx) { let s = std::slice::from_raw_parts(v, 3); gl.vertex_attrib_3_f32(idx, s[0], s[1], s[2]); } }
unsafe extern "C" fn vertex_attrib4f(ctx: PP_Resource, idx: GLuint, x: GLfloat, y: GLfloat, z: GLfloat, w: GLfloat) { if let Some(gl) = with_gl(ctx) { gl.vertex_attrib_4_f32(idx, x, y, z, w); } }
unsafe extern "C" fn vertex_attrib4fv(ctx: PP_Resource, idx: GLuint, v: *const GLfloat) { if let Some(gl) = with_gl(ctx) { let s = std::slice::from_raw_parts(v, 4); gl.vertex_attrib_4_f32(idx, s[0], s[1], s[2], s[3]); } }
unsafe extern "C" fn vertex_attrib_pointer(ctx: PP_Resource, idx: GLuint, size: GLint, type_: GLenum, normalized: GLboolean, stride: GLsizei, ptr_: *const c_void) {
    if let Some(gl) = with_gl(ctx) { gl.vertex_attrib_pointer_f32(idx, size, type_, normalized != 0, stride, ptr_ as i32); }
}
unsafe extern "C" fn viewport(ctx: PP_Resource, x: GLint, y: GLint, w: GLsizei, h: GLsizei) {
    if let Some(gl) = with_gl(ctx) { gl.viewport(x, y, w, h); }
}

static VTABLE: PPB_OpenGLES2_1_0 = PPB_OpenGLES2_1_0 {
    ActiveTexture: Some(active_texture), AttachShader: Some(attach_shader),
    BindAttribLocation: Some(bind_attrib_location), BindBuffer: Some(bind_buffer),
    BindFramebuffer: Some(bind_framebuffer), BindRenderbuffer: Some(bind_renderbuffer),
    BindTexture: Some(bind_texture), BlendColor: Some(blend_color),
    BlendEquation: Some(blend_equation), BlendEquationSeparate: Some(blend_equation_separate),
    BlendFunc: Some(blend_func), BlendFuncSeparate: Some(blend_func_separate),
    BufferData: Some(buffer_data), BufferSubData: Some(buffer_sub_data),
    CheckFramebufferStatus: Some(check_framebuffer_status), Clear: Some(clear),
    ClearColor: Some(clear_color), ClearDepthf: Some(clear_depthf),
    ClearStencil: Some(clear_stencil), ColorMask: Some(color_mask),
    CompileShader: Some(compile_shader), CompressedTexImage2D: Some(compressed_tex_image_2d),
    CompressedTexSubImage2D: Some(compressed_tex_sub_image_2d),
    CopyTexImage2D: Some(copy_tex_image_2d), CopyTexSubImage2D: Some(copy_tex_sub_image_2d),
    CreateProgram: Some(create_program), CreateShader: Some(create_shader),
    CullFace: Some(cull_face), DeleteBuffers: Some(delete_buffers),
    DeleteFramebuffers: Some(delete_framebuffers), DeleteProgram: Some(delete_program),
    DeleteRenderbuffers: Some(delete_renderbuffers), DeleteShader: Some(delete_shader),
    DeleteTextures: Some(delete_textures), DepthFunc: Some(depth_func),
    DepthMask: Some(depth_mask), DepthRangef: Some(depth_rangef),
    DetachShader: Some(detach_shader), Disable: Some(disable),
    DisableVertexAttribArray: Some(disable_vertex_attrib_array),
    DrawArrays: Some(draw_arrays), DrawElements: Some(draw_elements),
    Enable: Some(enable), EnableVertexAttribArray: Some(enable_vertex_attrib_array),
    Finish: Some(finish), Flush: Some(flush),
    FramebufferRenderbuffer: Some(framebuffer_renderbuffer),
    FramebufferTexture2D: Some(framebuffer_texture_2d),
    FrontFace: Some(front_face), GenBuffers: Some(gen_buffers),
    GenerateMipmap: Some(generate_mipmap), GenFramebuffers: Some(gen_framebuffers),
    GenRenderbuffers: Some(gen_renderbuffers), GenTextures: Some(gen_textures),
    GetActiveAttrib: Some(get_active_attrib), GetActiveUniform: Some(get_active_uniform),
    GetAttachedShaders: Some(get_attached_shaders), GetAttribLocation: Some(get_attrib_location),
    GetBooleanv: Some(get_booleanv), GetBufferParameteriv: Some(get_buffer_parameteriv),
    GetError: Some(get_error), GetFloatv: Some(get_floatv),
    GetFramebufferAttachmentParameteriv: Some(get_framebuffer_attachment_parameteriv),
    GetIntegerv: Some(get_integerv), GetProgramiv: Some(get_programiv),
    GetProgramInfoLog: Some(get_program_info_log),
    GetRenderbufferParameteriv: Some(get_renderbuffer_parameteriv),
    GetShaderiv: Some(get_shaderiv), GetShaderInfoLog: Some(get_shader_info_log),
    GetShaderPrecisionFormat: Some(get_shader_precision_format),
    GetShaderSource: Some(get_shader_source), GetString: Some(get_string),
    GetTexParameterfv: Some(get_tex_parameterfv), GetTexParameteriv: Some(get_tex_parameteriv),
    GetUniformfv: Some(get_uniformfv), GetUniformiv: Some(get_uniformiv),
    GetUniformLocation: Some(get_uniform_location),
    GetVertexAttribfv: Some(get_vertex_attribfv), GetVertexAttribiv: Some(get_vertex_attribiv),
    GetVertexAttribPointerv: Some(get_vertex_attrib_pointerv),
    Hint: Some(hint), IsBuffer: Some(is_buffer), IsEnabled: Some(is_enabled),
    IsFramebuffer: Some(is_framebuffer), IsProgram: Some(is_program),
    IsRenderbuffer: Some(is_renderbuffer), IsShader: Some(is_shader),
    IsTexture: Some(is_texture), LineWidth: Some(line_width),
    LinkProgram: Some(link_program), PixelStorei: Some(pixel_storei),
    PolygonOffset: Some(polygon_offset), ReadPixels: Some(read_pixels),
    ReleaseShaderCompiler: Some(release_shader_compiler),
    RenderbufferStorage: Some(renderbuffer_storage), SampleCoverage: Some(sample_coverage),
    Scissor: Some(scissor), ShaderBinary: Some(shader_binary),
    ShaderSource: Some(shader_source), StencilFunc: Some(stencil_func),
    StencilFuncSeparate: Some(stencil_func_separate), StencilMask: Some(stencil_mask),
    StencilMaskSeparate: Some(stencil_mask_separate), StencilOp: Some(stencil_op),
    StencilOpSeparate: Some(stencil_op_separate), TexImage2D: Some(tex_image_2d),
    TexParameterf: Some(tex_parameterf), TexParameterfv: Some(tex_parameterfv),
    TexParameteri: Some(tex_parameteri), TexParameteriv: Some(tex_parameteriv),
    TexSubImage2D: Some(tex_sub_image_2d),
    Uniform1f: Some(uniform1f), Uniform1fv: Some(uniform1fv),
    Uniform1i: Some(uniform1i), Uniform1iv: Some(uniform1iv),
    Uniform2f: Some(uniform2f), Uniform2fv: Some(uniform2fv),
    Uniform2i: Some(uniform2i), Uniform2iv: Some(uniform2iv),
    Uniform3f: Some(uniform3f), Uniform3fv: Some(uniform3fv),
    Uniform3i: Some(uniform3i), Uniform3iv: Some(uniform3iv),
    Uniform4f: Some(uniform4f), Uniform4fv: Some(uniform4fv),
    Uniform4i: Some(uniform4i), Uniform4iv: Some(uniform4iv),
    UniformMatrix2fv: Some(uniform_matrix2fv), UniformMatrix3fv: Some(uniform_matrix3fv),
    UniformMatrix4fv: Some(uniform_matrix4fv),
    UseProgram: Some(use_program), ValidateProgram: Some(validate_program),
    VertexAttrib1f: Some(vertex_attrib1f), VertexAttrib1fv: Some(vertex_attrib1fv),
    VertexAttrib2f: Some(vertex_attrib2f), VertexAttrib2fv: Some(vertex_attrib2fv),
    VertexAttrib3f: Some(vertex_attrib3f), VertexAttrib3fv: Some(vertex_attrib3fv),
    VertexAttrib4f: Some(vertex_attrib4f), VertexAttrib4fv: Some(vertex_attrib4fv),
    VertexAttribPointer: Some(vertex_attrib_pointer), Viewport: Some(viewport),
};

// ---------------------------------------------------------------------------
// Extension vtables
// ---------------------------------------------------------------------------

unsafe extern "C" fn draw_arrays_instanced(ctx: PP_Resource, mode: GLenum, first: GLint, count: GLsizei, primcount: GLsizei) {
    if let Some(gl) = with_gl(ctx) { gl.draw_arrays_instanced(mode, first, count, primcount); }
}
unsafe extern "C" fn draw_elements_instanced(ctx: PP_Resource, mode: GLenum, count: GLsizei, type_: GLenum, indices: *const c_void, primcount: GLsizei) {
    if let Some(gl) = with_gl(ctx) { gl.draw_elements_instanced(mode, count, type_, indices as i32, primcount); }
}
unsafe extern "C" fn vertex_attrib_divisor(ctx: PP_Resource, index: GLuint, divisor: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.vertex_attrib_divisor(index, divisor); }
}
static INSTANCED_ARRAYS_VTABLE: PPB_OpenGLES2InstancedArrays_1_0 = PPB_OpenGLES2InstancedArrays_1_0 {
    DrawArraysInstancedANGLE: Some(draw_arrays_instanced),
    DrawElementsInstancedANGLE: Some(draw_elements_instanced),
    VertexAttribDivisorANGLE: Some(vertex_attrib_divisor),
};

unsafe extern "C" fn blit_framebuffer(ctx: PP_Resource, sx0: GLint, sy0: GLint, sx1: GLint, sy1: GLint, dx0: GLint, dy0: GLint, dx1: GLint, dy1: GLint, mask: GLbitfield, filter: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.blit_framebuffer(sx0, sy0, sx1, sy1, dx0, dy0, dx1, dy1, mask, filter); }
}
static FRAMEBUFFER_BLIT_VTABLE: PPB_OpenGLES2FramebufferBlit_1_0 = PPB_OpenGLES2FramebufferBlit_1_0 { BlitFramebufferEXT: Some(blit_framebuffer) };

unsafe extern "C" fn renderbuffer_storage_multisample(ctx: PP_Resource, target: GLenum, samples: GLsizei, fmt: GLenum, w: GLsizei, h: GLsizei) {
    if let Some(gl) = with_gl(ctx) { gl.renderbuffer_storage_multisample(target, samples, fmt, w, h); }
}
static FRAMEBUFFER_MULTISAMPLE_VTABLE: PPB_OpenGLES2FramebufferMultisample_1_0 = PPB_OpenGLES2FramebufferMultisample_1_0 { RenderbufferStorageMultisampleEXT: Some(renderbuffer_storage_multisample) };

unsafe extern "C" fn enable_feature_chromium(_ctx: PP_Resource, feature: *const c_char) -> GLboolean {
    if feature.is_null() { return 0; }
    let name = unsafe { CStr::from_ptr(feature) }.to_str().unwrap_or("");
    match name {
        "pepper3d_allow_buffers_on_multiple_threads"
        | "pepper3d_support_image_chromium" => {
            tracing::debug!("EnableFeatureCHROMIUM: enabled '{}'", name);
            1
        }
        _ => {
            tracing::warn!("EnableFeatureCHROMIUM: unknown feature '{}'", name);
            0
        }
    }
}
static CHROMIUM_ENABLE_VTABLE: PPB_OpenGLES2ChromiumEnableFeature_1_0 = PPB_OpenGLES2ChromiumEnableFeature_1_0 { EnableFeatureCHROMIUM: Some(enable_feature_chromium) };

// ---------------------------------------------------------------------------
// Chromium MapSub - temp-buffer based buffer/texture upload
// ---------------------------------------------------------------------------

enum ChromiumMappingKind {
    Buffer { target: GLenum, offset: GLintptr },
    Texture { target: GLenum, level: GLint, x: GLint, y: GLint, w: GLsizei, h: GLsizei, format: GLenum, type_: GLenum },
}

struct ChromiumMapping {
    ctx: PP_Resource,
    kind: ChromiumMappingKind,
    data: Vec<u8>,
}

// Safety: data is a heap Vec only accessed under the mutex.
unsafe impl Send for ChromiumMapping {}

static CHROMIUM_MAPPINGS: Mutex<Option<HashMap<usize, ChromiumMapping>>> = Mutex::new(None);

fn chromium_mappings() -> &'static Mutex<Option<HashMap<usize, ChromiumMapping>>> {
    &CHROMIUM_MAPPINGS
}

unsafe extern "C" fn map_buffer_sub_data(ctx: PP_Resource, target: GLuint, offset: GLintptr, size: GLsizeiptr, _access: GLenum) -> *mut c_void {
    if size <= 0 { return ptr::null_mut(); }
    let mut data = vec![0u8; size as usize];
    let ptr = data.as_mut_ptr() as usize;
    let mapping = ChromiumMapping {
        ctx,
        kind: ChromiumMappingKind::Buffer { target, offset },
        data,
    };
    let mut guard = chromium_mappings().lock().unwrap();
    guard.get_or_insert_with(HashMap::new).insert(ptr, mapping);
    ptr as *mut c_void
}

unsafe extern "C" fn unmap_buffer_sub_data(_ctx: PP_Resource, mem: *const c_void) {
    if mem.is_null() { return; }
    let key = mem as usize;
    let mapping = {
        let mut guard = chromium_mappings().lock().unwrap();
        guard.as_mut().and_then(|m| m.remove(&key))
    };
    let Some(mapping) = mapping else { return };
    if let ChromiumMappingKind::Buffer { target, offset } = mapping.kind {
        if let Some(gl) = with_gl(mapping.ctx) {
            unsafe { gl.buffer_sub_data_u8_slice(target, offset as i32, &mapping.data) };
        }
    }
}

unsafe extern "C" fn map_tex_sub_image_2d(ctx: PP_Resource, target: GLenum, level: GLint, x: GLint, y: GLint, w: GLsizei, h: GLsizei, fmt: GLenum, type_: GLenum, _access: GLenum) -> *mut c_void {
    if w <= 0 || h <= 0 { return ptr::null_mut(); }
    let size = (w as usize) * (h as usize) * pixel_byte_size(fmt, type_);
    if size == 0 { return ptr::null_mut(); }
    let mut data = vec![0u8; size];
    let ptr = data.as_mut_ptr() as usize;
    let mapping = ChromiumMapping {
        ctx,
        kind: ChromiumMappingKind::Texture { target, level, x, y, w, h, format: fmt, type_ },
        data,
    };
    let mut guard = chromium_mappings().lock().unwrap();
    guard.get_or_insert_with(HashMap::new).insert(ptr, mapping);
    ptr as *mut c_void
}

unsafe extern "C" fn unmap_tex_sub_image_2d(_ctx: PP_Resource, mem: *const c_void) {
    if mem.is_null() { return; }
    let key = mem as usize;
    let mapping = {
        let mut guard = chromium_mappings().lock().unwrap();
        guard.as_mut().and_then(|m| m.remove(&key))
    };
    let Some(mapping) = mapping else { return };
    if let ChromiumMappingKind::Texture { target, level, x, y, w, h, format, type_ } = mapping.kind {
        if let Some(gl) = with_gl(mapping.ctx) {
            let pixel_data = glow::PixelUnpackData::Slice(Some(&mapping.data));
            unsafe { gl.tex_sub_image_2d(target, level, x, y, w, h, format, type_, pixel_data) };
        }
    }
}

static CHROMIUM_MAP_SUB_VTABLE: PPB_OpenGLES2ChromiumMapSub_1_0 = PPB_OpenGLES2ChromiumMapSub_1_0 {
    MapBufferSubDataCHROMIUM: Some(map_buffer_sub_data), UnmapBufferSubDataCHROMIUM: Some(unmap_buffer_sub_data),
    MapTexSubImage2DCHROMIUM: Some(map_tex_sub_image_2d), UnmapTexSubImage2DCHROMIUM: Some(unmap_tex_sub_image_2d),
};

unsafe extern "C" fn gen_queries(ctx: PP_Resource, n: GLsizei, q: *mut GLuint) {
    if let Some(gl) = with_gl(ctx) {
        let ids = std::slice::from_raw_parts_mut(q, n as usize);
        for id in ids.iter_mut() { *id = gl.create_query().map_or(0, |q| q.0.get()); }
    }
}
unsafe extern "C" fn delete_queries(ctx: PP_Resource, n: GLsizei, q: *const GLuint) {
    if let Some(gl) = with_gl(ctx) {
        let ids = std::slice::from_raw_parts(q, n as usize);
        for &id in ids { if let Some(nz) = NonZeroU32::new(id) { gl.delete_query(glow::NativeQuery(nz)); } }
    }
}
unsafe extern "C" fn is_query(ctx: PP_Resource, id: GLuint) -> GLboolean {
    // glow doesn't expose glIsQuery. Use raw GL.
    if with_gl(ctx).is_some() {
        type IsQueryFn = unsafe extern "system" fn(u32) -> u8;
        static IS_QUERY: std::sync::OnceLock<Option<IsQueryFn>> = std::sync::OnceLock::new();
        let f = IS_QUERY.get_or_init(|| {
            let p = gl_context::gl_proc_address(c"glIsQuery");
            if p.is_null() { None } else { Some(std::mem::transmute(p)) }
        });
        if let Some(func) = f { return func(id); }
    }
    0
}
unsafe extern "C" fn begin_query(ctx: PP_Resource, target: GLenum, id: GLuint) {
    if let Some(gl) = with_gl(ctx) { if let Some(nz) = NonZeroU32::new(id) { gl.begin_query(target, glow::NativeQuery(nz)); } }
}
unsafe extern "C" fn end_query(ctx: PP_Resource, target: GLenum) {
    if let Some(gl) = with_gl(ctx) { gl.end_query(target); }
}
unsafe extern "C" fn get_queryiv(ctx: PP_Resource, target: GLenum, pname: GLenum, p: *mut GLint) {
    // glGetQueryiv queries a target, not a query object - raw GL needed.
    if with_gl(ctx).is_some() {
        type GetQueryivFn = unsafe extern "system" fn(u32, u32, *mut i32);
        static GET_QUERYIV: std::sync::OnceLock<Option<GetQueryivFn>> = std::sync::OnceLock::new();
        let f = GET_QUERYIV.get_or_init(|| {
            let p = gl_context::gl_proc_address(c"glGetQueryiv");
            if p.is_null() { None } else { Some(std::mem::transmute(p)) }
        });
        if let Some(func) = f { func(target, pname, p); }
    }
}
unsafe extern "C" fn get_query_objectuiv(ctx: PP_Resource, id: GLuint, pname: GLenum, p: *mut GLuint) {
    if let Some(gl) = with_gl(ctx) {
        if let Some(nz) = NonZeroU32::new(id) {
            *p = gl.get_query_parameter_u32(glow::NativeQuery(nz), pname);
        }
    }
}
static QUERY_VTABLE: PPB_OpenGLES2Query_1_0 = PPB_OpenGLES2Query_1_0 {
    GenQueriesEXT: Some(gen_queries), DeleteQueriesEXT: Some(delete_queries), IsQueryEXT: Some(is_query),
    BeginQueryEXT: Some(begin_query), EndQueryEXT: Some(end_query), GetQueryivEXT: Some(get_queryiv), GetQueryObjectuivEXT: Some(get_query_objectuiv),
};

unsafe extern "C" fn gen_vertex_arrays(ctx: PP_Resource, n: GLsizei, a: *mut GLuint) {
    if let Some(gl) = with_gl(ctx) {
        let ids = std::slice::from_raw_parts_mut(a, n as usize);
        for id in ids.iter_mut() { *id = gl.create_vertex_array().map_or(0, |v| v.0.get()); }
    }
}
unsafe extern "C" fn delete_vertex_arrays(ctx: PP_Resource, n: GLsizei, a: *const GLuint) {
    if let Some(gl) = with_gl(ctx) {
        let ids = std::slice::from_raw_parts(a, n as usize);
        for &id in ids { if let Some(nz) = NonZeroU32::new(id) { gl.delete_vertex_array(glow::NativeVertexArray(nz)); } }
    }
}
unsafe extern "C" fn is_vertex_array(ctx: PP_Resource, a: GLuint) -> GLboolean {
    // glow doesn't expose glIsVertexArray. Use raw GL.
    if with_gl(ctx).is_some() {
        type IsVertexArrayFn = unsafe extern "system" fn(u32) -> u8;
        static IS_VERTEX_ARRAY: std::sync::OnceLock<Option<IsVertexArrayFn>> = std::sync::OnceLock::new();
        let f = IS_VERTEX_ARRAY.get_or_init(|| {
            let p = gl_context::gl_proc_address(c"glIsVertexArray");
            if p.is_null() { None } else { Some(std::mem::transmute(p)) }
        });
        if let Some(func) = f { return func(a); }
    }
    0
}
unsafe extern "C" fn bind_vertex_array(ctx: PP_Resource, a: GLuint) {
    if let Some(gl) = with_gl(ctx) { gl.bind_vertex_array(NonZeroU32::new(a).map(glow::NativeVertexArray)); }
}
static VAO_VTABLE: PPB_OpenGLES2VertexArrayObject_1_0 = PPB_OpenGLES2VertexArrayObject_1_0 {
    GenVertexArraysOES: Some(gen_vertex_arrays), DeleteVertexArraysOES: Some(delete_vertex_arrays),
    IsVertexArrayOES: Some(is_vertex_array), BindVertexArrayOES: Some(bind_vertex_array),
};

unsafe extern "C" fn draw_buffers_ext(ctx: PP_Resource, count: GLsizei, bufs: *const GLenum) {
    if let Some(gl) = with_gl(ctx) {
        let s = std::slice::from_raw_parts(bufs, count as usize);
        gl.draw_buffers(s);
    }
}
static DRAW_BUFFERS_VTABLE: PPB_OpenGLES2DrawBuffers_Dev_1_0 = PPB_OpenGLES2DrawBuffers_Dev_1_0 { DrawBuffersEXT: Some(draw_buffers_ext) };

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_OPENGLES2_INTERFACE_1_0, &VTABLE);
        registry.register(PPB_OPENGLES2_INSTANCEDARRAYS_INTERFACE_1_0, &INSTANCED_ARRAYS_VTABLE);
        registry.register(PPB_OPENGLES2_FRAMEBUFFERBLIT_INTERFACE_1_0, &FRAMEBUFFER_BLIT_VTABLE);
        registry.register(PPB_OPENGLES2_FRAMEBUFFERMULTISAMPLE_INTERFACE_1_0, &FRAMEBUFFER_MULTISAMPLE_VTABLE);
        registry.register(PPB_OPENGLES2_CHROMIUMENABLEFEATURE_INTERFACE_1_0, &CHROMIUM_ENABLE_VTABLE);
        registry.register(PPB_OPENGLES2_CHROMIUMMAPSUB_INTERFACE_1_0, &CHROMIUM_MAP_SUB_VTABLE);
        registry.register("PPB_OpenGLES2ChromiumMapSub(Dev);1.0\0", &CHROMIUM_MAP_SUB_VTABLE);
        registry.register("PPB_GLESChromiumTextureMapping(Dev);0.1\0", &CHROMIUM_MAP_SUB_VTABLE);
        registry.register(PPB_OPENGLES2_QUERY_INTERFACE_1_0, &QUERY_VTABLE);
        registry.register(PPB_OPENGLES2_VERTEXARRAYOBJECT_INTERFACE_1_0, &VAO_VTABLE);
        registry.register(PPB_OPENGLES2_DRAWBUFFERS_DEV_INTERFACE_1_0, &DRAW_BUFFERS_VTABLE);
    }
}
