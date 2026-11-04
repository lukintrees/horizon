//! Resource-owning safe renderer; GL names never cross the API boundary.
use crate::{
    Context,
    raw::{ContextHandle, Gl},
};
use horizon_render_api::{
    Clear, Draw, Frame, Image, MeshId, Pass, RenderBackend, TextureId, Vertex, relative_origin,
};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_void};
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub texture_bytes: usize,
    pub mesh_bytes: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            texture_bytes: 32 << 20,
            mesh_bytes: 32 << 20,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub texture_bytes: usize,
    pub mesh_bytes: usize,
    pub textures: usize,
    pub meshes: usize,
}
struct Texture {
    name: u32,
    size: [u32; 2],
    bytes: usize,
}
struct Mesh {
    buffers: [u32; 2],
    count: i32,
    bytes: usize,
    entity: bool,
}
struct Uniforms {
    matrix: i32,
    origin: i32,
    cutoff: i32,
    fog: i32,
    fog_end: i32,
    force_opaque: i32,
    entity: i32,
    overlay: i32,
}

pub struct Renderer<'a> {
    gl: Gl<'a>,
    program: u32,
    uniforms: Uniforms,
    textures: HashMap<TextureId, Texture>,
    meshes: HashMap<MeshId, Mesh>,
    budget: Budget,
    stats: Stats,
    max_texture: u32,
}
static IDS: AtomicU64 = AtomicU64::new(1);
fn id() -> Result<u64, String> {
    IDS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
        .map_err(|_| "resource ID space exhausted".into())
}
fn charge(used: usize, bytes: usize, limit: usize) -> Result<(), String> {
    if used.checked_add(bytes).is_none_or(|n| n > limit) {
        Err("resource budget exceeded".into())
    } else {
        Ok(())
    }
}
fn bytes(image: &Image<'_>) -> Result<usize, String> {
    let size = (image.width as usize)
        .checked_mul(image.height as usize)
        .and_then(|n| n.checked_mul(4));
    if image.width == 0 || image.height == 0 || size != Some(image.rgba.len()) {
        Err("invalid RGBA8 image dimensions/length".into())
    } else {
        Ok(image.rgba.len())
    }
}
fn rect_valid(rect: [u32; 4], size: [u32; 2]) -> bool {
    rect[2] > 0
        && rect[3] > 0
        && rect[0].checked_add(rect[2]).is_some_and(|v| v <= size[0])
        && rect[1].checked_add(rect[3]).is_some_and(|v| v <= size[1])
}
impl Renderer<'static> {
    /// Own the native context rather than borrow it. This renderer can be moved
    /// into the game/scene without a self-reference. Rc keeps it thread-confined.
    pub fn owned(context: Context, budget: Budget) -> Result<Self, String> {
        Self::from_context(ContextHandle::Owned(Rc::new(context)), budget)
    }
}
impl<'a> Renderer<'a> {
    pub fn new(context: &'a Context, budget: Budget) -> Result<Self, String> {
        Self::from_context(ContextHandle::Borrowed(context), budget)
    }
    pub fn context(&self) -> &Context {
        &self.gl.context
    }
    fn from_context(context: ContextHandle<'a>, budget: Budget) -> Result<Self, String> {
        context.make_current()?;
        // Qualify GL1 identity before requesting GL2 function pointers.
        // SAFETY: fixed GL1 ABI and live current context; strings are borrowed until copied.
        unsafe {
            let get = std::mem::transmute::<
                *const c_void,
                unsafe extern "system" fn(u32) -> *const u8,
            >(context.address(c"glGetString")?);
            let (version, renderer) = (get(0x1F02), get(0x1F01));
            if version.is_null() || renderer.is_null() {
                return Err("null GL identity".into());
            }
            Context::qualify(
                &CStr::from_ptr(version.cast()).to_string_lossy(),
                &CStr::from_ptr(renderer.cast()).to_string_lossy(),
            )?;
        }
        let gl = Gl::load(context)?;
        // SAFETY: valid GL2 integer queries with writable outputs.
        let (max_texture, attributes, units) = unsafe {
            let (mut t, mut a, mut u) = (0, 0, 0);
            (gl.get_integer)(0x0D33, &mut t);
            (gl.get_integer)(0x8869, &mut a);
            (gl.get_integer)(0x8872, &mut u);
            (t, a, u)
        };
        if max_texture < 1 || attributes < 6 || units < 2 {
            return Err("insufficient GL2 texture/attribute limits".into());
        }
        let program = program(&gl)?;
        let mut this = Self {
            gl,
            program,
            uniforms: Uniforms {
                matrix: -1,
                origin: -1,
                cutoff: -1,
                fog: -1,
                fog_end: -1,
                force_opaque: -1,
                entity: -1,
                overlay: -1,
            },
            textures: HashMap::new(),
            meshes: HashMap::new(),
            budget,
            stats: Stats::default(),
            max_texture: max_texture as u32,
        };
        // SAFETY: live linked program, NUL-terminated names, scalar uniform writes.
        unsafe {
            let location = |name: &CStr| -> Result<i32, String> {
                let at = (this.gl.uniform_location)(program, name.as_ptr());
                if at < 0 {
                    Err(format!("missing uniform {}", name.to_string_lossy()))
                } else {
                    Ok(at)
                }
            };
            this.uniforms = Uniforms {
                matrix: location(c"matrix")?,
                origin: location(c"origin")?,
                cutoff: location(c"cutoff")?,
                fog: location(c"fog")?,
                fog_end: location(c"fog_end")?,
                force_opaque: location(c"force_opaque")?,
                entity: location(c"entity_mode")?,
                overlay: location(c"overlay")?,
            };
            (this.gl.use_program)(program);
            (this.gl.uniform1i)(location(c"image")?, 0);
            (this.gl.uniform1i)(location(c"lightmap")?, 1);
        }
        this.check()?;
        Ok(this)
    }
    pub fn stats(&self) -> Stats {
        self.stats
    }
    /// Remaining tracked payload/object capacity, not actual driver VRAM. Owners
    /// can reserve space for a streamed GUI before clearing a world frame.
    pub fn remaining(&self) -> Stats {
        Stats {
            texture_bytes: self.budget.texture_bytes - self.stats.texture_bytes,
            mesh_bytes: self.budget.mesh_bytes - self.stats.mesh_bytes,
            textures: 4096 - self.stats.textures,
            meshes: 65536 - self.stats.meshes,
        }
    }

