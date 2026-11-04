//! What the GPU can do, as Horizon asks it: the table the whole legacy path is decided by.
//!
//! Three rules, learned from the target GPU (a Radeon X1650 on Catalyst 9.3, which reports
//! `2.1.8545 Release`, GLSL `1.20`, and none of `ARB_texture_array`,
//! `ARB_uniform_buffer_object`, `ARB_instanced_arrays`, `EXT_color_buffer_float` or compute):
//!
//! 1. **Ask, never assume.** The same driver version reports different extension lists on
//!    different chips, and Mesa's llvmpipe reports a 4.5 superset that would hide every bug
//!    this crate exists to avoid.
//! 2. **A capability is a table of booleans, not a version number.** `glGenVertexArrays`
//!    exists on 2.1 through `ARB_vertex_array_object` and on 3.0 in core; code asks
//!    [`Caps::vertex_array_object`], never "is this 3.0".
//! 3. **Strict mode turns the table into a contract.** With `HORIZON_GL21_STRICT` the table
//!    is the X1650's, whatever the driver reports, so llvmpipe cannot stand in for a
//!    feature the target lacks; a shader or a pass that needs one is refused instead of
//!    silently working here and failing there.

use std::collections::BTreeSet;

/// A capability the renderer may ask for. Every use of one goes through [`Caps::has`], which
/// is what lets strict mode refuse it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Feature {
    // --- GL 1.5 / 2.0, in the core of a 2.1 context ------------------------
    /// `GL_ARB_vertex_buffer_object` (1.5): every mesh is a VBO.
    VertexBufferObject,
    /// `GL_ARB_occlusion_query` (1.5): occlusion queries.
    OcclusionQuery,
    /// `glDrawRangeElements` (1.2) without a clipped index buffer.
    DrawRangeElements,
    /// `glBlendEquationSeparate` (2.0): additive and reverse-subtract for glint, OIT-free.
    BlendEquationSeparate,
    /// `GL_EXT_blend_func_separate`: separate alpha function, used by the HUD.
    BlendFuncSeparate,

    // --- 2.0 / 2.1, through extensions --------------------------------------
    /// `GL_ARB_shader_objects` + `ARB_vertex_shader` + `ARB_fragment_shader`: GLSL 1.10/1.20.
    ShaderObjects,
    /// `GL_EXT_framebuffer_object` or `ARB_framebuffer_object`: render to a texture.
    FramebufferObject,
    /// `GL_EXT_framebuffer_blit`: copying between FBOs, used for the menu blur.
    FramebufferBlit,
    /// `GL_EXT_framebuffer_multisample`: MSAA render targets. Off by default: 4x at 1080p on
    /// a 2006 chip is not free.
    FramebufferMultisample,
    /// `GL_ARB_half_float_pixel`: 16-bit float textures as sources (not as targets).
    HalfFloatTextures,
    /// `GL_EXT_texture_sRGB`: sRGB texture formats, which is how gamma stays correct without
    /// `GL_FRAMEBUFFER_SRGB` (a 3.0 feature the target does not have).
    SrgbTextures,
    /// `GL_EXT_texture_filter_anisotropic`.
    AnisotropicFilter,
    /// `GL_EXT_texture_compression_s3tc`: DXT1/3/5, for resource packs that ship them.
    S3tc,
    /// `GL_ARB_vertex_array_object`: VAOs, which make the state cache cheap. Present in the
    /// core only from 3.0, and only through this extension on 2.1.
    VertexArrayObject,
    /// `GL_EXT_multi_draw_arrays`: several draws in one call. On the target this is not an
    /// optimisation but a requirement: 800-1500 sections a frame at 20-60 us per call on a
    /// 2006 driver is 30-60 ms of CPU before anything is drawn.
    MultiDrawArrays,
    /// `GL_ARB_draw_instanced` (no per-instance divisor): one buffer, several instances.
    DrawInstanced,
    /// `GL_ARB_instanced_arrays`: `glVertexAttribDivisor`. Absent on the target.
    InstancedArrays,
    /// `GL_ARB_pixel_buffer_object`: `glReadPixels` into a buffer, for screenshots.
    PixelBufferObject,
    /// `GL_ARB_map_buffer_range`: mapping a sub-range of a buffer without copying it.
    MapBufferRange,
    /// `GL_ARB_shader_texture_lod` (2.1 core is `EXT`): `texture2DGradARB`, needed for mip
    /// selection that matches the wgpu path when derivatives are not the answer.
    ShaderTextureLod,
    /// `GL_ARB_texture_non_power_of_two` (2.0 core): NPOT atlases, so no atlas padding.
    TextureNpot,
    /// `GL_ARB_depth_texture` (1.4 core): depth as a sampleable texture (shadow maps).
    DepthTexture,
    /// `GL_EXT_packed_depth_stencil`: `GL_DEPTH24_STENCIL8` renderbuffers.
    PackedDepthStencil,
    /// `GL_ARB_point_sprite` (2.0 core): particles as points without a quad per particle.
    PointSprite,
    /// `WGL_EXT_swap_control` / `GLX_EXT_swap_control` / `EGL_EXT_swap_control`: VSync.
    SwapControl,
    /// `GL_EXT_fog_coord` or per-pixel fog in the fragment shader (this renderer always does
    /// the latter; the feature exists so a pass can ask instead of assuming).
    Fog,

    // --- present on the target, absent elsewhere, and vice versa ------------
    // The rest of the list is here to be *refused* by the target, and to make that refusal
    // a compile-time-visible fact in code and a static one in tools/gl21_budget.py.
    /// Uniform buffer objects (3.1 or `ARB_uniform_buffer_object`). Absent on the target:
    /// every uniform is a loose `glUniform*` location.
    UniformBuffers,
    /// Shader storage buffers (4.3 or `ARB_shader_storage_buffer_object`). Absent.
    StorageBuffers,
    /// `GL_ARB_texture_storage`, immutable texture storage. Absent.
    TextureStorage,
    /// Array textures (`EXT_texture_array`). Absent: a sprite "layer" is a UV offset.
    TextureArrays,
    /// `gl_VertexID` (3.0). Absent: index-free draws need a vertex buffer.
    VertexId,
    /// `gl_InstanceID` (3.1). Absent.
    InstanceId,
    /// `gl_FragDepth` (2.0 core, present, but it disables early-z on this chip; the feature
    /// is listed so passes that would use it must ask).
    FragDepth,
    /// `GLARB_texture_multisample` sampler2DMS. Absent.
    MultisampleTextures,
    /// Float render targets (`EXT_color_buffer_float` / `ARB_color_buffer_float`). Absent,
    /// which is why HDR output, TAA history and FSR cannot run on this path.
    FloatRenderTargets,
    /// Integer render targets. Absent.
    IntegerRenderTargets,
    /// 3D textures sampled per layer from a shader. 3D textures exist (1.2), but the paths
    /// that need them (OIT, visbuffer) need storage views that do not.
    Texture3D,
    /// Compute shaders (4.3). Absent: 27 entry points in the wgpu path have CPU or
    /// render-pass replacements (docs/gl21-x1650-plan.ru.md §8.2).
    ComputeShaders,
    /// Indirect draws (4.0). Absent.
    IndirectDraw,
    /// Multi-draw *indirect* count (4.6). Absent.
    MultiDrawIndirectCount,
    /// `GL_EXT_framebuffer_sRGB` for window sRGB conversion (0 on the target).
    FramebufferSrgb,
    /// `GL_ARB_timer_query`, GPU timers. Absent, and on a 2006 driver they stall anyway.
    TimerQuery,
    /// `GL_ARB_shader_subgroup*`. Absent.
    Subgroups,
    /// `GL_KHR_debug`. Absent on 9.3: there is no driver callback, only `glGetError`.
    DebugOutput,
    /// `GL_ARB_sync` (3.2), fences. Absent: submission is synchronous.
    Fences,
}

