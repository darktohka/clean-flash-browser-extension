//! PPB_OpenGLES2;1.0 and extension interfaces implementation.
//!
//! These provide the GL ES 2.0 function vtables that Flash queries at init.
//! Currently all functions are no-op stubs — real GL rendering would require
//! a PPB_Graphics3D context backed by an actual GL surface.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::{c_char, c_void};
use std::ptr;

// ---------------------------------------------------------------------------
// Stub helpers
// ---------------------------------------------------------------------------

/// Log at trace level when a GLES2 stub is called.
macro_rules! gles2_stub {
    ($name:expr) => {
        tracing::trace!(concat!("PPB_OpenGLES2::", $name, " (stub)"));
    };
}

// ---------------------------------------------------------------------------
// PPB_OpenGLES2;1.0 — 142 GL ES 2.0 function pointers
// ---------------------------------------------------------------------------

unsafe extern "C" fn active_texture(_ctx: PP_Resource, _texture: GLenum) { gles2_stub!("ActiveTexture"); }
unsafe extern "C" fn attach_shader(_ctx: PP_Resource, _program: GLuint, _shader: GLuint) { gles2_stub!("AttachShader"); }
unsafe extern "C" fn bind_attrib_location(_ctx: PP_Resource, _program: GLuint, _index: GLuint, _name: *const c_char) { gles2_stub!("BindAttribLocation"); }
unsafe extern "C" fn bind_buffer(_ctx: PP_Resource, _target: GLenum, _buffer: GLuint) { gles2_stub!("BindBuffer"); }
unsafe extern "C" fn bind_framebuffer(_ctx: PP_Resource, _target: GLenum, _fb: GLuint) { gles2_stub!("BindFramebuffer"); }
unsafe extern "C" fn bind_renderbuffer(_ctx: PP_Resource, _target: GLenum, _rb: GLuint) { gles2_stub!("BindRenderbuffer"); }
unsafe extern "C" fn bind_texture(_ctx: PP_Resource, _target: GLenum, _texture: GLuint) { gles2_stub!("BindTexture"); }
unsafe extern "C" fn blend_color(_ctx: PP_Resource, _r: GLclampf, _g: GLclampf, _b: GLclampf, _a: GLclampf) { gles2_stub!("BlendColor"); }
unsafe extern "C" fn blend_equation(_ctx: PP_Resource, _mode: GLenum) { gles2_stub!("BlendEquation"); }
unsafe extern "C" fn blend_equation_separate(_ctx: PP_Resource, _rgb: GLenum, _alpha: GLenum) { gles2_stub!("BlendEquationSeparate"); }
unsafe extern "C" fn blend_func(_ctx: PP_Resource, _sf: GLenum, _df: GLenum) { gles2_stub!("BlendFunc"); }
unsafe extern "C" fn blend_func_separate(_ctx: PP_Resource, _sr: GLenum, _dr: GLenum, _sa: GLenum, _da: GLenum) { gles2_stub!("BlendFuncSeparate"); }
unsafe extern "C" fn buffer_data(_ctx: PP_Resource, _target: GLenum, _size: GLsizeiptr, _data: *const c_void, _usage: GLenum) { gles2_stub!("BufferData"); }
unsafe extern "C" fn buffer_sub_data(_ctx: PP_Resource, _target: GLenum, _offset: GLintptr, _size: GLsizeiptr, _data: *const c_void) { gles2_stub!("BufferSubData"); }
unsafe extern "C" fn check_framebuffer_status(_ctx: PP_Resource, _target: GLenum) -> GLenum { gles2_stub!("CheckFramebufferStatus"); 0x8CD5 /* GL_FRAMEBUFFER_COMPLETE */ }
unsafe extern "C" fn clear(_ctx: PP_Resource, _mask: GLbitfield) { gles2_stub!("Clear"); }
unsafe extern "C" fn clear_color(_ctx: PP_Resource, _r: GLclampf, _g: GLclampf, _b: GLclampf, _a: GLclampf) { gles2_stub!("ClearColor"); }
unsafe extern "C" fn clear_depthf(_ctx: PP_Resource, _depth: GLclampf) { gles2_stub!("ClearDepthf"); }
unsafe extern "C" fn clear_stencil(_ctx: PP_Resource, _s: GLint) { gles2_stub!("ClearStencil"); }
unsafe extern "C" fn color_mask(_ctx: PP_Resource, _r: GLboolean, _g: GLboolean, _b: GLboolean, _a: GLboolean) { gles2_stub!("ColorMask"); }
unsafe extern "C" fn compile_shader(_ctx: PP_Resource, _shader: GLuint) { gles2_stub!("CompileShader"); }
unsafe extern "C" fn compressed_tex_image_2d(_ctx: PP_Resource, _target: GLenum, _level: GLint, _fmt: GLenum, _w: GLsizei, _h: GLsizei, _border: GLint, _size: GLsizei, _data: *const c_void) { gles2_stub!("CompressedTexImage2D"); }
unsafe extern "C" fn compressed_tex_sub_image_2d(_ctx: PP_Resource, _target: GLenum, _level: GLint, _x: GLint, _y: GLint, _w: GLsizei, _h: GLsizei, _fmt: GLenum, _size: GLsizei, _data: *const c_void) { gles2_stub!("CompressedTexSubImage2D"); }
unsafe extern "C" fn copy_tex_image_2d(_ctx: PP_Resource, _target: GLenum, _level: GLint, _fmt: GLenum, _x: GLint, _y: GLint, _w: GLsizei, _h: GLsizei, _border: GLint) { gles2_stub!("CopyTexImage2D"); }
unsafe extern "C" fn copy_tex_sub_image_2d(_ctx: PP_Resource, _target: GLenum, _level: GLint, _xoff: GLint, _yoff: GLint, _x: GLint, _y: GLint, _w: GLsizei, _h: GLsizei) { gles2_stub!("CopyTexSubImage2D"); }
unsafe extern "C" fn create_program(_ctx: PP_Resource) -> GLuint { gles2_stub!("CreateProgram"); 0 }
unsafe extern "C" fn create_shader(_ctx: PP_Resource, _type: GLenum) -> GLuint { gles2_stub!("CreateShader"); 0 }
unsafe extern "C" fn cull_face(_ctx: PP_Resource, _mode: GLenum) { gles2_stub!("CullFace"); }
unsafe extern "C" fn delete_buffers(_ctx: PP_Resource, _n: GLsizei, _bufs: *const GLuint) { gles2_stub!("DeleteBuffers"); }
unsafe extern "C" fn delete_framebuffers(_ctx: PP_Resource, _n: GLsizei, _fbs: *const GLuint) { gles2_stub!("DeleteFramebuffers"); }
unsafe extern "C" fn delete_program(_ctx: PP_Resource, _prog: GLuint) { gles2_stub!("DeleteProgram"); }
unsafe extern "C" fn delete_renderbuffers(_ctx: PP_Resource, _n: GLsizei, _rbs: *const GLuint) { gles2_stub!("DeleteRenderbuffers"); }
unsafe extern "C" fn delete_shader(_ctx: PP_Resource, _shader: GLuint) { gles2_stub!("DeleteShader"); }
unsafe extern "C" fn delete_textures(_ctx: PP_Resource, _n: GLsizei, _texs: *const GLuint) { gles2_stub!("DeleteTextures"); }
unsafe extern "C" fn depth_func(_ctx: PP_Resource, _func: GLenum) { gles2_stub!("DepthFunc"); }
unsafe extern "C" fn depth_mask(_ctx: PP_Resource, _flag: GLboolean) { gles2_stub!("DepthMask"); }
unsafe extern "C" fn depth_rangef(_ctx: PP_Resource, _near: GLclampf, _far: GLclampf) { gles2_stub!("DepthRangef"); }
unsafe extern "C" fn detach_shader(_ctx: PP_Resource, _prog: GLuint, _shader: GLuint) { gles2_stub!("DetachShader"); }
unsafe extern "C" fn disable(_ctx: PP_Resource, _cap: GLenum) { gles2_stub!("Disable"); }
unsafe extern "C" fn disable_vertex_attrib_array(_ctx: PP_Resource, _idx: GLuint) { gles2_stub!("DisableVertexAttribArray"); }
unsafe extern "C" fn draw_arrays(_ctx: PP_Resource, _mode: GLenum, _first: GLint, _count: GLsizei) { gles2_stub!("DrawArrays"); }
unsafe extern "C" fn draw_elements(_ctx: PP_Resource, _mode: GLenum, _count: GLsizei, _type: GLenum, _indices: *const c_void) { gles2_stub!("DrawElements"); }
unsafe extern "C" fn enable(_ctx: PP_Resource, _cap: GLenum) { gles2_stub!("Enable"); }
unsafe extern "C" fn enable_vertex_attrib_array(_ctx: PP_Resource, _idx: GLuint) { gles2_stub!("EnableVertexAttribArray"); }
unsafe extern "C" fn finish(_ctx: PP_Resource) { gles2_stub!("Finish"); }
unsafe extern "C" fn flush(_ctx: PP_Resource) { gles2_stub!("Flush"); }
unsafe extern "C" fn framebuffer_renderbuffer(_ctx: PP_Resource, _target: GLenum, _attachment: GLenum, _rbtarget: GLenum, _rb: GLuint) { gles2_stub!("FramebufferRenderbuffer"); }
unsafe extern "C" fn framebuffer_texture_2d(_ctx: PP_Resource, _target: GLenum, _attachment: GLenum, _textarget: GLenum, _texture: GLuint, _level: GLint) { gles2_stub!("FramebufferTexture2D"); }
unsafe extern "C" fn front_face(_ctx: PP_Resource, _mode: GLenum) { gles2_stub!("FrontFace"); }
unsafe extern "C" fn gen_buffers(_ctx: PP_Resource, _n: GLsizei, _bufs: *mut GLuint) { gles2_stub!("GenBuffers"); }
unsafe extern "C" fn generate_mipmap(_ctx: PP_Resource, _target: GLenum) { gles2_stub!("GenerateMipmap"); }
unsafe extern "C" fn gen_framebuffers(_ctx: PP_Resource, _n: GLsizei, _fbs: *mut GLuint) { gles2_stub!("GenFramebuffers"); }
unsafe extern "C" fn gen_renderbuffers(_ctx: PP_Resource, _n: GLsizei, _rbs: *mut GLuint) { gles2_stub!("GenRenderbuffers"); }
unsafe extern "C" fn gen_textures(_ctx: PP_Resource, _n: GLsizei, _texs: *mut GLuint) { gles2_stub!("GenTextures"); }
unsafe extern "C" fn get_active_attrib(_ctx: PP_Resource, _prog: GLuint, _idx: GLuint, _bufsize: GLsizei, _length: *mut GLsizei, _size: *mut GLint, _type: *mut GLenum, _name: *mut c_char) { gles2_stub!("GetActiveAttrib"); }
unsafe extern "C" fn get_active_uniform(_ctx: PP_Resource, _prog: GLuint, _idx: GLuint, _bufsize: GLsizei, _length: *mut GLsizei, _size: *mut GLint, _type: *mut GLenum, _name: *mut c_char) { gles2_stub!("GetActiveUniform"); }
unsafe extern "C" fn get_attached_shaders(_ctx: PP_Resource, _prog: GLuint, _max: GLsizei, _count: *mut GLsizei, _shaders: *mut GLuint) { gles2_stub!("GetAttachedShaders"); }
unsafe extern "C" fn get_attrib_location(_ctx: PP_Resource, _prog: GLuint, _name: *const c_char) -> GLint { gles2_stub!("GetAttribLocation"); -1 }
unsafe extern "C" fn get_booleanv(_ctx: PP_Resource, _pname: GLenum, _params: *mut GLboolean) { gles2_stub!("GetBooleanv"); }
unsafe extern "C" fn get_buffer_parameteriv(_ctx: PP_Resource, _target: GLenum, _pname: GLenum, _params: *mut GLint) { gles2_stub!("GetBufferParameteriv"); }
unsafe extern "C" fn get_error(_ctx: PP_Resource) -> GLenum { gles2_stub!("GetError"); 0 /* GL_NO_ERROR */ }
unsafe extern "C" fn get_floatv(_ctx: PP_Resource, _pname: GLenum, _params: *mut GLfloat) { gles2_stub!("GetFloatv"); }
unsafe extern "C" fn get_framebuffer_attachment_parameteriv(_ctx: PP_Resource, _target: GLenum, _attachment: GLenum, _pname: GLenum, _params: *mut GLint) { gles2_stub!("GetFramebufferAttachmentParameteriv"); }
unsafe extern "C" fn get_integerv(_ctx: PP_Resource, _pname: GLenum, _params: *mut GLint) { gles2_stub!("GetIntegerv"); }
unsafe extern "C" fn get_programiv(_ctx: PP_Resource, _prog: GLuint, _pname: GLenum, _params: *mut GLint) { gles2_stub!("GetProgramiv"); }
unsafe extern "C" fn get_program_info_log(_ctx: PP_Resource, _prog: GLuint, _bufsize: GLsizei, _length: *mut GLsizei, _infolog: *mut c_char) { gles2_stub!("GetProgramInfoLog"); }
unsafe extern "C" fn get_renderbuffer_parameteriv(_ctx: PP_Resource, _target: GLenum, _pname: GLenum, _params: *mut GLint) { gles2_stub!("GetRenderbufferParameteriv"); }
unsafe extern "C" fn get_shaderiv(_ctx: PP_Resource, _shader: GLuint, _pname: GLenum, _params: *mut GLint) { gles2_stub!("GetShaderiv"); }
unsafe extern "C" fn get_shader_info_log(_ctx: PP_Resource, _shader: GLuint, _bufsize: GLsizei, _length: *mut GLsizei, _infolog: *mut c_char) { gles2_stub!("GetShaderInfoLog"); }
unsafe extern "C" fn get_shader_precision_format(_ctx: PP_Resource, _shadertype: GLenum, _precisiontype: GLenum, _range: *mut GLint, _precision: *mut GLint) { gles2_stub!("GetShaderPrecisionFormat"); }
unsafe extern "C" fn get_shader_source(_ctx: PP_Resource, _shader: GLuint, _bufsize: GLsizei, _length: *mut GLsizei, _source: *mut c_char) { gles2_stub!("GetShaderSource"); }
unsafe extern "C" fn get_string(_ctx: PP_Resource, _name: GLenum) -> *const GLubyte { gles2_stub!("GetString"); b"\0".as_ptr() }
unsafe extern "C" fn get_tex_parameterfv(_ctx: PP_Resource, _target: GLenum, _pname: GLenum, _params: *mut GLfloat) { gles2_stub!("GetTexParameterfv"); }
unsafe extern "C" fn get_tex_parameteriv(_ctx: PP_Resource, _target: GLenum, _pname: GLenum, _params: *mut GLint) { gles2_stub!("GetTexParameteriv"); }
unsafe extern "C" fn get_uniformfv(_ctx: PP_Resource, _prog: GLuint, _loc: GLint, _params: *mut GLfloat) { gles2_stub!("GetUniformfv"); }
unsafe extern "C" fn get_uniformiv(_ctx: PP_Resource, _prog: GLuint, _loc: GLint, _params: *mut GLint) { gles2_stub!("GetUniformiv"); }
unsafe extern "C" fn get_uniform_location(_ctx: PP_Resource, _prog: GLuint, _name: *const c_char) -> GLint { gles2_stub!("GetUniformLocation"); -1 }
unsafe extern "C" fn get_vertex_attribfv(_ctx: PP_Resource, _idx: GLuint, _pname: GLenum, _params: *mut GLfloat) { gles2_stub!("GetVertexAttribfv"); }
unsafe extern "C" fn get_vertex_attribiv(_ctx: PP_Resource, _idx: GLuint, _pname: GLenum, _params: *mut GLint) { gles2_stub!("GetVertexAttribiv"); }
unsafe extern "C" fn get_vertex_attrib_pointerv(_ctx: PP_Resource, _idx: GLuint, _pname: GLenum, _ptr: *mut *mut c_void) { gles2_stub!("GetVertexAttribPointerv"); }
unsafe extern "C" fn hint(_ctx: PP_Resource, _target: GLenum, _mode: GLenum) { gles2_stub!("Hint"); }
unsafe extern "C" fn is_buffer(_ctx: PP_Resource, _buffer: GLuint) -> GLboolean { gles2_stub!("IsBuffer"); 0 }
unsafe extern "C" fn is_enabled(_ctx: PP_Resource, _cap: GLenum) -> GLboolean { gles2_stub!("IsEnabled"); 0 }
unsafe extern "C" fn is_framebuffer(_ctx: PP_Resource, _fb: GLuint) -> GLboolean { gles2_stub!("IsFramebuffer"); 0 }
unsafe extern "C" fn is_program(_ctx: PP_Resource, _prog: GLuint) -> GLboolean { gles2_stub!("IsProgram"); 0 }
unsafe extern "C" fn is_renderbuffer(_ctx: PP_Resource, _rb: GLuint) -> GLboolean { gles2_stub!("IsRenderbuffer"); 0 }
unsafe extern "C" fn is_shader(_ctx: PP_Resource, _shader: GLuint) -> GLboolean { gles2_stub!("IsShader"); 0 }
unsafe extern "C" fn is_texture(_ctx: PP_Resource, _tex: GLuint) -> GLboolean { gles2_stub!("IsTexture"); 0 }
unsafe extern "C" fn line_width(_ctx: PP_Resource, _width: GLfloat) { gles2_stub!("LineWidth"); }
unsafe extern "C" fn link_program(_ctx: PP_Resource, _prog: GLuint) { gles2_stub!("LinkProgram"); }
unsafe extern "C" fn pixel_storei(_ctx: PP_Resource, _pname: GLenum, _param: GLint) { gles2_stub!("PixelStorei"); }
unsafe extern "C" fn polygon_offset(_ctx: PP_Resource, _factor: GLfloat, _units: GLfloat) { gles2_stub!("PolygonOffset"); }
unsafe extern "C" fn read_pixels(_ctx: PP_Resource, _x: GLint, _y: GLint, _w: GLsizei, _h: GLsizei, _fmt: GLenum, _type: GLenum, _pixels: *mut c_void) { gles2_stub!("ReadPixels"); }
unsafe extern "C" fn release_shader_compiler(_ctx: PP_Resource) { gles2_stub!("ReleaseShaderCompiler"); }
unsafe extern "C" fn renderbuffer_storage(_ctx: PP_Resource, _target: GLenum, _fmt: GLenum, _w: GLsizei, _h: GLsizei) { gles2_stub!("RenderbufferStorage"); }
unsafe extern "C" fn sample_coverage(_ctx: PP_Resource, _value: GLclampf, _invert: GLboolean) { gles2_stub!("SampleCoverage"); }
unsafe extern "C" fn scissor(_ctx: PP_Resource, _x: GLint, _y: GLint, _w: GLsizei, _h: GLsizei) { gles2_stub!("Scissor"); }
unsafe extern "C" fn shader_binary(_ctx: PP_Resource, _n: GLsizei, _shaders: *const GLuint, _fmt: GLenum, _binary: *const c_void, _length: GLsizei) { gles2_stub!("ShaderBinary"); }
unsafe extern "C" fn shader_source(_ctx: PP_Resource, _shader: GLuint, _count: GLsizei, _str: *const *const c_char, _length: *const GLint) { gles2_stub!("ShaderSource"); }
unsafe extern "C" fn stencil_func(_ctx: PP_Resource, _func: GLenum, _ref_: GLint, _mask: GLuint) { gles2_stub!("StencilFunc"); }
unsafe extern "C" fn stencil_func_separate(_ctx: PP_Resource, _face: GLenum, _func: GLenum, _ref_: GLint, _mask: GLuint) { gles2_stub!("StencilFuncSeparate"); }
unsafe extern "C" fn stencil_mask(_ctx: PP_Resource, _mask: GLuint) { gles2_stub!("StencilMask"); }
unsafe extern "C" fn stencil_mask_separate(_ctx: PP_Resource, _face: GLenum, _mask: GLuint) { gles2_stub!("StencilMaskSeparate"); }
unsafe extern "C" fn stencil_op(_ctx: PP_Resource, _fail: GLenum, _zfail: GLenum, _zpass: GLenum) { gles2_stub!("StencilOp"); }
unsafe extern "C" fn stencil_op_separate(_ctx: PP_Resource, _face: GLenum, _fail: GLenum, _zfail: GLenum, _zpass: GLenum) { gles2_stub!("StencilOpSeparate"); }
unsafe extern "C" fn tex_image_2d(_ctx: PP_Resource, _target: GLenum, _level: GLint, _ifmt: GLint, _w: GLsizei, _h: GLsizei, _border: GLint, _fmt: GLenum, _type: GLenum, _pixels: *const c_void) { gles2_stub!("TexImage2D"); }
unsafe extern "C" fn tex_parameterf(_ctx: PP_Resource, _target: GLenum, _pname: GLenum, _param: GLfloat) { gles2_stub!("TexParameterf"); }
unsafe extern "C" fn tex_parameterfv(_ctx: PP_Resource, _target: GLenum, _pname: GLenum, _params: *const GLfloat) { gles2_stub!("TexParameterfv"); }
unsafe extern "C" fn tex_parameteri(_ctx: PP_Resource, _target: GLenum, _pname: GLenum, _param: GLint) { gles2_stub!("TexParameteri"); }
unsafe extern "C" fn tex_parameteriv(_ctx: PP_Resource, _target: GLenum, _pname: GLenum, _params: *const GLint) { gles2_stub!("TexParameteriv"); }
unsafe extern "C" fn tex_sub_image_2d(_ctx: PP_Resource, _target: GLenum, _level: GLint, _x: GLint, _y: GLint, _w: GLsizei, _h: GLsizei, _fmt: GLenum, _type: GLenum, _pixels: *const c_void) { gles2_stub!("TexSubImage2D"); }
unsafe extern "C" fn uniform1f(_ctx: PP_Resource, _loc: GLint, _x: GLfloat) { gles2_stub!("Uniform1f"); }
unsafe extern "C" fn uniform1fv(_ctx: PP_Resource, _loc: GLint, _count: GLsizei, _v: *const GLfloat) { gles2_stub!("Uniform1fv"); }
unsafe extern "C" fn uniform1i(_ctx: PP_Resource, _loc: GLint, _x: GLint) { gles2_stub!("Uniform1i"); }
unsafe extern "C" fn uniform1iv(_ctx: PP_Resource, _loc: GLint, _count: GLsizei, _v: *const GLint) { gles2_stub!("Uniform1iv"); }
unsafe extern "C" fn uniform2f(_ctx: PP_Resource, _loc: GLint, _x: GLfloat, _y: GLfloat) { gles2_stub!("Uniform2f"); }
unsafe extern "C" fn uniform2fv(_ctx: PP_Resource, _loc: GLint, _count: GLsizei, _v: *const GLfloat) { gles2_stub!("Uniform2fv"); }
unsafe extern "C" fn uniform2i(_ctx: PP_Resource, _loc: GLint, _x: GLint, _y: GLint) { gles2_stub!("Uniform2i"); }
unsafe extern "C" fn uniform2iv(_ctx: PP_Resource, _loc: GLint, _count: GLsizei, _v: *const GLint) { gles2_stub!("Uniform2iv"); }
unsafe extern "C" fn uniform3f(_ctx: PP_Resource, _loc: GLint, _x: GLfloat, _y: GLfloat, _z: GLfloat) { gles2_stub!("Uniform3f"); }
unsafe extern "C" fn uniform3fv(_ctx: PP_Resource, _loc: GLint, _count: GLsizei, _v: *const GLfloat) { gles2_stub!("Uniform3fv"); }
unsafe extern "C" fn uniform3i(_ctx: PP_Resource, _loc: GLint, _x: GLint, _y: GLint, _z: GLint) { gles2_stub!("Uniform3i"); }
unsafe extern "C" fn uniform3iv(_ctx: PP_Resource, _loc: GLint, _count: GLsizei, _v: *const GLint) { gles2_stub!("Uniform3iv"); }
unsafe extern "C" fn uniform4f(_ctx: PP_Resource, _loc: GLint, _x: GLfloat, _y: GLfloat, _z: GLfloat, _w: GLfloat) { gles2_stub!("Uniform4f"); }
unsafe extern "C" fn uniform4fv(_ctx: PP_Resource, _loc: GLint, _count: GLsizei, _v: *const GLfloat) { gles2_stub!("Uniform4fv"); }
unsafe extern "C" fn uniform4i(_ctx: PP_Resource, _loc: GLint, _x: GLint, _y: GLint, _z: GLint, _w: GLint) { gles2_stub!("Uniform4i"); }
unsafe extern "C" fn uniform4iv(_ctx: PP_Resource, _loc: GLint, _count: GLsizei, _v: *const GLint) { gles2_stub!("Uniform4iv"); }
unsafe extern "C" fn uniform_matrix2fv(_ctx: PP_Resource, _loc: GLint, _count: GLsizei, _transpose: GLboolean, _value: *const GLfloat) { gles2_stub!("UniformMatrix2fv"); }
unsafe extern "C" fn uniform_matrix3fv(_ctx: PP_Resource, _loc: GLint, _count: GLsizei, _transpose: GLboolean, _value: *const GLfloat) { gles2_stub!("UniformMatrix3fv"); }
unsafe extern "C" fn uniform_matrix4fv(_ctx: PP_Resource, _loc: GLint, _count: GLsizei, _transpose: GLboolean, _value: *const GLfloat) { gles2_stub!("UniformMatrix4fv"); }
unsafe extern "C" fn use_program(_ctx: PP_Resource, _prog: GLuint) { gles2_stub!("UseProgram"); }
unsafe extern "C" fn validate_program(_ctx: PP_Resource, _prog: GLuint) { gles2_stub!("ValidateProgram"); }
unsafe extern "C" fn vertex_attrib1f(_ctx: PP_Resource, _idx: GLuint, _x: GLfloat) { gles2_stub!("VertexAttrib1f"); }
unsafe extern "C" fn vertex_attrib1fv(_ctx: PP_Resource, _idx: GLuint, _values: *const GLfloat) { gles2_stub!("VertexAttrib1fv"); }
unsafe extern "C" fn vertex_attrib2f(_ctx: PP_Resource, _idx: GLuint, _x: GLfloat, _y: GLfloat) { gles2_stub!("VertexAttrib2f"); }
unsafe extern "C" fn vertex_attrib2fv(_ctx: PP_Resource, _idx: GLuint, _values: *const GLfloat) { gles2_stub!("VertexAttrib2fv"); }
unsafe extern "C" fn vertex_attrib3f(_ctx: PP_Resource, _idx: GLuint, _x: GLfloat, _y: GLfloat, _z: GLfloat) { gles2_stub!("VertexAttrib3f"); }
unsafe extern "C" fn vertex_attrib3fv(_ctx: PP_Resource, _idx: GLuint, _values: *const GLfloat) { gles2_stub!("VertexAttrib3fv"); }
unsafe extern "C" fn vertex_attrib4f(_ctx: PP_Resource, _idx: GLuint, _x: GLfloat, _y: GLfloat, _z: GLfloat, _w: GLfloat) { gles2_stub!("VertexAttrib4f"); }
unsafe extern "C" fn vertex_attrib4fv(_ctx: PP_Resource, _idx: GLuint, _values: *const GLfloat) { gles2_stub!("VertexAttrib4fv"); }
unsafe extern "C" fn vertex_attrib_pointer(_ctx: PP_Resource, _idx: GLuint, _size: GLint, _type: GLenum, _normalized: GLboolean, _stride: GLsizei, _ptr: *const c_void) { gles2_stub!("VertexAttribPointer"); }
unsafe extern "C" fn viewport(_ctx: PP_Resource, _x: GLint, _y: GLint, _w: GLsizei, _h: GLsizei) { gles2_stub!("Viewport"); }