    fn check(&self) -> Result<(), String> {
        // SAFETY: current live context, no pointer parameters.
        let error = unsafe { (self.gl.get_error)() };
        if error == 0 {
            Ok(())
        } else {
            Err(format!("OpenGL error: {error:#x}"))
        }
    }
    fn upload_mesh<T>(
        &mut self,
        vertices: &[T],
        indices: &[u16],
        entity: bool,
        base: impl Fn(&T) -> &Vertex,
    ) -> Result<MeshId, String> {
        if self.stats.meshes >= 65536 {
            return Err("mesh object limit exceeded".into());
        }
        if vertices.is_empty()
            || vertices.len() > 65536
            || indices.is_empty()
            || indices.len() % 3 != 0
            || indices.len() > i32::MAX as usize
            || indices.iter().any(|&i| i as usize >= vertices.len())
            || vertices.iter().any(|value| {
                let v = base(value);
                v.pos
                    .iter()
                    .chain(&v.uv)
                    .chain(&v.light_uv)
                    .chain(&v.color)
                    .any(|n| !n.is_finite())
            })
        {
            return Err("invalid indexed triangle mesh".into());
        }
        let vertex_bytes = size_of_val(vertices);
        let index_bytes = size_of_val(indices);
        let bytes = vertex_bytes
            .checked_add(index_bytes)
            .ok_or("mesh size overflow")?;
        if vertex_bytes > isize::MAX as usize || index_bytes > isize::MAX as usize {
            return Err("GL buffer size overflow".into());
        }
        charge(self.stats.mesh_bytes, bytes, self.budget.mesh_bytes)?;
        let id = MeshId(id()?);
        self.gl.context.make_current()?;
        let mut buffers = [0; 2];
        // SAFETY: the only callers instantiate T as repr(C) Vertex/LitVertex,
        // containing only floats and no padding. Checked sizes match live arrays. GL copies client storage during BufferData.
        unsafe {
            (self.gl.gen_buffers)(2, buffers.as_mut_ptr());
            (self.gl.bind_buffer)(0x8892, buffers[0]);
            (self.gl.buffer_data)(
                0x8892,
                vertex_bytes as isize,
                vertices.as_ptr().cast(),
                0x88E4,
            );
            (self.gl.bind_buffer)(0x8893, buffers[1]);
            (self.gl.buffer_data)(
                0x8893,
                index_bytes as isize,
                indices.as_ptr().cast(),
                0x88E4,
            );
        }
        if let Err(error) = self.check().and_then(|_| {
            if buffers.contains(&0) {
                Err("GL buffer allocation failed".into())
            } else {
                Ok(())
            }
        }) {
            // SAFETY: generated names; zero entries are ignored.
            unsafe { (self.gl.delete_buffers)(2, buffers.as_ptr()) };
            return Err(error);
        }
        self.meshes.insert(
            id,
            Mesh {
                buffers,
                count: indices.len() as i32,
                bytes,
                entity,
            },
        );
        self.stats.mesh_bytes += bytes;
        self.stats.meshes += 1;
        Ok(id)
    }
    fn validate_draw(&self, draw: &Draw, frame: &Frame<'_>) -> Result<[f32; 3], String> {
        if !self.meshes.contains_key(&draw.mesh)
            || !self.textures.contains_key(&draw.texture)
            || !self.textures.contains_key(&draw.lightmap)
        {
            return Err("unknown, foreign or released mesh/texture ID".into());
        }
        if self.meshes[&draw.mesh].entity != matches!(draw.pass, Pass::Entity(_)) {
            return Err("entity mesh/pass mismatch".into());
        }
        if let Pass::Entity(material) = draw.pass {
            if material.overlay_color().is_none() {
                return Err("invalid entity overlay".into());
            }
        }
        // GL2 texture sampling state belongs to the texture, not to a sampler
        // object. One nonconstant texture cannot be both linear atlas and nearest
        // lightmap in the same draw. A 1x1 white texture is equivalent either way.
        if draw.pass == Pass::OverlayVignette
            && draw.texture == draw.lightmap
            && self.textures[&draw.texture].size != [1, 1]
        {
            return Err("vignette atlas and lightmap need separate textures".into());
        }
        if draw
            .scissor
            .is_some_and(|r| !rect_valid(r, self.gl.context.size()))
        {
            return Err("invalid scissor rectangle".into());
        }
        relative_origin(draw.origin, frame.camera).ok_or("non-finite relative origin".into())
    }
}
impl RenderBackend for Renderer<'_> {
    type Error = String;
    fn create_texture(&mut self, image: Image<'_>) -> Result<TextureId, String> {
        let bytes = bytes(&image)?;
        if self.stats.textures >= 4096 {
            return Err("texture object limit exceeded".into());
        }
        if [image.width, image.height]
            .iter()
            .any(|&n| !n.is_power_of_two() || n > self.max_texture)
        {
            return Err("texture must be power-of-two and within driver limits".into());
        }
        charge(self.stats.texture_bytes, bytes, self.budget.texture_bytes)?;
        let id = TextureId(id()?);
        self.gl.context.make_current()?;
        let mut name = 0;
        // SAFETY: sizes validated; RGBA8 storage exactly matches the readable slice.
        // Texture has one level, so NEAREST minification is complete without mips.
        unsafe {
            (self.gl.gen_textures)(1, &mut name);
            (self.gl.active_texture)(0x84C0);
            (self.gl.bind_texture)(0x0DE1, name);
            (self.gl.pixel_store)(0x0CF5, 1); // UNPACK_ALIGNMENT
            for (parameter, value) in [
                (0x2800, 0x2600),
                (0x2801, 0x2600),
                (0x2802, 0x812F),
                (0x2803, 0x812F),
            ] {
                (self.gl.tex_parameter)(0x0DE1, parameter, value);
            }
            (self.gl.tex_image)(
                0x0DE1,
                0,
                0x8058,
                image.width as i32,
                image.height as i32,
                0,
                0x1908,
                0x1401,
                image.rgba.as_ptr().cast(),
            );
        }
        if let Err(error) = self.check().and_then(|_| {
            if name == 0 {
                Err("GL texture allocation failed".into())
            } else {
                Ok(())
            }
        }) {
            // SAFETY: generated name (zero is ignored); current context.
            unsafe { (self.gl.delete_textures)(1, &name) };
            return Err(error);
        }
        self.textures.insert(
            id,
            Texture {
                name,
                size: [image.width, image.height],
                bytes,
            },
        );
        self.stats.texture_bytes += bytes;
        self.stats.textures += 1;
        Ok(id)
    }
    fn update_texture(
        &mut self,
        id: TextureId,
        at: [u32; 2],
        image: Image<'_>,
    ) -> Result<(), String> {
        bytes(&image)?;
        let texture = self.textures.get(&id).ok_or("unknown texture ID")?;
        if !rect_valid([at[0], at[1], image.width, image.height], texture.size) {
            return Err("texture update out of bounds".into());
        }
        self.gl.context.make_current()?;
        // SAFETY: validated live texture and subrectangle; slice contains all RGBA8 pixels.
        unsafe {
            (self.gl.active_texture)(0x84C0);
            (self.gl.bind_texture)(0x0DE1, texture.name);
            (self.gl.pixel_store)(0x0CF5, 1);
            (self.gl.tex_update)(
                0x0DE1,
                0,
                at[0] as i32,
                at[1] as i32,
                image.width as i32,
                image.height as i32,
                0x1908,
                0x1401,
                image.rgba.as_ptr().cast(),
            );
        }
        self.check()
    }
    fn remove_texture(&mut self, id: TextureId) -> Result<(), String> {
        self.gl.context.make_current()?;
        let texture = self.textures.remove(&id).ok_or("unknown texture ID")?;
        // SAFETY: owned live texture; its GL name is removed from the ID table first.
        unsafe { (self.gl.delete_textures)(1, &texture.name) };
        self.stats.texture_bytes -= texture.bytes;
        self.stats.textures -= 1;
        self.check()
    }
    fn create_mesh(&mut self, vertices: &[Vertex], indices: &[u16]) -> Result<MeshId, String> {
        self.upload_mesh(vertices, indices, false, |v| v)
    }
    fn create_entity_mesh(
        &mut self,
        vertices: &[horizon_render_api::entity::LitVertex],
        indices: &[u16],
    ) -> Result<MeshId, String> {
        if vertices.iter().any(|v| {
            v.back_color
                .iter()
                .chain(&v.light_color)
                .any(|f| !f.is_finite())
        }) {
            return Err("invalid entity lighting".into());
        }
        self.upload_mesh(vertices, indices, true, |v| &v.base)
    }
    fn remove_mesh(&mut self, id: MeshId) -> Result<(), String> {
        self.gl.context.make_current()?;
        let mesh = self.meshes.remove(&id).ok_or("unknown mesh ID")?;
        // SAFETY: uniquely owned buffer names, removed from the ID table first.
        unsafe { (self.gl.delete_buffers)(2, mesh.buffers.as_ptr()) };
        self.stats.mesh_bytes -= mesh.bytes;
        self.stats.meshes -= 1;
        self.check()
    }
    fn render(&mut self, frame: &Frame<'_>) -> Result<(), String> {
        if frame.view_projection.iter().any(|v| !v.is_finite())
            || frame.camera.iter().any(|v| !v.is_finite())
            || matches!(frame.clear, Clear::ColorDepth(color) if color.iter().any(|v| !v.is_finite()))
        {
            return Err("non-finite frame state".into());
        }
        if frame.fog.is_some_and(|f| {
            !f.start.is_finite()
                || !f.end.is_finite()
                || f.start >= f.end
                || f.color.iter().any(|v| !v.is_finite())
        }) {
            return Err("invalid fog range/color".into());
        }
        // Validate the entire draw list before clearing or submitting any part.
        for draw in frame.draws {
            self.validate_draw(draw, frame)?;
        }
        self.gl.context.make_current()?;
        let [width, height] = self.gl.context.size();
        // SAFETY: all handles, counts, byte offsets and uniforms below have been
        // validated. Attribute offsets refer to bound VBO storage, not CPU pointers.
        unsafe {
            (self.gl.use_program)(self.program);
            (self.gl.viewport)(0, 0, width as i32, height as i32);
            (self.gl.disable)(0x0C11); // SCISSOR_TEST before clear
            (self.gl.disable)(0x0B44); // CULL_FACE: CPU prepares the visible faces
            (self.gl.depth_mask)(1);
            (self.gl.depth_func)(0x0201); // LESS
            match frame.clear {
                Clear::Load => (),
                Clear::Depth => (self.gl.clear)(0x0100),
                Clear::ColorDepth(color) => {
                    (self.gl.clear_color)(color[0], color[1], color[2], color[3]);
                    (self.gl.clear)(0x4000 | 0x0100);
                }
            }
            (self.gl.uniform_matrix)(self.uniforms.matrix, 1, 0, frame.view_projection.as_ptr());
            for draw in frame.draws {
                let mesh = &self.meshes[&draw.mesh];
                let offset = self.validate_draw(draw, frame)?;
                (self.gl.uniform3f)(self.uniforms.origin, offset[0], offset[1], offset[2]);
                let (start, end, color) = match (frame.fog, draw.pass) {
                    (Some(f), pass) if !pass.is_overlay() => (f.start, f.end, f.color),
                    _ => (1e20, 2e20, [0.; 3]),
                };
                (self.gl.uniform4f)(self.uniforms.fog, color[0], color[1], color[2], start);
                (self.gl.uniform1f)(self.uniforms.fog_end, end);
                let cutoff = match draw.pass {
                    Pass::Cutout | Pass::Entity(_) => 0.1,
                    Pass::Overlay | Pass::OverlayAdditive | Pass::OverlayDepth => 0.004,
                    _ => 0.,
                };
                (self.gl.uniform1f)(self.uniforms.cutoff, cutoff);
                let entity = match draw.pass {
                    Pass::Entity(m) => Some(m),
                    _ => None,
                };
                (self.gl.uniform1f)(self.uniforms.entity, if entity.is_some() { 1. } else { 0. });
                let overlay = entity
                    .and_then(|m| m.overlay_color())
                    .unwrap_or([0., 0., 0., 1.]);
                (self.gl.uniform4f)(
                    self.uniforms.overlay,
                    overlay[0],
                    overlay[1],
                    overlay[2],
                    overlay[3],
                );
                (self.gl.front_face)(0x0901);
                if entity.is_some_and(|m| m.cull) {
                    (self.gl.enable)(0x0B44);
                    (self.gl.cull_face)(0x0405);
                } else {
                    (self.gl.disable)(0x0B44);
                }

                (self.gl.uniform1f)(
                    self.uniforms.force_opaque,
                    if draw.pass == Pass::OverlayVignette {
                        1.
                    } else {
                        0.
                    },
                );
                if draw.pass.is_overlay() && draw.pass != Pass::OverlayDepth {
                    (self.gl.disable)(0x0B71);
                } else {
                    (self.gl.enable)(0x0B71);
                }
                (self.gl.depth_func)(
                    if draw.pass == Pass::OverlayDepth || matches!(draw.pass, Pass::Entity(_)) {
                        0x0203
                    } else {
                        0x0201
                    },
                );
                let depth_write =
                    matches!(draw.pass, Pass::Opaque | Pass::Cutout | Pass::OverlayDepth)
                        || matches!(draw.pass, Pass::Entity(m) if !m.particle);
                (self.gl.depth_mask)(u8::from(depth_write));
                let factors = match draw.pass {
                    Pass::OverlayInvert => Some((0x0307, 0x0301, 1, 0)),
                    Pass::OverlayAdditive => Some((1, 1, 1, 1)),
                    Pass::OverlayVignette => Some((0, 0x0301, 0, 1)),
                    Pass::Overlay | Pass::OverlayDepth | Pass::Entity(_) => {
                        Some((0x0302, 0x0303, 1, 0x0303))
                    }
                    Pass::Translucent => Some((0x0302, 0x0303, 0x0302, 0x0303)),
                    _ => None,
                };
                if let Some((src, dst, src_alpha, dst_alpha)) = factors {
                    (self.gl.enable)(0x0BE2);
                    (self.gl.blend_func)(src, dst, src_alpha, dst_alpha);
                } else {
                    (self.gl.disable)(0x0BE2);
                }
                if let Some([x, y, w, h]) = draw.scissor {
                    (self.gl.enable)(0x0C11);
                    (self.gl.scissor)(x as i32, y as i32, w as i32, h as i32);
                } else {
                    (self.gl.disable)(0x0C11);
                }
                for (unit, id) in [(0x84C0, draw.texture), (0x84C1, draw.lightmap)] {
                    (self.gl.active_texture)(unit);
                    (self.gl.bind_texture)(0x0DE1, self.textures[&id].name);
                    // The credits vignette uses a linear atlas sampler. Restore
                    // nearest on every other draw, including the lightmap.
                    let filter = if unit == 0x84C0 && draw.pass == Pass::OverlayVignette {
                        0x2601
                    } else {
                        0x2600
                    };
                    (self.gl.tex_parameter)(0x0DE1, 0x2800, filter);
                    (self.gl.tex_parameter)(0x0DE1, 0x2801, filter);
                }
                (self.gl.bind_buffer)(0x8892, mesh.buffers[0]);
                (self.gl.bind_buffer)(0x8893, mesh.buffers[1]);
                for (index, count, offset) in [(0, 3, 0), (1, 2, 12), (2, 2, 20), (3, 4, 28)] {
                    (self.gl.attribute_pointer)(
                        index,
                        count,
                        0x1406,
                        0,
                        if mesh.entity { 76 } else { 44 },
                        offset as *const c_void,
                    );
                    (self.gl.enable_attribute)(index);
                }
                for (index, offset) in [(4, 44), (5, 60)] {
                    if mesh.entity {
                        (self.gl.attribute_pointer)(
                            index,
                            4,
                            0x1406,
                            0,
                            76,
                            offset as *const c_void,
                        );
                        (self.gl.enable_attribute)(index);
                    } else {
                        (self.gl.disable_attribute)(index);
                        // Array draws leave generic attribute values unspecified.
                        (self.gl.attribute4f)(index, 1., 1., 1., 1.);
                    }
                }
                (self.gl.draw_elements)(0x0004, mesh.count, 0x1403, ptr::null());
            }
        }
        self.check()
    }
    fn read_rgba(&mut self, rect: [u32; 4]) -> Result<Vec<u8>, String> {
        if !rect_valid(rect, self.gl.context.size()) {
            return Err("readback rectangle out of bounds".into());
        }
        self.gl.context.make_current()?;
        let [x, y, w, h] = rect;
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        // SAFETY: validated bounded rectangle and exactly w*h*4 writable RGBA8 bytes.
        unsafe {
            (self.gl.pixel_store)(0x0D05, 1); // PACK_ALIGNMENT
            (self.gl.read_pixels)(
                x as i32,
                y as i32,
                w as i32,
                h as i32,
                0x1908,
                0x1401,
                pixels.as_mut_ptr().cast(),
            );
        }
        self.check()?;
        Ok(pixels)
    }
}
impl Drop for Renderer<'_> {
    fn drop(&mut self) {
        // Never call through a different context if rebinding fails. The owning
        // native context will release any remaining objects when it is destroyed.
        if self.gl.context.make_current().is_err() {
            return;
        }
        // SAFETY: uniquely owned names and live context (borrow outlives renderer).
        unsafe {
            for mesh in self.meshes.values() {
                (self.gl.delete_buffers)(2, mesh.buffers.as_ptr());
            }
            for texture in self.textures.values() {
                (self.gl.delete_textures)(1, &texture.name);
            }
            (self.gl.delete_program)(self.program);
        }
    }
}
fn program(gl: &Gl<'_>) -> Result<u32, String> {
    // SAFETY: fixed shader kinds, live strings and sized writable query/log storage.
    unsafe {
        let mut shaders = Vec::new();
        for (kind, source) in [
            (0x8B31, include_str!("terrain.vert")),
            (0x8B30, include_str!("terrain.frag")),
        ] {
            let shader = (gl.create_shader)(kind);
            shaders.push(shader);
            let source = CString::new(source).unwrap();
            (gl.shader_source)(shader, 1, &source.as_ptr(), ptr::null());
            (gl.compile_shader)(shader);
            let mut compiled = 0;
            (gl.shader_integer)(shader, 0x8B81, &mut compiled);
            if compiled == 0 {
                let mut length = 0;
                (gl.shader_integer)(shader, 0x8B84, &mut length);
                let mut log = vec![0; length.max(1) as usize];
                (gl.shader_log)(shader, log.len() as i32, ptr::null_mut(), log.as_mut_ptr());
                for id in shaders {
                    (gl.delete_shader)(id);
                }
                return Err(format!(
                    "legacy shader compile failed: {}",
                    CStr::from_ptr(log.as_ptr()).to_string_lossy()
                ));
            }
        }
        let program = (gl.create_program)();
        for &shader in &shaders {
            (gl.attach_shader)(program, shader);
        }
        for (index, name) in [
            c"position",
            c"texcoord",
            c"lightcoord",
            c"color",
            c"back_color",
            c"light_color",
        ]
        .iter()
        .enumerate()
        {
            (gl.bind_attribute)(program, index as u32, name.as_ptr());
        }
        (gl.link_program)(program);
        for shader in shaders {
            (gl.delete_shader)(shader);
        }
        let mut linked = 0;
        (gl.program_integer)(program, 0x8B82, &mut linked);
        if linked == 0 {
            let mut length = 0;
            (gl.program_integer)(program, 0x8B84, &mut length);
            let mut log = vec![0; length.max(1) as usize];
            (gl.program_log)(program, log.len() as i32, ptr::null_mut(), log.as_mut_ptr());
            (gl.delete_program)(program);
            return Err(format!(
                "legacy program link failed: {}",
                CStr::from_ptr(log.as_ptr()).to_string_lossy()
            ));
        }
        Ok(program)
    }
}
const _: () = {
    assert!(size_of::<Vertex>() == 44);
    assert!(std::mem::offset_of!(Vertex, color) == 28);
};