impl Feature {
    /// Every feature, for iteration in the CLI's report and in tests.
    pub const ALL: &'static [Feature] = &[
        Feature::VertexBufferObject,
        Feature::OcclusionQuery,
        Feature::DrawRangeElements,
        Feature::BlendEquationSeparate,
        Feature::BlendFuncSeparate,
        Feature::ShaderObjects,
        Feature::FramebufferObject,
        Feature::FramebufferBlit,
        Feature::FramebufferMultisample,
        Feature::HalfFloatTextures,
        Feature::SrgbTextures,
        Feature::AnisotropicFilter,
        Feature::S3tc,
        Feature::VertexArrayObject,
        Feature::MultiDrawArrays,
        Feature::DrawInstanced,
        Feature::InstancedArrays,
        Feature::PixelBufferObject,
        Feature::MapBufferRange,
        Feature::ShaderTextureLod,
        Feature::TextureNpot,
        Feature::DepthTexture,
        Feature::PackedDepthStencil,
        Feature::PointSprite,
        Feature::SwapControl,
        Feature::Fog,
        Feature::UniformBuffers,
        Feature::StorageBuffers,
        Feature::TextureStorage,
        Feature::TextureArrays,
        Feature::VertexId,
        Feature::InstanceId,
        Feature::FragDepth,
        Feature::MultisampleTextures,
        Feature::FloatRenderTargets,
        Feature::IntegerRenderTargets,
        Feature::Texture3D,
        Feature::ComputeShaders,
        Feature::IndirectDraw,
        Feature::MultiDrawIndirectCount,
        Feature::FramebufferSrgb,
        Feature::TimerQuery,
        Feature::Subgroups,
        Feature::DebugOutput,
        Feature::Fences,
    ];

    /// The GL extension (or core version note) that provides it, for the settings screen and
    /// for a player's log when a pass is refused.
    pub fn source(self) -> &'static str {
        match self {
            Feature::VertexBufferObject => "GL_ARB_vertex_buffer_object (1.5)",
            Feature::OcclusionQuery => "GL_ARB_occlusion_query (1.5)",
            Feature::DrawRangeElements => "core 1.2",
            Feature::BlendEquationSeparate => "core 2.0",
            Feature::BlendFuncSeparate => "GL_EXT_blend_func_separate (core 1.4)",
            Feature::ShaderObjects => "GL_ARB_shader_objects + vertex/fragment_shader (2.0)",
            Feature::FramebufferObject => "GL_EXT_framebuffer_object / ARB_framebuffer_object",
            Feature::FramebufferBlit => "GL_EXT_framebuffer_blit",
            Feature::FramebufferMultisample => "GL_EXT_framebuffer_multisample",
            Feature::HalfFloatTextures => "GL_ARB_half_float_pixel",
            Feature::SrgbTextures => "GL_EXT_texture_sRGB",
            Feature::AnisotropicFilter => "GL_EXT_texture_filter_anisotropic",
            Feature::S3tc => "GL_EXT_texture_compression_s3tc",
            Feature::VertexArrayObject => "GL_ARB_vertex_array_object (core 3.0)",
            Feature::MultiDrawArrays => "GL_EXT_multi_draw_arrays",
            Feature::DrawInstanced => "GL_ARB_draw_instanced",
            Feature::InstancedArrays => "GL_ARB_instanced_arrays (core 3.3)",
            Feature::PixelBufferObject => "GL_ARB_pixel_buffer_object (core 2.1)",
            Feature::MapBufferRange => "GL_ARB_map_buffer_range (core 3.0)",
            Feature::ShaderTextureLod => "GL_ARB_shader_texture_lod",
            Feature::TextureNpot => "core 2.0",
            Feature::DepthTexture => "GL_ARB_depth_texture (core 1.4)",
            Feature::PackedDepthStencil => "GL_EXT_packed_depth_stencil",
            Feature::PointSprite => "core 2.0",
            Feature::SwapControl => "WGL_EXT_swap_control / GLX_EXT_swap_control",
            Feature::Fog => "per-pixel fog in the fragment shader",
            Feature::UniformBuffers => "GL_ARB_uniform_buffer_object (core 3.1)",
            Feature::StorageBuffers => "GL_ARB_shader_storage_buffer_object (core 4.3)",
            Feature::TextureStorage => "GL_ARB_texture_storage (core 4.2)",
            Feature::TextureArrays => "GL_EXT_texture_array (core 3.0)",
            Feature::VertexId => "gl_VertexID (core 3.0)",
            Feature::InstanceId => "gl_InstanceID (core 3.1)",
            Feature::FragDepth => "gl_FragDepth (core 2.0)",
            Feature::MultisampleTextures => "ARB_texture_multisample (core 3.2)",
            Feature::FloatRenderTargets => "GL_EXT_color_buffer_float / ARB_color_buffer_float",
            Feature::IntegerRenderTargets => "GL_EXT_texture_integer (core 3.0)",
            Feature::Texture3D => "core 1.2 textures, 4.x storage views",
            Feature::ComputeShaders => "core 4.3",
            Feature::IndirectDraw => "core 4.0",
            Feature::MultiDrawIndirectCount => "core 4.6",
            Feature::FramebufferSrgb => "GL_EXT_framebuffer_sRGB / core 3.0",
            Feature::TimerQuery => "GL_ARB_timer_query",
            Feature::Subgroups => "GL_KHR_shader_subgroup",
            Feature::DebugOutput => "GL_KHR_debug",
            Feature::Fences => "GL_ARB_sync (core 3.2)",
        }
    }

    /// Why the renderer asks for it, in one line, for the log a refused pass prints.
    pub fn why(self) -> &'static str {
        match self {
            Feature::VertexBufferObject => "every mesh lives in a VBO",
            Feature::OcclusionQuery => "occlusion culling, off by default",
            Feature::DrawRangeElements => "drawing a page's index range",
            Feature::BlendEquationSeparate => "additive glint and reverse-subtract fog marks",
            Feature::BlendFuncSeparate => "the HUD's separate alpha",
            Feature::ShaderObjects => "everything: the terrain, entity and HUD programs",
            Feature::FramebufferObject => "the menu blur and any pass that renders to a texture",
            Feature::FramebufferBlit => "the menu blur's copy",
            Feature::FramebufferMultisample => "MSAA targets, off by default",
            Feature::HalfFloatTextures => "16-bit float textures a resource pack supplies",
            Feature::SrgbTextures => "gamma-correct textures without a sRGB framebuffer",
            Feature::AnisotropicFilter => "the video setting's Anisotropic filtering",
            Feature::S3tc => "DXT-compressed resource pack textures",
            Feature::VertexArrayObject => "cheap state changes between draws",
            Feature::MultiDrawArrays => "one call for 800-1500 sections",
            Feature::DrawInstanced => "repeated geometry without a divisor",
            Feature::InstancedArrays => "per-instance attributes",
            Feature::PixelBufferObject => "asynchronous screenshots",
            Feature::MapBufferRange => "streaming vertex data",
            Feature::ShaderTextureLod => "explicit mip selection",
            Feature::TextureNpot => "atlases that are not a power of two",
            Feature::DepthTexture => "sampling depth",
            Feature::PackedDepthStencil => "a depth+stencil renderbuffer",
            Feature::PointSprite => "particles",
            Feature::SwapControl => "VSync",
            Feature::Fog => "distance fog",
            Feature::UniformBuffers => "wgpu-походный стиль uniform'ов (не используется здесь)",
            Feature::StorageBuffers => "GPU-side buffers (not on this path)",
            Feature::TextureStorage => "immutable textures",
            Feature::TextureArrays => "one sampler per sprite array",
            Feature::VertexId => "index-free fullscreen quads",
            Feature::InstanceId => "per-instance identity",
            Feature::FragDepth => "writing depth from a shader",
            Feature::MultisampleTextures => "reading an MSAA target",
            Feature::FloatRenderTargets => "HDR, TAA history, FSR",
            Feature::IntegerRenderTargets => "visbuffer and frame generation ids",
            Feature::Texture3D => "OIT and visbuffer volumes",
            Feature::ComputeShaders => "GPU culling, skinning, texture animation",
            Feature::IndirectDraw => "flat/multi-draw and GPU culling",
            Feature::MultiDrawIndirectCount => "translucent batching",
            Feature::FramebufferSrgb => "automatic sRGB conversion",
            Feature::TimerQuery => "GPU frame times",
            Feature::Subgroups => "shader packs",
            Feature::DebugOutput => "driver error messages",
            Feature::Fences => "asynchronous submission",
        }
    }
}

