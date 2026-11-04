//! Experimental OpenGL 2.1 terrain renderer. This crate does not create a
//! window or a context and is not yet wired into the Horizon game loop.
//!
//! All GL operations are unsafe: the caller must keep the same compatible
//! context current on the calling thread, load valid context-specific entry
//! points, and explicitly destroy resources before the context is destroyed.
//! This isolates the GL FFI from the safe CPU mesher and the wgpu renderer.
use std::{
    ffi::{CStr, c_char, c_void},
    marker::PhantomData,
    mem,
    rc::Rc,
};

pub const TERRAIN_VERTEX_SHADER: &str = include_str!("terrain.vert");
pub const TERRAIN_FRAGMENT_SHADER: &str = include_str!("terrain.frag");

/// Interleaved GLSL attributes. Positions are relative to a section; include
/// its origin in the transform. All vertices in a quad share `atlas_rect`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub sprite_uv: [f32; 2],
    pub atlas_rect: [f32; 4],
    pub color: [f32; 4],
}

/// Consecutive vertices using one atlas page, in draw order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Batch {
    pub page: usize,
    pub first: usize,
    pub count: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    MissingFunction(&'static str),
    InvalidVersion(String),
    InsufficientLimits,
    Shader(String),
    Program(String),
    MissingUniform(&'static str),
    InvalidPage,
    InvalidBatch,
    TooManyVertices,
    ObjectCreation(&'static str),
    Gl(u32),
}

/// Padded RGBA8 page. Power-of-two dimensions and gutters are provided by the
/// CPU atlas; the renderer uses GL_NEAREST without mipmaps.
pub struct Page<'a> {
    pub side: u32,
    pub pixels: &'a [u8],
}

#[derive(Clone, Copy)]
pub enum Pass {
    /// Depth writes on, blending off; cutout pixels can be discarded.
    Solid {
        alpha_cutoff: f32,
    },
    /// Depth writes off, alpha blending on. Draw back to front.
    Translucent {
        alpha_cutoff: f32,
    },
    /// GUI drawn over the world, with no depth test or depth writes.
    HudAlpha,
    HudInvert,
    HudAdditive,
}

macro_rules! gl_functions {
    ($( $field:ident : $name:literal => ($($arg:ty),*) -> $ret:ty ),+ $(,)?) => {
        struct Gl { $( $field: unsafe extern "system" fn($($arg),*) -> $ret, )+ }
        impl Gl {
            unsafe fn load(mut resolve: impl FnMut(&CStr) -> *const c_void) -> Result<Self, Error> {
                $(
                    let name = CStr::from_bytes_with_nul(concat!($name, "\0").as_bytes()).expect("static GL name");
                    let address = resolve(name);
                    // wglGetProcAddress reports these sentinel values on failure.
                    if address.is_null() || (address as usize) <= 3 || address as usize == usize::MAX {
                        return Err(Error::MissingFunction($name));
                    }
                    // SAFETY: the caller guarantees the returned symbol has the named GL ABI.
                    let $field = unsafe { mem::transmute::<*const c_void, unsafe extern "system" fn($($arg),*) -> $ret>(address) };
                )+
                Ok(Self { $($field),+ })
            }
        }
    };
}

gl_functions! {
    get_string: "glGetString" => (u32) -> *const u8,
    get_integer: "glGetIntegerv" => (u32, *mut i32) -> (),
    get_error: "glGetError" => () -> u32,
    create_shader: "glCreateShader" => (u32) -> u32,
    shader_source: "glShaderSource" => (u32, i32, *const *const c_char, *const i32) -> (),
    compile_shader: "glCompileShader" => (u32) -> (),
    get_shader: "glGetShaderiv" => (u32, u32, *mut i32) -> (),
    shader_log: "glGetShaderInfoLog" => (u32, i32, *mut i32, *mut c_char) -> (),
    delete_shader: "glDeleteShader" => (u32) -> (),
    create_program: "glCreateProgram" => () -> u32,
    attach_shader: "glAttachShader" => (u32, u32) -> (),
    bind_attrib: "glBindAttribLocation" => (u32, u32, *const c_char) -> (),
    link_program: "glLinkProgram" => (u32) -> (),
    get_program: "glGetProgramiv" => (u32, u32, *mut i32) -> (),
    program_log: "glGetProgramInfoLog" => (u32, i32, *mut i32, *mut c_char) -> (),
    delete_program: "glDeleteProgram" => (u32) -> (),
    use_program: "glUseProgram" => (u32) -> (),
    uniform_location: "glGetUniformLocation" => (u32, *const c_char) -> i32,
    uniform_1i: "glUniform1i" => (i32, i32) -> (),
    uniform_1f: "glUniform1f" => (i32, f32) -> (),
    uniform_matrix: "glUniformMatrix4fv" => (i32, i32, u8, *const f32) -> (),
    gen_buffers: "glGenBuffers" => (i32, *mut u32) -> (),
    delete_buffers: "glDeleteBuffers" => (i32, *const u32) -> (),
    bind_buffer: "glBindBuffer" => (u32, u32) -> (),
    buffer_data: "glBufferData" => (u32, isize, *const c_void, u32) -> (),
    enable_attrib: "glEnableVertexAttribArray" => (u32) -> (),
    disable_attrib: "glDisableVertexAttribArray" => (u32) -> (),
    attrib_pointer: "glVertexAttribPointer" => (u32, i32, u32, u8, i32, *const c_void) -> (),
    gen_textures: "glGenTextures" => (i32, *mut u32) -> (),
    delete_textures: "glDeleteTextures" => (i32, *const u32) -> (),
    active_texture: "glActiveTexture" => (u32) -> (),
    bind_texture: "glBindTexture" => (u32, u32) -> (),
    tex_parameter: "glTexParameteri" => (u32, u32, i32) -> (),
    pixel_store: "glPixelStorei" => (u32, i32) -> (),
    tex_image: "glTexImage2D" => (u32, i32, i32, i32, i32, i32, u32, u32, *const c_void) -> (),
    tex_sub_image: "glTexSubImage2D" => (u32, i32, i32, i32, i32, i32, u32, u32, *const c_void) -> (),
    enable: "glEnable" => (u32) -> (),
    disable: "glDisable" => (u32) -> (),
    depth_func: "glDepthFunc" => (u32) -> (),
    depth_mask: "glDepthMask" => (u8) -> (),
    depth_range: "glDepthRange" => (f64, f64) -> (),
    color_mask: "glColorMask" => (u8, u8, u8, u8) -> (),
    blend_func_separate: "glBlendFuncSeparate" => (u32, u32, u32, u32) -> (),
    blend_equation: "glBlendEquation" => (u32) -> (),
    draw_arrays: "glDrawArrays" => (u32, i32, i32) -> (),
}

const GL_TEXTURE_2D: u32 = 0x0de1;
const GL_ARRAY_BUFFER: u32 = 0x8892;
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;

fn version_at_least(value: &str, expected: (u32, u32)) -> bool {
    let token = value.split_whitespace().next().unwrap_or("");
    let mut parts = token.split('.');
    match (
        parts.next().and_then(|v| v.parse().ok()),
        parts.next().and_then(|v| v.parse().ok()),
    ) {
        (Some(major), Some(minor)) => (major, minor) >= expected,
        _ => false,
    }
}

fn validate_batches(
    vertices: &[Vertex],
    batches: &[Batch],
    page_count: usize,
) -> Result<(), Error> {
    if vertices.len() > i32::MAX as usize || mem::size_of_val(vertices) > isize::MAX as usize {
        return Err(Error::TooManyVertices);
    }
    for batch in batches {
        let end = batch
            .first
            .checked_add(batch.count)
            .ok_or(Error::InvalidBatch)?;
        if batch.page >= page_count
            || batch.first % 3 != 0
            || batch.count % 3 != 0
            || end > vertices.len()
        {
            return Err(Error::InvalidBatch);
        }
    }
    Ok(())
}

/// Context-bound objects; deliberately !Send and !Sync. Explicit destruction is
/// required: Drop cannot safely assume that a GL context is still current.
pub struct Renderer {
    gl: Gl,
    program: u32,
    buffer: u32,
    textures: Vec<u32>,
    texture_sides: Vec<u32>,
    max_texture_size: u32,
    view_projection: i32,
    alpha_cutoff: i32,
    atlas: i32,
    _thread: PhantomData<Rc<()>>,
}

impl Renderer {
    /// Load entry points and compile the prototype GLSL 1.20 terrain program.
    ///
    /// # Safety
    /// A desktop OpenGL context must be current on this thread. `resolve` must
    /// return the correct function for this context and ABI, including core
    /// functions from opengl32.dll on Windows when wglGetProcAddress fails.
    pub unsafe fn new(resolve: impl FnMut(&CStr) -> *const c_void) -> Result<Self, Error> {
        // SAFETY: guaranteed by the caller for this and all GL calls below.
        unsafe {
            let gl = Gl::load(resolve)?;
            let preexisting = (gl.get_error)();
            if preexisting != 0 {
                return Err(Error::Gl(preexisting));
            }
            let get_text = |name| {
                let p = (gl.get_string)(name);
                if p.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(p.cast()).to_string_lossy().into_owned()
                }
            };
            let version = get_text(0x1f02);
            let glsl = get_text(0x8b8c);
            if !version_at_least(&version, (2, 1)) || !version_at_least(&glsl, (1, 20)) {
                return Err(Error::InvalidVersion(format!("GL {version}, GLSL {glsl}")));
            }
            let mut max_texture = 0;
            let mut attributes = 0;
            (gl.get_integer)(0x0d33, &mut max_texture);
            (gl.get_integer)(0x8869, &mut attributes);
            if max_texture < 4 || attributes < 4 {
                return Err(Error::InsufficientLimits);
            }
            let vert = compile(&gl, 0x8b31, TERRAIN_VERTEX_SHADER)?;
            let frag = match compile(&gl, 0x8b30, TERRAIN_FRAGMENT_SHADER) {
                Ok(shader) => shader,
                Err(error) => {
                    (gl.delete_shader)(vert);
                    return Err(error);
                }
            };
            let program = (gl.create_program)();
            if program == 0 {
                (gl.delete_shader)(vert);
                (gl.delete_shader)(frag);
                return Err(Error::Program("glCreateProgram returned zero".into()));
            }
            (gl.attach_shader)(program, vert);
            (gl.attach_shader)(program, frag);
            for (index, name) in [c"position", c"sprite_uv", c"atlas_rect", c"vertex_color"]
                .iter()
                .enumerate()
            {
                (gl.bind_attrib)(program, index as u32, name.as_ptr());
            }
            (gl.link_program)(program);
            (gl.delete_shader)(vert);
            (gl.delete_shader)(frag);
            let mut linked = 0;
            (gl.get_program)(program, 0x8b82, &mut linked);
            if linked == 0 {
                let log = program_log(&gl, program);
                (gl.delete_program)(program);
                return Err(Error::Program(log));
            }
            let uniform = |name: &CStr| (gl.uniform_location)(program, name.as_ptr());
            let view_projection = uniform(c"view_projection");
            let alpha_cutoff = uniform(c"alpha_cutoff");
            let atlas = uniform(c"atlas");
            for (name, value) in [
                ("view_projection", view_projection),
                ("alpha_cutoff", alpha_cutoff),
                ("atlas", atlas),
            ] {
                if value < 0 {
                    (gl.delete_program)(program);
                    return Err(Error::MissingUniform(name));
                }
            }
            let mut buffer = 0;
            (gl.gen_buffers)(1, &mut buffer);
            let error = (gl.get_error)();
            if error != 0 || buffer == 0 {
                if buffer != 0 {
                    (gl.delete_buffers)(1, &buffer);
                }
                (gl.delete_program)(program);
                return Err(if error != 0 {
                    Error::Gl(error)
                } else {
                    Error::ObjectCreation("buffer")
                });
            }
            Ok(Self {
                gl,
                program,
                buffer,
                textures: Vec::new(),
                texture_sides: Vec::new(),
                max_texture_size: max_texture as u32,
                view_projection,
                alpha_cutoff,
                atlas,
                _thread: PhantomData,
            })
        }
    }

    pub fn max_texture_size(&self) -> u32 {
        self.max_texture_size
    }

    /// Replace all atlas pages. On failure the previous pages are retained.
    ///
    /// # Safety
    /// The same context used for `new` must be current on this thread.
    pub unsafe fn upload_pages(&mut self, pages: &[Page<'_>]) -> Result<(), Error> {
        for page in pages {
            let bytes = (page.side as usize)
                .checked_mul(page.side as usize)
                .and_then(|n| n.checked_mul(4));
            if !page.side.is_power_of_two()
                || page.side > self.max_texture_size
                || bytes != Some(page.pixels.len())
            {
                return Err(Error::InvalidPage);
            }
        }
        if pages.len() > i32::MAX as usize {
            return Err(Error::InvalidPage);
        }
        // SAFETY: caller guarantees the owning context is current.
        unsafe {
            let preexisting = (self.gl.get_error)();
            if preexisting != 0 {
                return Err(Error::Gl(preexisting));
            }
            let mut textures = vec![0; pages.len()];
            if !textures.is_empty() {
                (self.gl.gen_textures)(textures.len() as i32, textures.as_mut_ptr());
            }
            prepare_unpack(&self.gl);
            for (page, &texture) in pages.iter().zip(&textures) {
                (self.gl.bind_texture)(GL_TEXTURE_2D, texture);
                (self.gl.tex_parameter)(GL_TEXTURE_2D, 0x2801, 0x2600); // GL_NEAREST
                (self.gl.tex_parameter)(GL_TEXTURE_2D, 0x2800, 0x2600);
                (self.gl.tex_parameter)(GL_TEXTURE_2D, 0x2802, 0x812f); // GL_CLAMP_TO_EDGE
                (self.gl.tex_parameter)(GL_TEXTURE_2D, 0x2803, 0x812f);
                (self.gl.tex_image)(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA as i32,
                    page.side as i32,
                    page.side as i32,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    page.pixels.as_ptr().cast(),
                );
            }
            let error = (self.gl.get_error)();
            if error != 0 || textures.contains(&0) {
                if !textures.is_empty() {
                    (self.gl.delete_textures)(textures.len() as i32, textures.as_ptr());
                }
                return Err(if error != 0 {
                    Error::Gl(error)
                } else {
                    Error::ObjectCreation("texture")
                });
            }
            if !self.textures.is_empty() {
                (self.gl.delete_textures)(self.textures.len() as i32, self.textures.as_ptr());
            }
            self.textures = textures;
            self.texture_sides = pages.iter().map(|page| page.side).collect();
            Ok(())
        }
    }

    /// Replace the pixels of one existing atlas page, for animated sprites.
    /// The dimensions must match the original page. If the driver reports an
    /// error, its texture contents are unspecified and should be reuploaded.
    ///
    /// # Safety
    /// The same context used for `new` must be current on this thread.
    pub unsafe fn update_page(&mut self, index: usize, page: Page<'_>) -> Result<(), Error> {
        let expected = (page.side as usize)
            .checked_mul(page.side as usize)
            .and_then(|n| n.checked_mul(4));
        if self.texture_sides.get(index).copied() != Some(page.side)
            || expected != Some(page.pixels.len())
        {
            return Err(Error::InvalidPage);
        }
        // SAFETY: the caller guarantees the owning context and the pixels
        // remain live throughout the call.
        unsafe {
            let gl = &self.gl;
            let preexisting = (gl.get_error)();
            if preexisting != 0 {
                return Err(Error::Gl(preexisting));
            }
            prepare_unpack(gl);
            (gl.bind_texture)(GL_TEXTURE_2D, self.textures[index]);
            (gl.tex_sub_image)(
                GL_TEXTURE_2D,
                0,
                0,
                0,
                page.side as i32,
                page.side as i32,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                page.pixels.as_ptr().cast(),
            );
            let error = (gl.get_error)();
            if error != 0 {
                Err(Error::Gl(error))
            } else {
                Ok(())
            }
        }
    }

    /// Upload and draw the batches in order. Changes GL program, buffer,
    /// textures, attributes and depth/blend/cull state; it does not restore
    /// previous state. The caller sets viewport, clear and framebuffer. The
    /// transform is column-major and must produce OpenGL clip coordinates
    /// (depth -1..1), not the 0..1 convention used by wgpu.
    ///
    /// # Safety
    /// The same context used for `new` must be current on this thread.
    pub unsafe fn draw(
        &mut self,
        vertices: &[Vertex],
        batches: &[Batch],
        transform: &[f32; 16],
        pass: Pass,
    ) -> Result<(), Error> {
        validate_batches(vertices, batches, self.textures.len())?;
        // SAFETY: caller guarantees the owning context is current; all slice
        // pointers and byte sizes are validated and remain live for each call.
        unsafe {
            let gl = &self.gl;
            let preexisting = (gl.get_error)();
            if preexisting != 0 {
                return Err(Error::Gl(preexisting));
            }
            let cutoff = match pass {
                Pass::Solid { alpha_cutoff } | Pass::Translucent { alpha_cutoff } => alpha_cutoff,
                Pass::HudInvert => 0.0001,
                Pass::HudAlpha | Pass::HudAdditive => 0.004,
            };
            (gl.use_program)(self.program);
            (gl.uniform_matrix)(self.view_projection, 1, 0, transform.as_ptr());
            (gl.uniform_1i)(self.atlas, 0);
            (gl.uniform_1f)(self.alpha_cutoff, cutoff);
            (gl.enable)(0x0b71); // depth test
            (gl.depth_func)(0x0201); // less
            (gl.depth_range)(0.0, 1.0);
            (gl.color_mask)(1, 1, 1, 1);
            (gl.disable)(0x0b44); // culling is not yet selected per material
            match pass {
                Pass::Solid { .. } => {
                    (gl.disable)(0x0be2);
                    (gl.depth_mask)(1);
                }
                Pass::Translucent { .. } | Pass::HudAlpha => {
                    if matches!(pass, Pass::HudAlpha) {
                        (gl.disable)(0x0b71);
                    }
                    (gl.enable)(0x0be2);
                    (gl.blend_equation)(0x8006); // GL_FUNC_ADD
                    (gl.blend_func_separate)(0x0302, 0x0303, 1, 0x0303);
                    (gl.depth_mask)(0);
                }
                Pass::HudInvert => {
                    (gl.disable)(0x0b71);
                    (gl.enable)(0x0be2);
                    (gl.blend_equation)(0x8006);
                    (gl.blend_func_separate)(0x0307, 0x0301, 1, 0);
                    (gl.depth_mask)(0);
                }
                Pass::HudAdditive => {
                    (gl.disable)(0x0b71);
                    (gl.enable)(0x0be2);
                    (gl.blend_equation)(0x8006);
                    (gl.blend_func_separate)(1, 1, 1, 1);
                    (gl.depth_mask)(0);
                }
            }
            (gl.active_texture)(0x84c0);
            (gl.bind_buffer)(GL_ARRAY_BUFFER, self.buffer);
            (gl.buffer_data)(
                GL_ARRAY_BUFFER,
                mem::size_of_val(vertices) as isize,
                vertices.as_ptr().cast(),
                0x88e0,
            ); // STREAM_DRAW
            let stride = mem::size_of::<Vertex>() as i32;
            for (index, size, offset) in [
                (0, 3, mem::offset_of!(Vertex, position)),
                (1, 2, mem::offset_of!(Vertex, sprite_uv)),
                (2, 4, mem::offset_of!(Vertex, atlas_rect)),
                (3, 4, mem::offset_of!(Vertex, color)),
            ] {
                (gl.enable_attrib)(index);
                (gl.attrib_pointer)(index, size, 0x1406, 0, stride, offset as *const c_void);
            }
            for batch in batches {
                (gl.bind_texture)(GL_TEXTURE_2D, self.textures[batch.page]);
                (gl.draw_arrays)(0x0004, batch.first as i32, batch.count as i32);
            }
            for index in 0..4 {
                (gl.disable_attrib)(index);
            }
            (gl.depth_mask)(1); // do not leave subsequent clears with writes disabled
            let error = (gl.get_error)();
            if error != 0 {
                Err(Error::Gl(error))
            } else {
                Ok(())
            }
        }
    }

    /// Delete this renderer's GL resources.
    ///
    /// # Safety
    /// The same context used for `new` must be current on this thread.
    pub unsafe fn destroy(self) {
        // SAFETY: caller guarantees the owning context is current.
        unsafe {
            if !self.textures.is_empty() {
                (self.gl.delete_textures)(self.textures.len() as i32, self.textures.as_ptr());
            }
            (self.gl.delete_buffers)(1, &self.buffer);
            (self.gl.delete_program)(self.program);
        }
    }
}

unsafe fn prepare_unpack(gl: &Gl) {
    // SAFETY: called only with a current context and loaded entry points.
    unsafe {
        (gl.active_texture)(0x84c0);
        (gl.bind_buffer)(0x88ec, 0); // GL_PIXEL_UNPACK_BUFFER
        (gl.pixel_store)(0x0cf5, 4); // GL_UNPACK_ALIGNMENT
        (gl.pixel_store)(0x0cf2, 0); // GL_UNPACK_ROW_LENGTH
        (gl.pixel_store)(0x0cf3, 0); // GL_UNPACK_SKIP_ROWS
        (gl.pixel_store)(0x0cf4, 0); // GL_UNPACK_SKIP_PIXELS
    }
}

unsafe fn compile(gl: &Gl, kind: u32, source: &str) -> Result<u32, Error> {
    // SAFETY: called only with a current context and loaded entry points.
    unsafe {
        let shader = (gl.create_shader)(kind);
        if shader == 0 {
            return Err(Error::Shader("glCreateShader returned zero".into()));
        }
        let text = source.as_ptr().cast::<c_char>();
        let length = source.len() as i32;
        (gl.shader_source)(shader, 1, &text, &length);
        (gl.compile_shader)(shader);
        let mut compiled = 0;
        (gl.get_shader)(shader, 0x8b81, &mut compiled);
        if compiled == 0 {
            let mut log = vec![0u8; 8192];
            let mut written = 0;
            (gl.shader_log)(
                shader,
                log.len() as i32,
                &mut written,
                log.as_mut_ptr().cast(),
            );
            (gl.delete_shader)(shader);
            log.truncate((written.max(0) as usize).min(log.len()));
            return Err(Error::Shader(String::from_utf8_lossy(&log).into_owned()));
        }
        Ok(shader)
    }
}

unsafe fn program_log(gl: &Gl, program: u32) -> String {
    // SAFETY: called only with a current context and a live program.
    unsafe {
        let mut log = vec![0u8; 8192];
        let mut written = 0;
        (gl.program_log)(
            program,
            log.len() as i32,
            &mut written,
            log.as_mut_ptr().cast(),
        );
        log.truncate((written.max(0) as usize).min(log.len()));
        String::from_utf8_lossy(&log).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_desktop_versions_only() {
        assert!(version_at_least("2.1 Mesa 21.1.4", (2, 1)));
        assert!(version_at_least("1.20", (1, 20)));
        assert!(!version_at_least("OpenGL ES 3.0", (2, 1)));
        assert!(!version_at_least("2.0", (2, 1)));
    }
    #[test]
    fn rejects_invalid_batches_before_gl() {
        let v = Vertex {
            position: [0.; 3],
            sprite_uv: [0.; 2],
            atlas_rect: [0.; 4],
            color: [1.; 4],
        };
        let verts = [v; 6];
        assert!(
            validate_batches(
                &verts,
                &[Batch {
                    page: 0,
                    first: 0,
                    count: 6
                }],
                1
            )
            .is_ok()
        );
        assert_eq!(
            validate_batches(
                &verts,
                &[Batch {
                    page: 1,
                    first: 0,
                    count: 6
                }],
                1
            ),
            Err(Error::InvalidBatch)
        );
        assert_eq!(
            validate_batches(
                &verts,
                &[Batch {
                    page: 0,
                    first: 3,
                    count: 6
                }],
                1
            ),
            Err(Error::InvalidBatch)
        );
        assert_eq!(
            validate_batches(
                &verts,
                &[Batch {
                    page: 0,
                    first: 1,
                    count: 3
                }],
                1
            ),
            Err(Error::InvalidBatch)
        );
        assert_eq!(
            validate_batches(
                &verts,
                &[Batch {
                    page: 0,
                    first: usize::MAX,
                    count: 3
                }],
                1
            ),
            Err(Error::InvalidBatch)
        );
    }
}
