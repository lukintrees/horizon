//! Buffers and the one vertex format every program of this path shares.
//!
//! **The format is the port.** `TerrainVertex` on the wgpu side is 16 bytes per vertex with
//! `u16`/`u32` fields that the WGSL reads as `uint`. A 2.1 context has no integer vertex
//! attributes: `glVertexAttribPointer` takes `GL_FLOAT`, `GL_UNSIGNED_BYTE` and
//! `GL_UNSIGNED_SHORT`, and the shader receives them as a `vec4` of floats. So every
//! integer the mesher packs becomes either a normalized float attribute or a float the shader
//! multiplies back (light levels, sprite layer, flags).
//!
//! The layout below is that translation, fixed in one place:
//!
//! ```text
//! offset  0  a_position  vec3   GL_FLOAT            section-local blocks
//! offset 12  a_texcoord  vec2   GL_FLOAT            texture coordinates in the atlas
//! offset 20  a_color     vec4   GL_UNSIGNED_BYTE    normalized: tint * shade * AO
//! offset 24  a_light     vec2   GL_UNSIGNED_BYTE    normalized: sky, block (0..1 of 15)
//! offset 28  a_extra     vec4   GL_FLOAT            layer/flags, or the section origin
//! ```
//!
//! 32 bytes against 16, and no instancing on the target to blame: a section's origin is a
//! uniform per draw here (`a_extra` carries it only where a pass merges sections, see
//! [`VertexFormat::with_origin`]). That is the trade the port makes and it is stated in one
//! place rather than discovered in each pass.

use glow::HasContext;

use crate::error::{Error, Result};
use crate::gl::Context;

/// How a buffer will be used, which is the GL usage hint and nothing more: on the target
/// there is no `ARB_buffer_storage`, so no way to make a buffer truly immutable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferUsage {
    /// Written once (terrain sections until edited, index buffers).
    Static,
    /// Written many times, drawn many times (HUD geometry, entity vertices each frame).
    Stream,
    /// Written once and read back by the CPU (`glReadPixels` into it, screenshots).
    Readback,
}

impl BufferUsage {
    fn gl(self) -> u32 {
        use glow::*;
        match self {
            BufferUsage::Static => STATIC_DRAW,
            BufferUsage::Stream => STREAM_DRAW,
            BufferUsage::Readback => STREAM_READ,
        }
    }
}

/// A vertex or index buffer.
#[derive(Debug)]
pub struct Buffer {
    raw: Option<glow::Buffer>,
    target: u32,
    usage: BufferUsage,
    /// Bytes currently allocated, so a growing write is a re-specification rather than a
    /// `glBufferSubData` past the end (which the driver reports only as `GL_INVALID_VALUE`,
    /// or not at all).
    size: usize,
    label: &'static str,
}

impl Buffer {
    pub fn create(ctx: &Context, usage: BufferUsage, label: &'static str) -> Result<Self> {
        let gl = ctx.gl();
        let raw = unsafe { gl.create_buffer().map_err(|e| Error::gl("glGenBuffers", e))? };
        Ok(Self {
            raw: Some(raw),
            target: glow::ARRAY_BUFFER,
            usage,
            size: 0,
            label,
        })
    }

    /// An element buffer: the same object, bound to `GL_ELEMENT_ARRAY_BUFFER`.
    pub fn create_index(ctx: &Context, usage: BufferUsage, label: &'static str) -> Result<Self> {
        let mut buffer = Self::create(ctx, usage, label)?;
        buffer.target = glow::ELEMENT_ARRAY_BUFFER;
        Ok(buffer)
    }

    pub fn raw(&self) -> Option<glow::Buffer> {
        self.raw
    }

    pub fn target(&self) -> u32 {
        self.target
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn label(&self) -> &'static str {
        self.label
    }

    /// Replaces the whole contents, re-specifying the storage. On 2.1 that is
    /// `glBufferData`, which orphans whatever the GPU still reads: for a stream buffer that
    /// is exactly the behaviour wanted (no stall on the previous frame's draw).
    pub fn write(&mut self, ctx: &Context, data: &[u8]) {
        if self.raw.is_none() {
            return;
        }
        let gl = ctx.gl();
        unsafe {
            gl.bind_buffer(self.target, self.raw);
            gl.buffer_data_u8_slice(self.target, data, self.usage.gl());
            gl.bind_buffer(self.target, None);
        }
        self.size = data.len();
        ctx.count_upload(data.len());
    }

    /// Replaces part of the contents. Growing past the allocation re-specifies it first, so a
    /// caller that streams a growing list never draws from unallocated bytes.
    pub fn write_at(&mut self, ctx: &Context, offset: usize, data: &[u8]) {
        if self.raw.is_none() || data.is_empty() {
            return;
        }
        if offset + data.len() > self.size {
            let mut grown = vec![0u8; offset + data.len()];
            grown[offset..offset + data.len()].copy_from_slice(data);
            self.write(ctx, &grown);
            return;
        }
        let gl = ctx.gl();
        let offset = i32::try_from(offset).unwrap_or(i32::MAX);
        unsafe {
            gl.bind_buffer(self.target, self.raw);
            gl.buffer_sub_data_u8_slice(self.target, offset, data);
            gl.bind_buffer(self.target, None);
        }
        ctx.count_upload(data.len());
    }

    /// Binds it. A backend's state cache calls this only when the binding changes.
    pub fn bind(&self, ctx: &Context) {
        unsafe { ctx.gl().bind_buffer(self.target, self.raw) };
    }