/// The target GPU: what a Radeon X1650 with Catalyst 9.3 reports, feature by feature.
///
/// This is the table strict mode enforces, and the one `tools/gl21_budget.py`'s
/// `gl21-x1650` profile is built from. Anything not listed here is absent, by design: a new
/// feature added to [`Feature`] is absent on the target until someone measures otherwise.
pub const X1650: &[(Feature, bool)] = &[
    // Present (measured on the chip, or core in 2.1).
    (Feature::VertexBufferObject, true),
    (Feature::OcclusionQuery, true),
    (Feature::DrawRangeElements, true),
    (Feature::BlendEquationSeparate, true),
    (Feature::BlendFuncSeparate, true),
    (Feature::ShaderObjects, true),
    (Feature::FramebufferObject, true),
    (Feature::FramebufferBlit, true),
    (Feature::FramebufferMultisample, true),
    (Feature::HalfFloatTextures, true),
    (Feature::SrgbTextures, true),
    (Feature::AnisotropicFilter, true),
    (Feature::S3tc, true),
    (Feature::VertexArrayObject, true),
    (Feature::MultiDrawArrays, true),
    (Feature::DrawInstanced, false), // ARB_draw_instanced is not in the 112 extensions
    (Feature::InstancedArrays, false),
    (Feature::PixelBufferObject, true),
    (Feature::MapBufferRange, true),
    (Feature::ShaderTextureLod, true),
    (Feature::TextureNpot, true),
    (Feature::DepthTexture, true),
    (Feature::PackedDepthStencil, true),
    (Feature::PointSprite, true),
    (Feature::SwapControl, true),
    (Feature::Fog, true),
    // Absent.
    (Feature::UniformBuffers, false),
    (Feature::StorageBuffers, false),
    (Feature::TextureStorage, false),
    (Feature::TextureArrays, false),
    (Feature::VertexId, false),
    (Feature::InstanceId, false),
    (Feature::FragDepth, true),
    (Feature::MultisampleTextures, false),
    (Feature::FloatRenderTargets, false),
    (Feature::IntegerRenderTargets, false),
    (Feature::Texture3D, false),
    (Feature::ComputeShaders, false),
    (Feature::IndirectDraw, false),
    (Feature::MultiDrawIndirectCount, false),
    (Feature::FramebufferSrgb, false),
    (Feature::TimerQuery, false),
    (Feature::Subgroups, false),
    (Feature::DebugOutput, false),
    (Feature::Fences, false),
];

