//! Experimental OpenGL 2 terrain renderer. This does not create a window or
//! select a backend in the game. A host must provide a current desktop GL 2.0
//! context, a function loader, and destroy the renderer before that context.
//!
//! All GL entry points are isolated here; no unsafe code is needed by the CPU
//! mesh preparation in `horizon-client-render`.

use std::{
    ffi::{CStr, CString, c_char, c_void},
    marker::PhantomData,
    mem, ptr,
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
};

const VERTEX_SHADER: &str = include_str!("../../horizon-client-render/src/legacy_gl_terrain.vert");
const FRAGMENT_SHADER: &str =
    include_str!("../../horizon-client-render/src/legacy_gl_terrain.frag");
const ARRAY_BUFFER: u32 = 0x8892;
const ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
const TEXTURE_2D: u32 = 0x0de1;
const RGBA: u32 = 0x1908;
const UNSIGNED_BYTE: u32 = 0x1401;
const FLOAT: u32 = 0x1406;
const UNSIGNED_SHORT: u32 = 0x1403;

macro_rules! gl_functions {
    ($( $field:ident : $name:literal ( $($arg:ty),* ) -> $ret:ty; )*) => {
        #[derive(Clone, Copy)]
        struct Gl { $( $field: unsafe extern "system" fn($($arg),*) -> $ret, )* }
        impl Gl {
            unsafe fn load(mut lookup: impl FnMut(&CStr) -> *const c_void) -> Result<Self, Error> {
                $(
                    let name = CStr::from_bytes_with_nul(concat!($name, "\0").as_bytes()).expect("static GL name");
                    let address = lookup(name);
                    // WGL uses 1, 2, 3 and -1 as failure sentinels, not just null.
                    if address.is_null() || (address as usize) <= 3 || address as isize == -1 {
                        return Err(Error::MissingFunction($name));
                    }
                    let $field = mem::transmute::<*const c_void, unsafe extern "system" fn($($arg),*) -> $ret>(address);
                )*
                Ok(Self { $($field,)* })
            }
        }
    }
}