static VTABLE: PPB_OpenGLES2_1_0 = PPB_OpenGLES2_1_0 {
    ActiveTexture: Some(active_texture),
    AttachShader: Some(attach_shader),
    BindAttribLocation: Some(bind_attrib_location),
    BindBuffer: Some(bind_buffer),
    BindFramebuffer: Some(bind_framebuffer),
    BindRenderbuffer: Some(bind_renderbuffer),
    BindTexture: Some(bind_texture),
    BlendColor: Some(blend_color),
    BlendEquation: Some(blend_equation),
    BlendEquationSeparate: Some(blend_equation_separate),
    BlendFunc: Some(blend_func),
    BlendFuncSeparate: Some(blend_func_separate),
    BufferData: Some(buffer_data),
    BufferSubData: Some(buffer_sub_data),
    CheckFramebufferStatus: Some(check_framebuffer_status),
    Clear: Some(clear),
    ClearColor: Some(clear_color),
    ClearDepthf: Some(clear_depthf),
    ClearStencil: Some(clear_stencil),
    ColorMask: Some(color_mask),
    CompileShader: Some(compile_shader),
    CompressedTexImage2D: Some(compressed_tex_image_2d),
    CompressedTexSubImage2D: Some(compressed_tex_sub_image_2d),
    CopyTexImage2D: Some(copy_tex_image_2d),
    CopyTexSubImage2D: Some(copy_tex_sub_image_2d),
    CreateProgram: Some(create_program),
    CreateShader: Some(create_shader),
    CullFace: Some(cull_face),
    DeleteBuffers: Some(delete_buffers),
    DeleteFramebuffers: Some(delete_framebuffers),
    DeleteProgram: Some(delete_program),
    DeleteRenderbuffers: Some(delete_renderbuffers),
    DeleteShader: Some(delete_shader),
    DeleteTextures: Some(delete_textures),
    DepthFunc: Some(depth_func),
    DepthMask: Some(depth_mask),
    DepthRangef: Some(depth_rangef),
    DetachShader: Some(detach_shader),
    Disable: Some(disable),
    DisableVertexAttribArray: Some(disable_vertex_attrib_array),
    DrawArrays: Some(draw_arrays),
    DrawElements: Some(draw_elements),
    Enable: Some(enable),
    EnableVertexAttribArray: Some(enable_vertex_attrib_array),
    Finish: Some(finish),
    Flush: Some(flush),
    FramebufferRenderbuffer: Some(framebuffer_renderbuffer),
    FramebufferTexture2D: Some(framebuffer_texture_2d),
    FrontFace: Some(front_face),
    GenBuffers: Some(gen_buffers),
    GenerateMipmap: Some(generate_mipmap),
    GenFramebuffers: Some(gen_framebuffers),
    GenRenderbuffers: Some(gen_renderbuffers),
    GenTextures: Some(gen_textures),
    GetActiveAttrib: Some(get_active_attrib),
    GetActiveUniform: Some(get_active_uniform),
    GetAttachedShaders: Some(get_attached_shaders),
    GetAttribLocation: Some(get_attrib_location),
    GetBooleanv: Some(get_booleanv),
    GetBufferParameteriv: Some(get_buffer_parameteriv),
    GetError: Some(get_error),
    GetFloatv: Some(get_floatv),
    GetFramebufferAttachmentParameteriv: Some(get_framebuffer_attachment_parameteriv),
    GetIntegerv: Some(get_integerv),
    GetProgramiv: Some(get_programiv),
    GetProgramInfoLog: Some(get_program_info_log),
    GetRenderbufferParameteriv: Some(get_renderbuffer_parameteriv),
    GetShaderiv: Some(get_shaderiv),
    GetShaderInfoLog: Some(get_shader_info_log),
    GetShaderPrecisionFormat: Some(get_shader_precision_format),
    GetShaderSource: Some(get_shader_source),
    GetString: Some(get_string),
    GetTexParameterfv: Some(get_tex_parameterfv),
    GetTexParameteriv: Some(get_tex_parameteriv),
    GetUniformfv: Some(get_uniformfv),
    GetUniformiv: Some(get_uniformiv),
    GetUniformLocation: Some(get_uniform_location),
    GetVertexAttribfv: Some(get_vertex_attribfv),
    GetVertexAttribiv: Some(get_vertex_attribiv),
    GetVertexAttribPointerv: Some(get_vertex_attrib_pointerv),
    Hint: Some(hint),
    IsBuffer: Some(is_buffer),
    IsEnabled: Some(is_enabled),
    IsFramebuffer: Some(is_framebuffer),
    IsProgram: Some(is_program),
    IsRenderbuffer: Some(is_renderbuffer),
    IsShader: Some(is_shader),
    IsTexture: Some(is_texture),
    LineWidth: Some(line_width),
    LinkProgram: Some(link_program),
    PixelStorei: Some(pixel_storei),
    PolygonOffset: Some(polygon_offset),
    ReadPixels: Some(read_pixels),
    ReleaseShaderCompiler: Some(release_shader_compiler),
    RenderbufferStorage: Some(renderbuffer_storage),
    SampleCoverage: Some(sample_coverage),
    Scissor: Some(scissor),
    ShaderBinary: Some(shader_binary),
    ShaderSource: Some(shader_source),
    StencilFunc: Some(stencil_func),
    StencilFuncSeparate: Some(stencil_func_separate),
    StencilMask: Some(stencil_mask),
    StencilMaskSeparate: Some(stencil_mask_separate),
    StencilOp: Some(stencil_op),
    StencilOpSeparate: Some(stencil_op_separate),
    TexImage2D: Some(tex_image_2d),
    TexParameterf: Some(tex_parameterf),
    TexParameterfv: Some(tex_parameterfv),
    TexParameteri: Some(tex_parameteri),
    TexParameteriv: Some(tex_parameteriv),
    TexSubImage2D: Some(tex_sub_image_2d),
    Uniform1f: Some(uniform1f),
    Uniform1fv: Some(uniform1fv),
    Uniform1i: Some(uniform1i),
    Uniform1iv: Some(uniform1iv),
    Uniform2f: Some(uniform2f),
    Uniform2fv: Some(uniform2fv),
    Uniform2i: Some(uniform2i),
    Uniform2iv: Some(uniform2iv),
    Uniform3f: Some(uniform3f),
    Uniform3fv: Some(uniform3fv),
    Uniform3i: Some(uniform3i),
    Uniform3iv: Some(uniform3iv),
    Uniform4f: Some(uniform4f),
    Uniform4fv: Some(uniform4fv),
    Uniform4i: Some(uniform4i),
    Uniform4iv: Some(uniform4iv),
    UniformMatrix2fv: Some(uniform_matrix2fv),
    UniformMatrix3fv: Some(uniform_matrix3fv),
    UniformMatrix4fv: Some(uniform_matrix4fv),
    UseProgram: Some(use_program),
    ValidateProgram: Some(validate_program),
    VertexAttrib1f: Some(vertex_attrib1f),
    VertexAttrib1fv: Some(vertex_attrib1fv),
    VertexAttrib2f: Some(vertex_attrib2f),
    VertexAttrib2fv: Some(vertex_attrib2fv),
    VertexAttrib3f: Some(vertex_attrib3f),
    VertexAttrib3fv: Some(vertex_attrib3fv),
    VertexAttrib4f: Some(vertex_attrib4f),
    VertexAttrib4fv: Some(vertex_attrib4fv),
    VertexAttribPointer: Some(vertex_attrib_pointer),
    Viewport: Some(viewport),
};