/// OpenGL version as the driver prints it, plus what it says about its shading language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    /// The string as returned by `glGetString(GL_VERSION)`, kept whole for the log: on the
    /// target it is `2.1.8545 Release`, whose fourth number is the vendor's build.
    pub raw: String,
    /// `true` for an OpenGL ES context (Android, ANGLE). Nothing in this crate supports one
    /// yet; a context that reports it is refused with a message that says so.
    pub embedded: bool,
}

impl Version {
    /// Parses what drivers print: `2.1.8545 Release`, `4.5 (Core Profile) Mesa 24.0.9`,
    /// `OpenGL ES 3.2 Mesa 24.0.9`, `2.1 Mesa 22.0.5`. Never fails: a string it cannot read
    /// gives 0.0, which every check treats as too old, with the raw string in the log.
    pub fn parse(raw: &str) -> Self {
        let mut s = raw.trim();
        let embedded = s.starts_with("OpenGL ES") || s.starts_with("OpenGL ES-CM");
        if let Some(rest) = s.strip_prefix("OpenGL ES-CM ") {
            s = rest;
        } else if let Some(rest) = s.strip_prefix("OpenGL ES ") {
            s = rest;
        }
        let mut it = s.split(|c: char| !(c.is_ascii_digit() || c == '.'));
        let mut numbers = Vec::new();
        for part in &mut it {
            if part.is_empty() {
                continue;
            }
            for n in part.split('.').filter(|n| !n.is_empty()) {
                match n.parse::<u32>() {
                    Ok(v) => numbers.push(v),
                    Err(_) => break,
                }
                if numbers.len() == 2 {
                    break;
                }
            }
            if numbers.len() == 2 {
                break;
            }
        }
        Self {
            major: numbers.first().copied().unwrap_or(0),
            minor: numbers.get(1).copied().unwrap_or(0),
            raw: raw.to_string(),
            embedded,
        }
    }