gl_functions! {
    get_error: "glGetError"() -> u32;
    get_integer: "glGetIntegerv"(u32, *mut i32) -> ();
    create_shader: "glCreateShader"(u32) -> u32;
    shader_source: "glShaderSource"(u32, i32, *const *const c_char, *const i32) -> ();
    compile_shader: "glCompileShader"(u32) -> ();
    get_shader: "glGetShaderiv"(u32, u32, *mut i32) -> ();
    shader_log: "glGetShaderInfoLog"(u32, i32, *mut i32, *mut c_char) -> ();
    delete_shader: "glDeleteShader"(u32) -> ();
    create_program: "glCreateProgram"() -> u32;
    attach_shader: "glAttachShader"(u32, u32) -> ();
    link_program: "glLinkProgram"(u32) -> ();
    get_program: "glGetProgramiv"(u32, u32, *mut i32) -> ();
    program_log: "glGetProgramInfoLog"(u32, i32, *mut i32, *mut c_char) -> ();
    delete_program: "glDeleteProgram"(u32) -> ();
    use_program: "glUseProgram"(u32) -> ();
    get_uniform: "glGetUniformLocation"(u32, *const c_char) -> i32;
    get_attrib: "glGetAttribLocation"(u32, *const c_char) -> i32;
    uniform_1i: "glUniform1i"(i32, i32) -> ();
    uniform_1f: "glUniform1f"(i32, f32) -> ();
    uniform_3f: "glUniform3f"(i32, f32, f32, f32) -> ();
    uniform_matrix: "glUniformMatrix4fv"(i32, i32, u8, *const f32) -> ();
    gen_buffers: "glGenBuffers"(i32, *mut u32) -> ();
    delete_buffers: "glDeleteBuffers"(i32, *const u32) -> ();
    bind_buffer: "glBindBuffer"(u32, u32) -> ();
    buffer_data: "glBufferData"(u32, isize, *const c_void, u32) -> ();
    enable_attrib: "glEnableVertexAttribArray"(u32) -> ();
    disable_attrib: "glDisableVertexAttribArray"(u32) -> ();
    attrib_pointer: "glVertexAttribPointer"(u32, i32, u32, u8, i32, *const c_void) -> ();
    active_texture: "glActiveTexture"(u32) -> ();
    gen_textures: "glGenTextures"(i32, *mut u32) -> ();
    delete_textures: "glDeleteTextures"(i32, *const u32) -> ();
    bind_texture: "glBindTexture"(u32, u32) -> ();
    tex_parameter: "glTexParameteri"(u32, u32, i32) -> ();
    pixel_store: "glPixelStorei"(u32, i32) -> ();
    tex_image: "glTexImage2D"(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void) -> ();
    tex_sub_image: "glTexSubImage2D"(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void) -> ();
    viewport: "glViewport"(i32, i32, i32, i32) -> ();
    clear_color: "glClearColor"(f32, f32, f32, f32) -> ();
    clear: "glClear"(u32) -> ();
    enable: "glEnable"(u32) -> ();
    disable: "glDisable"(u32) -> ();
    depth_func: "glDepthFunc"(u32) -> ();
    depth_mask: "glDepthMask"(u8) -> ();
    blend_func: "glBlendFunc"(u32, u32) -> ();
    blend_equation: "glBlendEquation"(u32) -> ();
    color_mask: "glColorMask"(u8, u8, u8, u8) -> ();
    draw_elements: "glDrawElements"(u32, i32, u32, *const c_void) -> ();
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    MissingFunction(&'static str),
    MissingShaderVariable(&'static str),
    Shader(String),
    Program(String),
    UnsupportedLimits,
    InvalidTexture,
    InvalidMesh,
    InvalidViewport,
    ObjectCreation,
    Gl(u32),
}

/// A GL-compatible vertex. Its layout matches the four GLSL attributes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub light_uv: [f32; 2],
    pub color: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureId {
    owner: usize,
    index: usize,
}

static NEXT_OWNER: AtomicUsize = AtomicUsize::new(1);

/// Filtering for a level-zero atlas (nearest) or a lightmap (linear).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureFilter {
    Nearest,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    Opaque,
    Cutout,
    Translucent,
}

/// Parameters common to streaming and resident terrain draws.
#[derive(Clone, Copy, Debug)]
pub struct DrawState {
    pub atlas: TextureId,
    pub lightmap: TextureId,
    /// Column-major OpenGL projection/view matrix, operating on camera-relative coordinates.
    pub view_projection: [f32; 16],
    pub section_origin: [f32; 3],
    pub pass: Pass,
    /// Alpha threshold for cutouts, in [0, 1].
    pub alpha_cutoff: f32,
}

pub struct Draw<'a> {
    pub vertices: &'a [Vertex],
    pub indices: &'a [u16],
    pub state: DrawState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshId {
    owner: usize,
    index: usize,
}

struct Mesh {
    vertex: u32,
    index: u32,
    count: i32,
}

struct Texture {
    id: u32,
    width: u32,
    height: u32,
}
struct Locations {
    position: u32,
    uv: u32,
    light_uv: u32,
    color: u32,
    matrix: i32,
    origin: i32,
    atlas: i32,
    lightmap: i32,
    cutoff: i32,
}

/// GL objects associated with one context. It cannot move to another thread.
/// Call [`Renderer::destroy`] while the context is current; dropping this value
/// alone does not call GL (the context eventually releases any remaining objects).
pub struct Renderer {
    gl: Gl,
    owner: usize,
    program: u32,
    vertex_buffer: u32,
    index_buffer: u32,
    textures: Vec<Option<Texture>>,
    meshes: Vec<Option<Mesh>>,
    locations: Locations,
    max_texture_size: u32,
    max_attribs: u32,
    _not_send: PhantomData<Rc<()>>,
}

struct InitGuard {
    gl: Gl,
    program: u32,
    vs: u32,
    fs: u32,
    vertex: u32,
    index: u32,
}
impl Drop for InitGuard {
    fn drop(&mut self) {
        unsafe {
            if self.vs != 0 {
                (self.gl.delete_shader)(self.vs);
            }
            if self.fs != 0 {
                (self.gl.delete_shader)(self.fs);
            }
            if self.program != 0 {
                (self.gl.delete_program)(self.program);
            }
            if self.vertex != 0 {
                (self.gl.delete_buffers)(1, &self.vertex);
            }
            if self.index != 0 {
                (self.gl.delete_buffers)(1, &self.index);
            }
        }
    }
}

unsafe fn check(gl: Gl) -> Result<(), Error> {
    let code = (gl.get_error)();
    if code == 0 {
        Ok(())
    } else {
        Err(Error::Gl(code))
    }
}
unsafe fn shader(gl: Gl, id: u32, source: &str) -> Result<(), Error> {
    let text = CString::new(source).map_err(|e| Error::Shader(e.to_string()))?;
    (gl.shader_source)(id, 1, &text.as_ptr(), ptr::null());
    (gl.compile_shader)(id);
    let mut status = 0;
    (gl.get_shader)(id, 0x8b81, &mut status);
    if status != 0 {
        return Ok(());
    }
    let mut data = [0u8; 4096];
    let mut len = 0;
    (gl.shader_log)(id, data.len() as i32, &mut len, data.as_mut_ptr().cast());
    Err(Error::Shader(
        String::from_utf8_lossy(&data[..(len.max(0) as usize).min(data.len())]).into_owned(),
    ))
}
unsafe fn location(
    gl: Gl,
    program: u32,
    name: &'static str,
    attribute: bool,
) -> Result<i32, Error> {
    let text = CString::new(name).expect("static shader variable");
    let value = if attribute {
        (gl.get_attrib)(program, text.as_ptr())
    } else {
        (gl.get_uniform)(program, text.as_ptr())
    };
    if value < 0 {
        Err(Error::MissingShaderVariable(name))
    } else {
        Ok(value)
    }
}

fn validate_mesh(vertices: &[Vertex], indices: &[u16]) -> Result<(isize, isize), Error> {
    if vertices.len() > 65536
        || indices.len() > i32::MAX as usize
        || indices.len() % 3 != 0
        || indices.iter().any(|&i| usize::from(i) >= vertices.len())
    {
        return Err(Error::InvalidMesh);
    }
    let vb = vertices
        .len()
        .checked_mul(mem::size_of::<Vertex>())
        .and_then(|n| isize::try_from(n).ok())
        .ok_or(Error::InvalidMesh)?;
    let ib = indices
        .len()
        .checked_mul(mem::size_of::<u16>())
        .and_then(|n| isize::try_from(n).ok())
        .ok_or(Error::InvalidMesh)?;
    Ok((vb, ib))
}

impl Renderer {
    /// Creates terrain GL objects and validates shader interfaces and device limits.
    ///
    /// # Safety
    /// A desktop GL 2.0 context must be current on this thread throughout this
    /// call and every later method call. `lookup` must return functions with the
    /// exact OpenGL ABI for that context, including core GL 1.1 functions. The
    /// context must outlive this renderer. GL state belongs exclusively to this
    /// renderer during calls; no concurrent context use is permitted.
    pub unsafe fn new(lookup: impl FnMut(&CStr) -> *const c_void) -> Result<Self, Error> {
        let gl = Gl::load(lookup)?;
        let mut units = 0;
        let mut maximum = 0;
        let mut max_attribs = 0;
        (gl.get_integer)(0x8869, &mut max_attribs); // MAX_VERTEX_ATTRIBS
        (gl.get_integer)(0x8872, &mut units); // MAX_TEXTURE_IMAGE_UNITS
        (gl.get_integer)(0x0d33, &mut maximum); // MAX_TEXTURE_SIZE
        check(gl)?;
        if units < 2 || maximum < 1 || max_attribs < 4 {
            return Err(Error::UnsupportedLimits);
        }
        let owner = NEXT_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Error::ObjectCreation)?;
        let mut guard = InitGuard {
            gl,
            program: 0,
            vs: 0,
            fs: 0,
            vertex: 0,
            index: 0,
        };
        guard.vs = (gl.create_shader)(0x8b31);
        if guard.vs == 0 {
            return Err(Error::ObjectCreation);
        }
        shader(gl, guard.vs, VERTEX_SHADER)?;
        guard.fs = (gl.create_shader)(0x8b30);
        if guard.fs == 0 {
            return Err(Error::ObjectCreation);
        }
        shader(gl, guard.fs, FRAGMENT_SHADER)?;
        guard.program = (gl.create_program)();
        if guard.program == 0 {
            return Err(Error::ObjectCreation);
        }
        (gl.attach_shader)(guard.program, guard.vs);
        (gl.attach_shader)(guard.program, guard.fs);
        (gl.link_program)(guard.program);
        let mut status = 0;
        (gl.get_program)(guard.program, 0x8b82, &mut status);
        if status == 0 {
            let mut data = [0u8; 4096];
            let mut len = 0;
            (gl.program_log)(
                guard.program,
                data.len() as i32,
                &mut len,
                data.as_mut_ptr().cast(),
            );
            return Err(Error::Program(
                String::from_utf8_lossy(&data[..(len.max(0) as usize).min(data.len())])
                    .into_owned(),
            ));
        }
        let p = guard.program;
        let locations = Locations {
            position: location(gl, p, "a_position", true)? as u32,
            uv: location(gl, p, "a_uv", true)? as u32,
            light_uv: location(gl, p, "a_light_uv", true)? as u32,
            color: location(gl, p, "a_color", true)? as u32,
            matrix: location(gl, p, "u_view_projection", false)?,
            origin: location(gl, p, "u_section_origin", false)?,
            atlas: location(gl, p, "u_atlas", false)?,
            lightmap: location(gl, p, "u_lightmap", false)?,
            cutoff: location(gl, p, "u_alpha_cutoff", false)?,
        };
        if [
            locations.position,
            locations.uv,
            locations.light_uv,
            locations.color,
        ]
        .iter()
        .any(|&n| n >= max_attribs as u32)
        {
            return Err(Error::UnsupportedLimits);
        }
        (gl.gen_buffers)(1, &mut guard.vertex);
        (gl.gen_buffers)(1, &mut guard.index);
        check(gl)?;
        if guard.vertex == 0 || guard.index == 0 {
            return Err(Error::ObjectCreation);
        }
        let renderer = Self {
            gl,
            owner,
            program: guard.program,
            vertex_buffer: guard.vertex,
            index_buffer: guard.index,
            textures: Vec::new(),
            meshes: Vec::new(),
            locations,
            max_texture_size: maximum as u32,
            max_attribs: max_attribs as u32,
            _not_send: PhantomData,
        };
        guard.program = 0;
        guard.vertex = 0;
        guard.index = 0;
        Ok(renderer)
    }

    /// Upload a complete RGBA8 texture without mipmaps. `width` and `height`
    /// must be power-of-two for baseline GL 2.0; this includes 1x1 lightmaps.
    ///
    /// # Safety
    /// The context and function-pointer requirements of [`Renderer::new`] still apply.
    pub unsafe fn create_texture(
        &mut self,
        width: u32,
        height: u32,
        pixels: &[u8],
        filter: TextureFilter,
    ) -> Result<TextureId, Error> {
        let bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if !width.is_power_of_two()
            || !height.is_power_of_two()
            || width > self.max_texture_size
            || height > self.max_texture_size
            || bytes != Some(pixels.len())
        {
            return Err(Error::InvalidTexture);
        }
        let gl = self.gl;
        let mut id = 0;
        (gl.gen_textures)(1, &mut id);
        if id == 0 {
            return Err(Error::ObjectCreation);
        }
        (gl.bind_texture)(TEXTURE_2D, id);
        (gl.pixel_store)(0x0cf5, 4); // UNPACK_ALIGNMENT
        (gl.pixel_store)(0x0cf2, 0); // UNPACK_ROW_LENGTH
        (gl.pixel_store)(0x0cf3, 0); // UNPACK_SKIP_ROWS
        (gl.pixel_store)(0x0cf4, 0); // UNPACK_SKIP_PIXELS
        let filter = match filter {
            TextureFilter::Nearest => 0x2600,
            TextureFilter::Linear => 0x2601,
        };
        (gl.tex_parameter)(TEXTURE_2D, 0x2801, filter); // minification without mipmaps
        (gl.tex_parameter)(TEXTURE_2D, 0x2800, filter);
        (gl.tex_parameter)(TEXTURE_2D, 0x2802, 0x812f); // CLAMP_TO_EDGE
        (gl.tex_parameter)(TEXTURE_2D, 0x2803, 0x812f);
        (gl.tex_image)(
            TEXTURE_2D,
            0,
            RGBA as i32,
            width as i32,
            height as i32,
            0,
            RGBA,
            UNSIGNED_BYTE,
            pixels.as_ptr().cast(),
        );
        if let Err(error) = check(gl) {
            (gl.delete_textures)(1, &id);
            return Err(error);
        }
        let handle = TextureId {
            owner: self.owner,
            index: self.textures.len(),
        };
        self.textures.push(Some(Texture { id, width, height }));
        Ok(handle)
    }

    /// Update an RGBA8 sub-rectangle, including the atlas padding as needed.
    /// # Safety
    /// The context and function-pointer requirements of [`Renderer::new`] still apply.
    pub unsafe fn update_texture(
        &mut self,
        handle: TextureId,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Result<(), Error> {
        if handle.owner != self.owner {
            return Err(Error::InvalidTexture);
        }
        let texture = self
            .textures
            .get(handle.index)
            .and_then(Option::as_ref)
            .ok_or(Error::InvalidTexture)?;
        let end_x = x.checked_add(width).ok_or(Error::InvalidTexture)?;
        let end_y = y.checked_add(height).ok_or(Error::InvalidTexture)?;
        let bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if width == 0
            || height == 0
            || end_x > texture.width
            || end_y > texture.height
            || bytes != Some(pixels.len())
        {
            return Err(Error::InvalidTexture);
        }
        (self.gl.bind_texture)(TEXTURE_2D, texture.id);
        (self.gl.pixel_store)(0x0cf5, 4);
        (self.gl.pixel_store)(0x0cf2, 0);
        (self.gl.pixel_store)(0x0cf3, 0);
        (self.gl.pixel_store)(0x0cf4, 0);
        (self.gl.tex_sub_image)(
            TEXTURE_2D,
            0,
            x as i32,
            y as i32,
            width as i32,
            height as i32,
            RGBA,
            UNSIGNED_BYTE,
            pixels.as_ptr().cast(),
        );
        check(self.gl)
    }

    /// Delete a texture; its handle will never be recycled by this renderer.
    /// # Safety
    /// The context and function-pointer requirements of [`Renderer::new`] still apply.
    pub unsafe fn delete_texture(&mut self, handle: TextureId) -> Result<(), Error> {
        if handle.owner != self.owner {
            return Err(Error::InvalidTexture);
        }
        let slot = self
            .textures
            .get_mut(handle.index)
            .ok_or(Error::InvalidTexture)?;
        let texture = slot.take().ok_or(Error::InvalidTexture)?;
        (self.gl.delete_textures)(1, &texture.id);
        check(self.gl)
    }

    /// Clear a frame. A zero-sized window should skip rendering instead.
    /// # Safety
    /// The context and function-pointer requirements of [`Renderer::new`] still apply.
    pub unsafe fn begin_frame(
        &mut self,
        width: u32,
        height: u32,
        clear: [f32; 4],
    ) -> Result<(), Error> {
        if width == 0
            || height == 0
            || width > i32::MAX as u32
            || height > i32::MAX as u32
            || clear.iter().any(|v| !v.is_finite())
        {
            return Err(Error::InvalidViewport);
        }
        let gl = self.gl;
        (gl.viewport)(0, 0, width as i32, height as i32);
        (gl.disable)(0x0c11); // SCISSOR_TEST
        (gl.depth_mask)(1);
        (gl.color_mask)(1, 1, 1, 1);
        (gl.clear_color)(clear[0], clear[1], clear[2], clear[3]);
        (gl.clear)(0x00004000 | 0x00000100);
        check(gl)
    }

    /// Upload a terrain run once for repeated draws. Use one mesh per ordered
    /// page run. Rebuild a translucent mesh when its sorted index order changes.
    /// # Safety
    /// The context and function-pointer requirements of [`Renderer::new`] still apply.
    pub unsafe fn create_mesh(
        &mut self,
        vertices: &[Vertex],
        indices: &[u16],
    ) -> Result<MeshId, Error> {
        let (vertex_bytes, index_bytes) = validate_mesh(vertices, indices)?;
        let gl = self.gl;
        let mut vertex = 0;
        let mut index = 0;
        (gl.gen_buffers)(1, &mut vertex);
        (gl.gen_buffers)(1, &mut index);
        if vertex == 0 || index == 0 {
            if vertex != 0 {
                (gl.delete_buffers)(1, &vertex);
            }
            if index != 0 {
                (gl.delete_buffers)(1, &index);
            }
            return Err(Error::ObjectCreation);
        }
        (gl.bind_buffer)(ARRAY_BUFFER, vertex);
        (gl.buffer_data)(ARRAY_BUFFER, vertex_bytes, vertices.as_ptr().cast(), 0x88e4); // STATIC_DRAW
        (gl.bind_buffer)(ELEMENT_ARRAY_BUFFER, index);
        (gl.buffer_data)(
            ELEMENT_ARRAY_BUFFER,
            index_bytes,
            indices.as_ptr().cast(),
            0x88e4,
        );
        if let Err(error) = check(gl) {
            (gl.delete_buffers)(1, &vertex);
            (gl.delete_buffers)(1, &index);
            return Err(error);
        }
        let handle = MeshId {
            owner: self.owner,
            index: self.meshes.len(),
        };
        self.meshes.push(Some(Mesh {
            vertex,
            index,
            count: indices.len() as i32,
        }));
        Ok(handle)
    }

    /// Delete a resident run; handles are never recycled.
    /// # Safety
    /// The context and function-pointer requirements of [`Renderer::new`] still apply.
    pub unsafe fn delete_mesh(&mut self, handle: MeshId) -> Result<(), Error> {
        if handle.owner != self.owner {
            return Err(Error::InvalidMesh);
        }
        let mesh = self
            .meshes
            .get_mut(handle.index)
            .and_then(Option::take)
            .ok_or(Error::InvalidMesh)?;
        (self.gl.delete_buffers)(1, &mesh.vertex);
        (self.gl.delete_buffers)(1, &mesh.index);
        check(self.gl)
    }

    /// Upload and draw one transient ordered terrain run.
    /// # Safety
    /// The context and function-pointer requirements of [`Renderer::new`] still apply.
    pub unsafe fn draw(&mut self, draw: &Draw<'_>) -> Result<(), Error> {
        let (vertex_bytes, index_bytes) = validate_mesh(draw.vertices, draw.indices)?;
        self.validate_state(&draw.state)?;
        let gl = self.gl;
        (gl.bind_buffer)(ARRAY_BUFFER, self.vertex_buffer);
        (gl.buffer_data)(
            ARRAY_BUFFER,
            vertex_bytes,
            draw.vertices.as_ptr().cast(),
            0x88e0,
        ); // STREAM_DRAW
        (gl.bind_buffer)(ELEMENT_ARRAY_BUFFER, self.index_buffer);
        (gl.buffer_data)(
            ELEMENT_ARRAY_BUFFER,
            index_bytes,
            draw.indices.as_ptr().cast(),
            0x88e0,
        );
        self.issue_draw(
            &draw.state,
            self.vertex_buffer,
            self.index_buffer,
            draw.indices.len() as i32,
        )
    }

    /// Draw an already uploaded run without retransmitting its geometry.
    /// The caller is responsible for ordering translucent runs.
    /// # Safety
    /// The context and function-pointer requirements of [`Renderer::new`] still apply.
    pub unsafe fn draw_mesh(&mut self, mesh: MeshId, state: &DrawState) -> Result<(), Error> {
        if mesh.owner != self.owner {
            return Err(Error::InvalidMesh);
        }
        let mesh = self
            .meshes
            .get(mesh.index)
            .and_then(Option::as_ref)
            .ok_or(Error::InvalidMesh)?;
        self.validate_state(state)?;
        self.issue_draw(state, mesh.vertex, mesh.index, mesh.count)
    }

    fn validate_state(&self, state: &DrawState) -> Result<(), Error> {
        if state.atlas.owner != self.owner
            || state.lightmap.owner != self.owner
            || self
                .textures
                .get(state.atlas.index)
                .and_then(Option::as_ref)
                .is_none()
            || self
                .textures
                .get(state.lightmap.index)
                .and_then(Option::as_ref)
                .is_none()
        {
            return Err(Error::InvalidTexture);
        }
        if !state.alpha_cutoff.is_finite()
            || !(0.0..=1.0).contains(&state.alpha_cutoff)
            || state
                .view_projection
                .iter()
                .chain(state.section_origin.iter())
                .any(|v| !v.is_finite())
        {
            return Err(Error::InvalidMesh);
        }
        Ok(())
    }

    unsafe fn issue_draw(
        &self,
        state: &DrawState,
        vertex_buffer: u32,
        index_buffer: u32,
        index_count: i32,
    ) -> Result<(), Error> {
        // validate_state has checked both handles without making any GL calls.
        let atlas = self.textures[state.atlas.index]
            .as_ref()
            .expect("validated atlas");
        let lightmap = self.textures[state.lightmap.index]
            .as_ref()
            .expect("validated lightmap");
        let gl = self.gl;
        (gl.use_program)(self.program);
        (gl.enable)(0x0b71); // DEPTH_TEST
        (gl.depth_func)(0x0203); // LEQUAL
        (gl.disable)(0x0b44); // CULL_FACE: meshes may be double-sided
        (gl.disable)(0x0c11); // SCISSOR_TEST
        (gl.disable)(0x0b90); // STENCIL_TEST
        (gl.disable)(0x8037); // POLYGON_OFFSET_FILL
        (gl.color_mask)(1, 1, 1, 1);
        if state.pass == Pass::Translucent {
            (gl.enable)(0x0be2);
            (gl.blend_equation)(0x8006);
            (gl.blend_func)(0x0302, 0x0303);
            (gl.depth_mask)(0);
        } else {
            (gl.disable)(0x0be2);
            (gl.depth_mask)(1);
        }
        (gl.uniform_matrix)(self.locations.matrix, 1, 0, state.view_projection.as_ptr());
        (gl.uniform_3f)(
            self.locations.origin,
            state.section_origin[0],
            state.section_origin[1],
            state.section_origin[2],
        );
        (gl.uniform_1f)(
            self.locations.cutoff,
            if state.pass == Pass::Cutout {
                state.alpha_cutoff
            } else {
                0.0
            },
        );
        for (unit, texture, uniform) in [
            (0, atlas.id, self.locations.atlas),
            (1, lightmap.id, self.locations.lightmap),
        ] {
            (gl.active_texture)(0x84c0 + unit);
            (gl.bind_texture)(TEXTURE_2D, texture);
            (gl.uniform_1i)(uniform, unit as i32);
        }
        (gl.bind_buffer)(ARRAY_BUFFER, vertex_buffer);
        (gl.bind_buffer)(ELEMENT_ARRAY_BUFFER, index_buffer);
        for index in 0..self.max_attribs {
            (gl.disable_attrib)(index);
        }
        for (loc, count, offset) in [
            (self.locations.position, 3, 0usize),
            (self.locations.uv, 2, 12),
            (self.locations.light_uv, 2, 20),
            (self.locations.color, 4, 28),
        ] {
            (gl.enable_attrib)(loc);
            (gl.attrib_pointer)(
                loc,
                count,
                FLOAT,
                0,
                mem::size_of::<Vertex>() as i32,
                offset as *const c_void,
            );
        }
        (gl.draw_elements)(0x0004, index_count, UNSIGNED_SHORT, ptr::null());
        check(gl)
    }

    /// Deletes every owned GL resource. Handles from this renderer then expire.
    /// # Safety
    /// The context and function-pointer requirements of [`Renderer::new`] still apply.
    pub unsafe fn destroy(mut self) {
        for mesh in self.meshes.drain(..).flatten() {
            (self.gl.delete_buffers)(1, &mesh.vertex);
            (self.gl.delete_buffers)(1, &mesh.index);
        }
        for texture in self.textures.drain(..).flatten() {
            (self.gl.delete_textures)(1, &texture.id);
        }
        (self.gl.delete_buffers)(1, &self.vertex_buffer);
        (self.gl.delete_buffers)(1, &self.index_buffer);
        (self.gl.delete_program)(self.program);
    }
}

#[cfg(test)]
mod tests;