// ---------------------------------------------------------------------------
// PPB_OpenGLES2InstancedArrays;1.0
// ---------------------------------------------------------------------------

unsafe extern "C" fn draw_arrays_instanced(_ctx: PP_Resource, _mode: GLenum, _first: GLint, _count: GLsizei, _primcount: GLsizei) { gles2_stub!("DrawArraysInstancedANGLE"); }
unsafe extern "C" fn draw_elements_instanced(_ctx: PP_Resource, _mode: GLenum, _count: GLsizei, _type: GLenum, _indices: *const c_void, _primcount: GLsizei) { gles2_stub!("DrawElementsInstancedANGLE"); }
unsafe extern "C" fn vertex_attrib_divisor(_ctx: PP_Resource, _index: GLuint, _divisor: GLuint) { gles2_stub!("VertexAttribDivisorANGLE"); }

static INSTANCED_ARRAYS_VTABLE: PPB_OpenGLES2InstancedArrays_1_0 = PPB_OpenGLES2InstancedArrays_1_0 {
    DrawArraysInstancedANGLE: Some(draw_arrays_instanced),
    DrawElementsInstancedANGLE: Some(draw_elements_instanced),
    VertexAttribDivisorANGLE: Some(vertex_attrib_divisor),
};

// ---------------------------------------------------------------------------
// PPB_OpenGLES2FramebufferBlit;1.0
// ---------------------------------------------------------------------------

