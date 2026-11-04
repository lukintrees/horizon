//! Private desktop GL2 ABI. No entry points newer than OpenGL 2.0.
use crate::Context;
use std::ffi::{c_char, c_void};
use std::ops::Deref;
use std::rc::Rc;

pub(crate) enum ContextHandle<'a> {
    Borrowed(&'a Context),
    Owned(Rc<Context>),
}
impl Deref for ContextHandle<'_> {
    type Target = Context;
    fn deref(&self) -> &Context {
        match self {
            Self::Borrowed(context) => context,
            Self::Owned(context) => context,
        }
    }
}
macro_rules! entry_points {
    ($($field:ident: $name:literal ($($arg:ty),*) -> $ret:ty;)*) => {
        pub(crate) struct Gl<'a> {
            pub context: ContextHandle<'a>,
            $ (pub $field: unsafe extern "system" fn($($arg),*) -> $ret,)*
        }
        impl<'a> Gl<'a> {
            pub fn load(context: ContextHandle<'a>) -> Result<Self, String> {
                Ok(Self {
                    $($field: {
                        let address = context.address($name)?;
                        // SAFETY: each fixed symbol has the declared desktop GL ABI.
                        unsafe { std::mem::transmute::<*const c_void, unsafe extern "system" fn($($arg),*) -> $ret>(address) }
                    },)*
                    context,
                })
            }
        }
    }
}
entry_points! {
    get_integer: c"glGetIntegerv" (u32, *mut i32) -> ();
    get_error: c"glGetError" () -> u32;
    create_shader: c"glCreateShader" (u32) -> u32;
    shader_source: c"glShaderSource" (u32, i32, *const *const c_char, *const i32) -> ();
    compile_shader: c"glCompileShader" (u32) -> ();
    shader_integer: c"glGetShaderiv" (u32, u32, *mut i32) -> ();
    shader_log: c"glGetShaderInfoLog" (u32, i32, *mut i32, *mut c_char) -> ();
    delete_shader: c"glDeleteShader" (u32) -> ();
    create_program: c"glCreateProgram" () -> u32;
    attach_shader: c"glAttachShader" (u32, u32) -> ();
    bind_attribute: c"glBindAttribLocation" (u32, u32, *const c_char) -> ();
    link_program: c"glLinkProgram" (u32) -> ();
    program_integer: c"glGetProgramiv" (u32, u32, *mut i32) -> ();
    program_log: c"glGetProgramInfoLog" (u32, i32, *mut i32, *mut c_char) -> ();
    use_program: c"glUseProgram" (u32) -> ();
    uniform_location: c"glGetUniformLocation" (u32, *const c_char) -> i32;
    uniform1f: c"glUniform1f" (i32, f32) -> ();
    uniform4f: c"glUniform4f" (i32, f32, f32, f32, f32) -> ();
    gen_buffers: c"glGenBuffers" (i32, *mut u32) -> ();
    bind_buffer: c"glBindBuffer" (u32, u32) -> ();
    buffer_data: c"glBufferData" (u32, isize, *const c_void, u32) -> ();
    attribute_pointer: c"glVertexAttribPointer" (u32, i32, u32, u8, i32, *const c_void) -> ();
    enable_attribute: c"glEnableVertexAttribArray" (u32) -> ();
    viewport: c"glViewport" (i32, i32, i32, i32) -> ();
    clear_color: c"glClearColor" (f32, f32, f32, f32) -> ();
    clear: c"glClear" (u32) -> ();
    enable: c"glEnable" (u32) -> ();
    draw_elements: c"glDrawElements" (u32, i32, u32, *const c_void) -> ();
    active_texture: c"glActiveTexture" (u32) -> ();
    gen_textures: c"glGenTextures" (i32, *mut u32) -> ();
    bind_texture: c"glBindTexture" (u32, u32) -> ();
    tex_parameter: c"glTexParameteri" (u32, u32, i32) -> ();
    tex_image: c"glTexImage2D" (u32, i32, i32, i32, i32, i32, u32, u32, *const c_void) -> ();
    tex_update: c"glTexSubImage2D" (u32, i32, i32, i32, i32, i32, u32, u32, *const c_void) -> ();
    delete_textures: c"glDeleteTextures" (i32, *const u32) -> ();
    delete_buffers: c"glDeleteBuffers" (i32, *const u32) -> ();
    delete_program: c"glDeleteProgram" (u32) -> ();
    uniform1i: c"glUniform1i" (i32, i32) -> ();
    uniform3f: c"glUniform3f" (i32, f32, f32, f32) -> ();
    uniform_matrix: c"glUniformMatrix4fv" (i32, i32, u8, *const f32) -> ();
    attribute4f: c"glVertexAttrib4f" (u32, f32, f32, f32, f32) -> ();
    disable_attribute: c"glDisableVertexAttribArray" (u32) -> ();
    cull_face: c"glCullFace" (u32) -> ();
    front_face: c"glFrontFace" (u32) -> ();
    disable: c"glDisable" (u32) -> ();
    color_mask: c"glColorMask" (u8, u8, u8, u8) -> ();
    polygon_offset: c"glPolygonOffset" (f32, f32) -> ();
    depth_mask: c"glDepthMask" (u8) -> ();
    depth_func: c"glDepthFunc" (u32) -> ();
    blend_func: c"glBlendFuncSeparate" (u32, u32, u32, u32) -> ();
    scissor: c"glScissor" (i32, i32, i32, i32) -> ();
    pixel_store: c"glPixelStorei" (u32, i32) -> ();
    read_pixels: c"glReadPixels" (i32, i32, i32, i32, u32, u32, *mut c_void) -> ();
}