    /// `2.1` for the log and the settings screen.
    pub fn short(&self) -> String {
        format!("{}.{}", self.major, self.minor)
    }

    pub fn at_least(&self, major: u32, minor: u32) -> bool {
        (self.major, self.minor) >= (major, minor)
    }

    /// The GLSL version a shader may declare (`#version 120`, `#version 330`).
    ///
    /// Not a guess from the GL version: the driver reports `GL_SHADING_LANGUAGE_VERSION`
    /// separately, and on old ATI parts it lags (`1.20` on a `2.1` context, which is the
    /// combination the target has).
    pub fn glsl(glsl_major: u32, glsl_minor: u32) -> u32 {
        glsl_major * 100 + glsl_minor * 10
    }
}

/// Everything the GL context reported, as the renderer needs it. Built by [`Caps::detect`],
/// replaced wholesale by [`Caps::strict`] under `HORIZON_GL21_STRICT`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caps {
    pub version: Version,
    /// `GL_SHADING_LANGUAGE_VERSION`, verbatim (`1.20`, `3.30 - Build 31.0.101...`).
    pub glsl: String,
    pub vendor: String,
    pub renderer: String,
    /// Every `GL_*` extension, sorted, for `has` and for the log.
    pub extensions: BTreeSet<String>,
    features: BTreeSet<Feature>,
    /// The limits the renderer plans against, not the ones the driver reports: strict mode
    /// uses the target's, so a plan that assumes 4096 texels fails on llvmpipe too.
    pub limits: Limits,
    /// `HORIZON_GL21_STRICT` was set: the table is the target's and refusals are errors.
    pub strict: bool,
}

/// The hard numbers behind the feature table. Same source as the flags: the GLview dump of
/// the chip (docs/gl21-x1650-plan.ru.md §1, Appendix C).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_texture_size: u32,
    pub max_3d_texture_size: u32,
    pub max_cube_map_size: u32,
    pub max_renderbuffer_size: u32,
    pub max_color_attachments: u32,
    pub max_texture_units: u32,
    pub max_vertex_attribs: u32,
    pub max_vertex_uniform_components: u32,
    pub max_fragment_uniform_components: u32,
    pub max_varying_floats: u32,
    /// Number of `texture` units a vertex shader may sample from. **Zero on the target**:
    /// this is the one limit that changes shader design rather than shader size.
    pub max_vertex_texture_units: u32,
    pub max_viewport_width: u32,
    pub max_viewport_height: u32,
    /// Shader instruction slots per stage (SM 3.0: 512 native ALU, 512 native tex).
    pub max_instructions: Option<u32>,
}

impl Limits {
    /// The X1650's numbers, from the driver's own report.
    pub const X1650: Limits = Limits {
        max_texture_size: 4096,
        max_3d_texture_size: 2048,
        max_cube_map_size: 4096,
        max_renderbuffer_size: 4096,
        max_color_attachments: 4,
        max_texture_units: 16,
        max_vertex_attribs: 16,
        max_vertex_uniform_components: 512,
        max_fragment_uniform_components: 512,
        max_varying_floats: 44,
        max_vertex_texture_units: 0,
        max_viewport_width: 4096,
        max_viewport_height: 4096,
        max_instructions: Some(512),
    };