unsafe extern "C" fn blit_framebuffer(_ctx: PP_Resource, _sx0: GLint, _sy0: GLint, _sx1: GLint, _sy1: GLint, _dx0: GLint, _dy0: GLint, _dx1: GLint, _dy1: GLint, _mask: GLbitfield, _filter: GLenum) { gles2_stub!("BlitFramebufferEXT"); }

static FRAMEBUFFER_BLIT_VTABLE: PPB_OpenGLES2FramebufferBlit_1_0 = PPB_OpenGLES2FramebufferBlit_1_0 {
    BlitFramebufferEXT: Some(blit_framebuffer),
};

// ---------------------------------------------------------------------------
// PPB_OpenGLES2FramebufferMultisample;1.0
// ---------------------------------------------------------------------------

unsafe extern "C" fn renderbuffer_storage_multisample(_ctx: PP_Resource, _target: GLenum, _samples: GLsizei, _fmt: GLenum, _w: GLsizei, _h: GLsizei) { gles2_stub!("RenderbufferStorageMultisampleEXT"); }

static FRAMEBUFFER_MULTISAMPLE_VTABLE: PPB_OpenGLES2FramebufferMultisample_1_0 = PPB_OpenGLES2FramebufferMultisample_1_0 {
    RenderbufferStorageMultisampleEXT: Some(renderbuffer_storage_multisample),
};