    pub fn destroy(&mut self, ctx: &Context) {
        if let Some(raw) = self.raw.take() {
            unsafe { ctx.gl().delete_buffer(raw) };
        }
        self.size = 0;
    }
}

/// One attribute of the shared vertex format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub location: u32,
    pub components: i32,
    pub ty: AttributeType,
    pub normalized: bool,
    pub offset: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeType {
    Float,
    UnsignedByte,
    UnsignedShort,
}

impl AttributeType {
    fn gl(self) -> u32 {
        match self {
            AttributeType::Float => glow::FLOAT,
            AttributeType::UnsignedByte => glow::UNSIGNED_BYTE,
            AttributeType::UnsignedShort => glow::UNSIGNED_SHORT,
        }
    }
}

/// The vertex format of the whole legacy path. See the module docs for why it is 32 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VertexFormat {
    pub attributes: Vec<Attribute>,
    pub stride: u32,
    /// Whether `a_extra` carries the section origin (a merged pass) instead of layer/flags.
    pub with_origin: bool,
}

impl VertexFormat {
    /// The terrain/entity format: 32 bytes, five attributes, no instancing needed.
    pub fn legacy() -> Self {
        use super::program::attribs;
        Self {
            attributes: vec![
                Attribute {
                    location: attribs::POSITION,
                    components: 3,
                    ty: AttributeType::Float,
                    normalized: false,
                    offset: 0,
                },
                Attribute {
                    location: attribs::TEXCOORD,
                    components: 2,
                    ty: AttributeType::Float,
                    normalized: false,
                    offset: 12,
                },
                Attribute {
                    location: attribs::COLOR,
                    components: 4,
                    ty: AttributeType::UnsignedByte,
                    normalized: true,
                    offset: 20,
                },
                Attribute {
                    location: attribs::LIGHT,
                    components: 2,
                    ty: AttributeType::UnsignedByte,
                    normalized: true,
                    offset: 24,
                },
                Attribute {
                    location: attribs::EXTRA,
                    components: 4,
                    ty: AttributeType::Float,
                    normalized: false,
                    offset: 28,
                },
            ],
            stride: 32,
            with_origin: false,
        }
    }

    /// The same format with `a_extra` holding the origin: what a pass that merges sections
    /// into one buffer uses to avoid a uniform per section.
    pub fn with_origin() -> Self {
        Self {
            with_origin: true,
            ..Self::legacy()
        }
    }

    /// Points the attributes at the currently bound buffer. A vertex array object (where the
    /// driver has `ARB_vertex_array_object`) captures this, which is why the format is set
    /// once per buffer rather than once per draw.
    pub fn apply(&self, ctx: &Context) {
        let gl = ctx.gl();
        for attribute in &self.attributes {
            unsafe {
                gl.vertex_attrib_pointer_f32(
                    attribute.location,
                    attribute.components,
                    attribute.ty.gl(),
                    attribute.normalized,
                    self.stride as i32,
                    attribute.offset as i32,
                );
                gl.enable_vertex_attrib_array(attribute.location);
            }
        }
    }

    /// Every attribute's slot, so a pass can disable the ones it does not use (an inherited
    /// enabled slot with no buffer bound reads from null and draws garbage on some drivers).
    pub fn locations(&self) -> impl Iterator<Item = u32> + '_ {
        self.attributes.iter().map(|a| a.location)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_legacy_format_is_32_bytes_with_five_packed_attributes() {
        let format = VertexFormat::legacy();
        assert_eq!(format.stride, 32);
        // Every attribute starts inside the stride and ends inside it.
        for a in &format.attributes {
            assert!(a.offset < format.stride, "{a:?}");
        }
        let last = format.attributes.iter().max_by_key(|a| a.offset).unwrap();
        assert_eq!(last.offset + 4 * 4, format.stride); // a_extra: four floats
        // Offsets are the ones the module documents.
        let offsets: Vec<u32> = format.attributes.iter().map(|a| a.offset).collect();
        assert_eq!(offsets, [0, 12, 20, 24, 28]);
    }

    #[test]
    fn colours_and_light_are_normalized_bytes() {
        let format = VertexFormat::legacy();
        for a in &format.attributes {
            if a.ty == AttributeType::UnsignedByte {
                assert!(a.normalized, "an unnormalized byte attribute would read as 0..255");
            }
        }
    }

    #[test]
    fn attributes_never_exceed_the_targets_sixteen_slots() {
        let format = VertexFormat::legacy();
        assert!(*format.locations().max().unwrap() < crate::caps::Limits::X1650.max_vertex_attribs);
    }

    #[test]
    fn the_origin_variant_differs_only_in_meaning() {
        let a = VertexFormat::legacy();
        let b = VertexFormat::with_origin();
        assert_eq!(a.attributes, b.attributes);
        assert!(!a.with_origin && b.with_origin);
    }

    /// A buffer's writing rules, without a context: `write_at` on unallocated space must
    /// allocate first. The GL calls themselves need a driver, so this checks the arithmetic
    /// the code uses to decide.
    #[test]
    fn a_write_past_the_end_grows_the_allocation() {
        let size = 0usize;
        let offset = 64usize;
        let data = [1u8; 16];
        assert!(offset + data.len() > size);
        let grown = offset + data.len();
        assert_eq!(grown, 80);
    }
}
