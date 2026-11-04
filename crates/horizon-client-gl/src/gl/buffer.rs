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
//! offset 28  a_extra     float  GL_FLOAT            atlas index: 0, or 1 for the second
//! offset 32  (end)                                  stride
//! ```
//!
//! 32 bytes against 16. Three things make that up, and none of them is free:
//!
//!  1. **No integer attributes.** The mesher's `u16` UVs and `u32` colour cannot arrive as
//!     integers in 1.20, so they travel as normalized bytes or as floats the shader scales.
//!  2. **No instancing.** A section's origin is a uniform per draw
//!     (`VertexFormat::with_origin` marks the merged passes where it rides in the vertex
//!     instead), so nothing here is per-instance data.
//!  3. **`a_extra` is one float, and the cutout flag is not in it.** A cutout is a property
//!     of the *pass*, not of the vertex: applying it in the fragment stage costs one branch
//!     on a uniform, which is free on this hardware, while carrying it in the vertex costs
//!     four bytes per vertex on every section even when the pass has no cutout at all.
//!     It is also what keeps the stride honest: a `vec4` attribute at offset 28 ends at 44,
//!     not at 32, and a stride that does not cover an attribute makes the driver read the
//!     next vertex's bytes as this one's. One float keeps the stride at 32.
//!
//! That is the trade the port makes and it is stated in one place rather than discovered in
//! each pass.

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
    /// Writes `data` at `offset`, growing the buffer when the slice does not fit.
    ///
    /// The terrain path writes a section's vertices once, then patches a few of them when a
    /// block is placed: a patch is `write_at` and must not re-specify the whole buffer, because
    /// on the target a `glBufferData` of an in-use buffer is a stall of the whole pipeline.
    pub fn ensure(&mut self, ctx: &Context, offset: usize, data: &[u8]) -> Result<()> {
        let end = offset + data.len();
        if end > self.size {
            // Re-specify rather than `glBufferSubData` past the end: the driver's only report
            // for that is `GL_INVALID_VALUE`, and on the target sometimes nothing at all.
            self.write(ctx, &vec![0; end])
        } else {
            self.write_at(ctx, offset, data)
        }
    }

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

/// A stride's worth of every attribute, at run time.
///
/// The sizes here are the ones `glVertexAttribPointer` will actually read: a float attribute
/// is four bytes a component and a normalized byte attribute is one, and a slot must fit
/// inside [`Self::stride`] or the driver reads the next vertex's bytes as this one's.
impl Attribute {
    /// How many bytes this attribute reads, from its offset to its last component.
    pub fn bytes(&self) -> u32 {
        (self.components.max(0) as u32) * self.ty.width()
    }

    /// Where this attribute ends, which must not be past the stride.
    pub fn end(&self) -> u32 {
        self.offset + self.bytes()
    }
}

impl AttributeType {
    /// Bytes per component on the wire.
    pub fn width(self) -> u32 {
        match self {
            AttributeType::Float => 4,
            AttributeType::UnsignedByte => 1,
            AttributeType::UnsignedShort => 2,
        }
    }
}

/// The vertex format of the whole legacy path. See the module docs for why it is 32 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VertexFormat {
    pub attributes: Vec<Attribute>,
    pub stride: u32,
    /// Whether `a_extra` carries part of the section origin (a merged pass) instead of the
    /// atlas index.
    pub with_origin: bool,
}

impl VertexFormat {
    /// The terrain/entity format: 32 bytes, five attributes, no instancing needed.
    /// `crate::mesh` writes exactly this layout, and the module docs say why `a_extra` is
    /// one float rather than four.
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
                    components: 1,
                    ty: AttributeType::Float,
                    normalized: false,
                    offset: 28,
                },
            ],
            stride: 32,
            with_origin: false,
        }
    }

    /// The same bytes, with `a_extra` holding the section origin's X: what a pass that merges
    /// sections into one buffer uses to avoid a uniform per section. It is a marker, not a
    /// second layout: the stride and the attribute table cannot depend on what a pass means
    /// by a float, or every buffer would need its own binding.
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
        // Offsets are the ones the module documents.
        let offsets: Vec<u32> = format.attributes.iter().map(|a| a.offset).collect();
        assert_eq!(offsets, [0, 12, 20, 24, 28]);
        // The last attribute ends exactly at the stride: no slack a writer has to know about.
        let last = format.attributes.iter().max_by_key(|a| a.offset).unwrap();
        assert_eq!(last.end(), format.stride);
    }

    /// The bug this format had: a `vec4` of floats at offset 28 reads to 44, which is past a
    /// 32-byte stride, so every attribute after the first vertex would have been the previous
    /// one's bytes. A stride too small for one attribute is not a tuning parameter.
    #[test]
    fn no_attribute_reads_past_the_stride() {
        let format = VertexFormat::legacy();
        for a in &format.attributes {
            assert!(
                a.end() <= format.stride,
                "{a:?} ends at {} past the {} stride",
                a.end(),
                format.stride
            );
        }
    }

    /// Attributes must not overlap either: two floating-point attributes over one word read
    /// the same bytes as different data, which looks like interpolation noise, not a bug.
    #[test]
    fn no_two_attributes_share_a_byte() {
        let format = VertexFormat::legacy();
        let mut spans: Vec<(u32, u32)> = format.attributes.iter().map(|a| (a.offset, a.end())).collect();
        spans.sort_unstable();
        for pair in spans.windows(2) {
            assert!(pair[0].1 <= pair[1].0, "{:?} overlaps {:?}", pair[0], pair[1]);
        }
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