// ---------------------------------------------------------------------------
// PPB_OpenGLES2ChromiumEnableFeature;1.0
// ---------------------------------------------------------------------------

unsafe extern "C" fn enable_feature_chromium(_ctx: PP_Resource, _feature: *const c_char) -> GLboolean { gles2_stub!("EnableFeatureCHROMIUM"); 0 }

static CHROMIUM_ENABLE_VTABLE: PPB_OpenGLES2ChromiumEnableFeature_1_0 = PPB_OpenGLES2ChromiumEnableFeature_1_0 {
    EnableFeatureCHROMIUM: Some(enable_feature_chromium),
};

// ---------------------------------------------------------------------------
// PPB_OpenGLES2ChromiumMapSub;1.0
// ---------------------------------------------------------------------------

unsafe extern "C" fn map_buffer_sub_data(_ctx: PP_Resource, _target: GLuint, _offset: GLintptr, _size: GLsizeiptr, _access: GLenum) -> *mut c_void { gles2_stub!("MapBufferSubDataCHROMIUM"); ptr::null_mut() }
unsafe extern "C" fn unmap_buffer_sub_data(_ctx: PP_Resource, _mem: *const c_void) { gles2_stub!("UnmapBufferSubDataCHROMIUM"); }
unsafe extern "C" fn map_tex_sub_image_2d(_ctx: PP_Resource, _target: GLenum, _level: GLint, _x: GLint, _y: GLint, _w: GLsizei, _h: GLsizei, _fmt: GLenum, _type: GLenum, _access: GLenum) -> *mut c_void { gles2_stub!("MapTexSubImage2DCHROMIUM"); ptr::null_mut() }
unsafe extern "C" fn unmap_tex_sub_image_2d(_ctx: PP_Resource, _mem: *const c_void) { gles2_stub!("UnmapTexSubImage2DCHROMIUM"); }