    /// What a desktop GL 3.3 driver gives, for comparison in the log and in tests.
    pub const GL33: Limits = Limits {
        max_texture_size: 16384,
        max_3d_texture_size: 2048,
        max_cube_map_size: 16384,
        max_renderbuffer_size: 16384,
        max_color_attachments: 8,
        max_texture_units: 16,
        max_vertex_attribs: 16,
        max_vertex_uniform_components: 4096,
        max_fragment_uniform_components: 4096,
        max_varying_floats: 60,
        max_vertex_texture_units: 16,
        max_viewport_width: 16384,
        max_viewport_height: 16384,
        max_instructions: None,
    };
}

impl Caps {
    /// From what the context reported. `extensions` is `GL_EXTENSIONS` split on spaces
    /// (`glGetStringi` does not exist on 2.1, so the whole string is read at once; old
    /// drivers truncate long lists, which is one more reason everything goes through
    /// `has`).
    pub fn detect(
        version: Version,
        glsl: &str,
        vendor: &str,
        renderer: &str,
        extensions: impl IntoIterator<Item = String>,
        reported: Limits,
    ) -> Self {
        let extensions: BTreeSet<String> = extensions.into_iter().collect();
        let core = |major: u32, minor: u32| version.at_least(major, minor);
        let has = |name: &str| extensions.contains(name);

        let mut features = BTreeSet::new();
        let mut set = |feature: Feature, on: bool| {
            if on {
                features.insert(feature);
            }
        };
        set(Feature::VertexBufferObject, core(1, 5));
        set(Feature::OcclusionQuery, core(1, 5) || has("GL_ARB_occlusion_query"));
        set(Feature::DrawRangeElements, core(1, 2));
        set(Feature::BlendEquationSeparate, core(2, 0));
        set(Feature::BlendFuncSeparate, core(1, 4));
        set(
            Feature::ShaderObjects,
            core(2, 0)
                || (has("GL_ARB_shader_objects")
                    && has("GL_ARB_vertex_shader")
                    && has("GL_ARB_fragment_shader")),
        );
        set(
            Feature::FramebufferObject,
            core(3, 0) || has("GL_ARB_framebuffer_object") || has("GL_EXT_framebuffer_object"),
        );
        set(
            Feature::FramebufferBlit,
            core(3, 0) || has("GL_EXT_framebuffer_blit") || has("GL_ARB_framebuffer_object"),
        );
        set(
            Feature::FramebufferMultisample,
            core(3, 0) || has("GL_EXT_framebuffer_multisample"),
        );
        set(Feature::HalfFloatTextures, has("GL_ARB_half_float_pixel"));
        set(Feature::SrgbTextures, has("GL_EXT_texture_sRGB"));
        set(
            Feature::AnisotropicFilter,
            has("GL_EXT_texture_filter_anisotropic"),
        );
        set(
            Feature::S3tc,
            has("GL_EXT_texture_compression_s3tc") || has("GL_ARB_texture_compression"),
        );
        set(
            Feature::VertexArrayObject,
            core(3, 0)
                || has("GL_ARB_vertex_array_object")
                || has("GL_APPLE_vertex_array_object"),
        );
        set(
            Feature::MultiDrawArrays,
            has("GL_EXT_multi_draw_arrays") || has("GL_SGI_multi_draw_arrays"),
        );
        set(Feature::DrawInstanced, has("GL_ARB_draw_instanced"));
        set(
            Feature::InstancedArrays,
            core(3, 3) || has("GL_ARB_instanced_arrays"),
        );
        set(Feature::PixelBufferObject, core(2, 1));
        set(
            Feature::MapBufferRange,
            core(3, 0) || has("GL_ARB_map_buffer_range"),
        );
        set(
            Feature::ShaderTextureLod,
            core(2, 1) || has("GL_ARB_shader_texture_lod"),
        );
        set(Feature::TextureNpot, core(2, 0));
        set(Feature::DepthTexture, core(1, 4) || has("GL_ARB_depth_texture"));
        set(Feature::PackedDepthStencil, has("GL_EXT_packed_depth_stencil"));
        set(Feature::PointSprite, core(2, 0));
        set(Feature::SwapControl, true);
        set(Feature::Fog, true);
        set(
            Feature::UniformBuffers,
            core(3, 1) || has("GL_ARB_uniform_buffer_object"),
        );
        set(
            Feature::StorageBuffers,
            core(4, 3) || has("GL_ARB_shader_storage_buffer_object"),
        );
        set(Feature::TextureStorage, core(4, 2));
        set(Feature::TextureArrays, core(3, 0) || has("GL_EXT_texture_array"));
        set(Feature::VertexId, core(3, 0));
        set(Feature::InstanceId, core(3, 1));
        set(Feature::FragDepth, core(2, 0));
        set(Feature::MultisampleTextures, core(3, 2));
        set(
            Feature::FloatRenderTargets,
            has("GL_EXT_color_buffer_float") || has("GL_ARB_color_buffer_float"),
        );
        set(Feature::IntegerRenderTargets, core(3, 0));
        set(Feature::Texture3D, core(1, 2));
        set(Feature::ComputeShaders, core(4, 3));
        set(Feature::IndirectDraw, core(4, 0));
        set(Feature::MultiDrawIndirectCount, core(4, 6));
        set(Feature::FramebufferSrgb, has("GL_EXT_framebuffer_sRGB"));
        set(Feature::TimerQuery, has("GL_ARB_timer_query") || core(3, 3));
        set(Feature::Subgroups, false);
        set(Feature::DebugOutput, has("GL_KHR_debug") || has("GL_ARB_debug_output"));
        set(Feature::Fences, core(3, 2) || has("GL_ARB_sync"));

        Self {
            version,
            glsl: glsl.to_string(),
            vendor: vendor.to_string(),
            renderer: renderer.to_string(),
            extensions,
            features,
            limits: reported,
            strict: false,
        }
    }

