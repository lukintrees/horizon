//! The X1650's capabilities, as the renderer's feature detection sees them.

/// What a graphics context offers, in the terms the renderer's feature detection uses. The
/// fields are what separates a 2006 card from a 2020 one; a field absent from this list
/// (framebuffer objects, `discard`, anisotropic filtering) is something GL 2.1 has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    /// The OpenGL version the context reports, as (major, minor): 2.1 for the X1650.
    pub gl: (u8, u8),
    /// The GLSL version the driver compiles, in hundreds: 120 for GLSL 1.20.
    pub glsl: u16,
    /// `GL_MAX_TEXTURE_SIZE`. The X1650 takes 4096; the sprite atlas is built for it.
    pub max_texture_size: u32,
    /// `GL_MAX_VERTEX_ATTRIBS`: 16 is the GL 2.1 floor, 8 the smallest the terrain and entity
    /// layouts can be squeezed into.
    pub max_vertex_attributes: u32,
    /// `GL_MAX_TEXTURE_IMAGE_UNITS` in the fragment stage; the terrain shader's sprite arrays,
    /// lightmap and emission thresholds have to fit in it once they are one atlas each.
    pub max_texture_units: u32,
    /// `texture_2d_array`, `texture_cube_array`: GL 3.0 (`EXT_texture_array` in practice).
    pub texture_arrays: bool,
    /// Uniform buffers: GL 3.1 (`ARB_uniform_buffer_object`).
    pub uniform_buffers: bool,
    /// Storage buffers and images: GL 4.3 (`ARB_shader_storage_buffer_object`).
    pub storage_buffers: bool,
    /// Compute shaders: GL 4.3 (`ARB_compute_shader`).
    pub compute: bool,
    /// `DrawArraysInstanced`: GL 3.0 (`ARB_draw_instanced`); instance-step vertex attributes
    /// with it.
    pub instancing: bool,
    /// `glVertexAttribIPointer` for integer vertex attributes: GL 3.0.
    pub integer_attributes: bool,
    /// A vertex shader can sample a texture: GL 3.0. R500 has no texture unit in its vertex
    /// pipeline.
    pub vertex_texture_fetch: bool,
    /// A vertex shader can index a uniform array with a computed index: GL 3.0. On the X1650
    /// every uniform array read in a vertex shader has to be a constant index ("Max Native
    /// Address Registers: 0").
    pub dynamic_uniform_indexing: bool,
    /// `glGenSamplers` and sampler objects: GL 3.3. Without them a texture carries its
    /// filtering and wrap state.
    pub sampler_objects: bool,
    /// `GL_ARB_framebuffer_object`: render targets other than the window. GL 2.1 has it as an
    /// extension; every R500 driver has it.
    pub framebuffer_objects: bool,
    /// `gl_FragData[]` through `GL_ARB_draw_buffers`: 4 targets on the X1650.
    pub draw_buffers: u32,
    /// Non-power-of-two textures can be mipmapped: not on R300/R500, which only take NPOT with
    /// clamp-to-edge and no mips (`LimitedNPOT`). Every atlas is a power of two for that
    /// reason.
    pub npot_mipmaps: bool,
}

impl Profile {
    /// A Radeon X1650 (R500) under its last driver: Catalyst 9.3 on Windows XP/Vista/7, or
    /// Mesa's `r300g` on Linux, which still reports exactly OpenGL 2.1 for this family.
    pub const X1650: Profile = Profile {
        gl: (2, 1),
        glsl: 120,
        max_texture_size: 4096,
        max_vertex_attributes: 16,
        max_texture_units: 16,
        texture_arrays: false,
        uniform_buffers: false,
        storage_buffers: false,
        compute: false,
        instancing: false,
        integer_attributes: false,
        vertex_texture_fetch: false,
        dynamic_uniform_indexing: false,
        sampler_objects: false,
        framebuffer_objects: true,
        draw_buffers: 4,
        npot_mipmaps: false,
    };

    /// Whether this profile can run Horizon's own renderer (`gfx`), which asks for OpenGL 3.3
    /// core, texture arrays, uniform and storage buffers and instancing. Always false for the
    /// X1650: that card needs the renderer `docs/opengl21-plan.md` describes.
    pub fn runs_horizons_renderer(&self) -> bool {
        self.gl >= (3, 3) && self.texture_arrays && self.uniform_buffers && self.instancing
    }
}