static CHROMIUM_MAP_SUB_VTABLE: PPB_OpenGLES2ChromiumMapSub_1_0 = PPB_OpenGLES2ChromiumMapSub_1_0 {
    MapBufferSubDataCHROMIUM: Some(map_buffer_sub_data),
    UnmapBufferSubDataCHROMIUM: Some(unmap_buffer_sub_data),
    MapTexSubImage2DCHROMIUM: Some(map_tex_sub_image_2d),
    UnmapTexSubImage2DCHROMIUM: Some(unmap_tex_sub_image_2d),
};

// ---------------------------------------------------------------------------
// PPB_OpenGLES2Query;1.0
// ---------------------------------------------------------------------------

unsafe extern "C" fn gen_queries(_ctx: PP_Resource, _n: GLsizei, _queries: *mut GLuint) { gles2_stub!("GenQueriesEXT"); }
unsafe extern "C" fn delete_queries(_ctx: PP_Resource, _n: GLsizei, _queries: *const GLuint) { gles2_stub!("DeleteQueriesEXT"); }
unsafe extern "C" fn is_query(_ctx: PP_Resource, _id: GLuint) -> GLboolean { gles2_stub!("IsQueryEXT"); 0 }
unsafe extern "C" fn begin_query(_ctx: PP_Resource, _target: GLenum, _id: GLuint) { gles2_stub!("BeginQueryEXT"); }
unsafe extern "C" fn end_query(_ctx: PP_Resource, _target: GLenum) { gles2_stub!("EndQueryEXT"); }
unsafe extern "C" fn get_queryiv(_ctx: PP_Resource, _target: GLenum, _pname: GLenum, _params: *mut GLint) { gles2_stub!("GetQueryivEXT"); }
unsafe extern "C" fn get_query_objectuiv(_ctx: PP_Resource, _id: GLuint, _pname: GLenum, _params: *mut GLuint) { gles2_stub!("GetQueryObjectuivEXT"); }