    /// The target table, whatever the driver says: what `HORIZON_GL21_STRICT` installs over
    /// a detected context. The version strings stay the driver's (so the log says where the
    /// run happened) and are labelled with `strict: true`.
    pub fn strict_over(self) -> Self {
        let mut features = BTreeSet::new();
        for (feature, on) in X1650 {
            if *on {
                features.insert(*feature);
            }
        }
        Self {
            limits: Limits::X1650,
            features,
            strict: true,
            ..self
        }
    }

    /// Whether the renderer may use `feature` here. Every pass asks this instead of testing
    /// a version number.
    pub fn has(&self, feature: Feature) -> bool {
        self.features.contains(&feature)
    }

    /// Every feature, for a log line or the CLI's report.
    pub fn report(&self) -> String {
        let mut s = String::new();
        for f in Feature::ALL {
            let mark = if self.has(*f) { "yes" } else { "no" };
            s.push_str(&format!("{:<28} {:<6} {}\n", format!("{f:?}"), mark, f.source()));
        }
        s
    }

    /// The renderer's own requirements: below this nothing is drawn, and the player is told
    /// why in a line that names the missing piece, not the version.
    pub fn meets_minimum(&self) -> Result<(), String> {
        if self.version.embedded {
            return Err(format!(
                "OpenGL ES {} ({}) is not supported by this path yet: it needs a desktop \
                 OpenGL 2.1 context",
                self.version.short(),
                self.version.raw
            ));
        }
        let mut missing = Vec::new();
        for (feature, needed) in [
            (Feature::ShaderObjects, "GLSL shaders (OpenGL 2.0 or ARB_shader_objects)"),
            (Feature::VertexBufferObject, "vertex buffer objects"),
            (Feature::FramebufferObject, "framebuffer objects"),
            (Feature::DrawRangeElements, "OpenGL 1.2"),
            (Feature::BlendEquationSeparate, "OpenGL 2.0 blend equations"),
        ] {
            if !self.has(feature) {
                missing.push(needed);
            }
        }
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "OpenGL {} ({}) is missing {}",
                self.version.short(),
                self.renderer,
                missing.join(", ")
            ))
        }
    }

    /// A one-paragraph summary for the log: what we run on, what we may use, what we may not.
    pub fn summary(&self) -> String {
        let available = Feature::ALL.iter().filter(|f| self.has(**f)).count();
        let blocked: Vec<&str> = Feature::ALL
            .iter()
            .filter(|f| !self.has(**f))
            .map(|f| f.source())
            .take(6)
            .collect();
        format!(
            "OpenGL {} ({}; GLSL {}, {}), {available}/{} features{}. Not available: {}{}",
            self.version.short(),
            self.renderer,
            self.glsl,
            self.vendor,
            Feature::ALL.len(),
            if self.strict {
                " (strict: the X1650's table)"
            } else {
                ""
            },
            blocked.join(", "),
            if blocked.len() < Feature::ALL.len() - available {
                "…"
            } else {
                ""
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn x1650_caps() -> Caps {
        // The extension list the driver reports, abbreviated to the ones this crate reads.
        let extensions = [
            "GL_ARB_half_float_pixel",
            "GL_EXT_packed_depth_stencil",
            "GL_ARB_vertex_buffer_object",
            "GL_ARB_pixel_buffer_object",
            "GL_ARB_framebuffer_object",
            "GL_EXT_framebuffer_blit",
            "GL_EXT_framebuffer_multisample",
            "GL_EXT_texture_sRGB",
            "GL_EXT_texture_filter_anisotropic",
            "GL_EXT_texture_compression_s3tc",
            "GL_ARB_vertex_array_object",
            "GL_EXT_multi_draw_arrays",
            "GL_ARB_occlusion_query",
            "GL_ARB_depth_texture",
            "GL_ARB_shader_objects",
            "GL_ARB_vertex_shader",
            "GL_ARB_fragment_shader",
            "GL_ARB_shader_texture_lod",
            "GL_ARB_map_buffer_range",
            "GL_ARB_texture_non_power_of_two",
            "GL_EXT_blend_func_separate",
            "WGL_EXT_swap_control",
        ];
        Caps::detect(
            Version::parse("2.1.8545 Release"),
            "1.20",
            "ATI Technologies Inc.",
            "Radeon X1600/X1650 Series",
            extensions.iter().map(|s| s.to_string()),
            Limits::X1650,
        )
    }

    #[test]
    fn parses_the_way_drivers_print_versions() {
        let v = Version::parse("2.1.8545 Release");
        assert_eq!((v.major, v.minor), (2, 1));
        assert!(!v.embedded);
        assert_eq!(v.short(), "2.1");

        let v = Version::parse("4.5 (Core Profile) Mesa 24.0.9");
        assert_eq!((v.major, v.minor), (4, 5));
        assert!(!v.embedded);

        let v = Version::parse("OpenGL ES 3.2 Mesa 24.0.9");
        assert_eq!((v.major, v.minor), (3, 2));
        assert!(v.embedded);

        // Unreadable strings are old, not new: that fails closed.
        let v = Version::parse("garbage");
        assert_eq!((v.major, v.minor), (0, 0));
    }

    #[test]
    fn the_target_meets_the_minimum_and_a_gles_context_does_not() {
        assert!(x1650_caps().meets_minimum().is_ok());

        let gles = Caps::detect(
            Version::parse("OpenGL ES 3.2 Mesa"),
            "3.20",
            "Mesa",
            "llvmpipe",
            [],
            Limits::GL33,
        );
        let err = gles.meets_minimum().unwrap_err();
        assert!(err.contains("OpenGL ES"), "{err}");
    }

    #[test]
    fn what_the_target_has_and_has_not() {
        let caps = x1650_caps();
        for f in [
            Feature::ShaderObjects,
            Feature::FramebufferObject,
            Feature::MultiDrawArrays,
            Feature::VertexArrayObject,
            Feature::BlendEquationSeparate,
            Feature::SrgbTextures,
            Feature::FragDepth,
        ] {
            assert!(caps.has(f), "{f:?} should be present on the target");
        }
        for f in [
            Feature::UniformBuffers,
            Feature::StorageBuffers,
            Feature::TextureArrays,
            Feature::InstancedArrays,
            Feature::FloatRenderTargets,
            Feature::IntegerRenderTargets,
            Feature::ComputeShaders,
            Feature::IndirectDraw,
            Feature::VertexId,
            Feature::InstanceId,
            Feature::FramebufferSrgb,
            Feature::TimerQuery,
            Feature::DebugOutput,
        ] {
            assert!(!caps.has(f), "{f:?} must be absent on the target");
        }
    }

    #[test]
    fn vertex_texture_units_are_zero() {
        assert_eq!(Limits::X1650.max_vertex_texture_units, 0);
        assert!(Limits::GL33.max_vertex_texture_units > 0);
    }

    #[test]
    fn strict_mode_replaces_a_generous_driver_with_the_target() {
        let llvmpipe = Caps::detect(
            Version::parse("4.5 (Core Profile) Mesa 24.0.9"),
            "4.50",
            "Mesa",
            "llvmpipe (LLVM 15.0.6, 256 bits)",
            [
                "GL_ARB_uniform_buffer_object",
                "GL_ARB_shader_storage_buffer_object",
                "GL_EXT_texture_array",
                "GL_ARB_instanced_arrays",
                "GL_EXT_color_buffer_float",
                "GL_ARB_compute_shader",
                "GL_ARB_indirect_parameters",
                "GL_ARB_draw_indirect",
                "GL_KHR_debug",
                "GL_ARB_timer_query",
            ]
            .iter()
            .map(|s| s.to_string()),
            Limits::GL33,
        );
        assert!(llvmpipe.has(Feature::UniformBuffers));
        assert!(llvmpipe.has(Feature::ComputeShaders));

        let strict = llvmpipe.clone().strict_over();
        assert!(strict.strict);
        assert!(!strict.has(Feature::UniformBuffers));
        assert!(!strict.has(Feature::ComputeShaders));
        assert!(!strict.has(Feature::TextureArrays));
        assert!(!strict.has(Feature::InstancedArrays));
        assert_eq!(strict.limits, Limits::X1650);
        // The driver's identity is kept: the log must say llvmpipe was the machine.
        assert!(strict.renderer.contains("llvmpipe"));
    }

    #[test]
    fn every_feature_has_a_source_and_a_reason() {
        for f in Feature::ALL {
            assert!(!f.source().is_empty(), "{f:?}");
            assert!(!f.why().is_empty(), "{f:?}");
        }
        for (f, _) in X1650 {
            assert!(Feature::ALL.contains(f), "{f:?} missing from ALL");
        }
    }

    #[test]
    fn summary_names_what_is_missing() {
        let s = Caps::strict_over(x1650_caps()).summary();
        assert!(s.contains("2.1"));
        assert!(s.contains("strict"));
        assert!(s.contains("storage") || s.contains("4.3") || s.contains("not"));
    }
}