static QUERY_VTABLE: PPB_OpenGLES2Query_1_0 = PPB_OpenGLES2Query_1_0 {
    GenQueriesEXT: Some(gen_queries),
    DeleteQueriesEXT: Some(delete_queries),
    IsQueryEXT: Some(is_query),
    BeginQueryEXT: Some(begin_query),
    EndQueryEXT: Some(end_query),
    GetQueryivEXT: Some(get_queryiv),
    GetQueryObjectuivEXT: Some(get_query_objectuiv),
};

// ---------------------------------------------------------------------------
// PPB_OpenGLES2VertexArrayObject;1.0
// ---------------------------------------------------------------------------

unsafe extern "C" fn gen_vertex_arrays(_ctx: PP_Resource, _n: GLsizei, _arrays: *mut GLuint) { gles2_stub!("GenVertexArraysOES"); }
unsafe extern "C" fn delete_vertex_arrays(_ctx: PP_Resource, _n: GLsizei, _arrays: *const GLuint) { gles2_stub!("DeleteVertexArraysOES"); }
unsafe extern "C" fn is_vertex_array(_ctx: PP_Resource, _array: GLuint) -> GLboolean { gles2_stub!("IsVertexArrayOES"); 0 }
unsafe extern "C" fn bind_vertex_array(_ctx: PP_Resource, _array: GLuint) { gles2_stub!("BindVertexArrayOES"); }

static VAO_VTABLE: PPB_OpenGLES2VertexArrayObject_1_0 = PPB_OpenGLES2VertexArrayObject_1_0 {
    GenVertexArraysOES: Some(gen_vertex_arrays),
    DeleteVertexArraysOES: Some(delete_vertex_arrays),
    IsVertexArrayOES: Some(is_vertex_array),
    BindVertexArrayOES: Some(bind_vertex_array),
};

// ---------------------------------------------------------------------------
// PPB_OpenGLES2DrawBuffers(Dev);1.0
// ---------------------------------------------------------------------------

unsafe extern "C" fn draw_buffers(_ctx: PP_Resource, _count: GLsizei, _bufs: *const GLenum) { gles2_stub!("DrawBuffersEXT"); }

static DRAW_BUFFERS_VTABLE: PPB_OpenGLES2DrawBuffers_Dev_1_0 = PPB_OpenGLES2DrawBuffers_Dev_1_0 {
    DrawBuffersEXT: Some(draw_buffers),
};

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
        registry.register(PPB_OPENGLES2_QUERY_INTERFACE_1_0, &QUERY_VTABLE);
        registry.register(PPB_OPENGLES2_VERTEXARRAYOBJECT_INTERFACE_1_0, &VAO_VTABLE);
        registry.register(PPB_OPENGLES2_DRAWBUFFERS_DEV_INTERFACE_1_0, &DRAW_BUFFERS_VTABLE);
    }
}
