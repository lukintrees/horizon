//! Legacy OpenGL 2.1/2.0 renderer scaffold (PR 2a/2b, PR 3, PR 4, PR 5, PR 6).
//!
//! Deliberately additive: this module has no `wgpu`, `naga`, `winit`, `glow`
//! or window dependency, so adding it cannot change the default build. The
//! `gl21_clear` example (PR 3 stage 2) is compiled only with `legacy-gl` and
//! uses `glutin`/`glow` outside this module; the library module itself stays
//! dependency-free.
//! `gfx::Renderer` remains the only active renderer until PR 8 switches the
//! `Renderer` type alias in `lib.rs` and splits `game.rs` with `#[cfg]`.
//!
//! Roadmap for this module:
//! * PR 3 — `context.rs` (validated caps), `state.rs` (GL state cache) and
//!   `smoke.rs` (headless `gl21_clear` command stream). The real GLX/WGL
//!   context, `glGet*` queries and glow loader now run in the example.
//! * PR 4 — `program.rs` + `shaders/`: six GLSL 110/120 pass programs and
//!   static validation; the example links all programs in the selected dialect.
//! * PR 5 — `atlas.rs` (2D atlas with gutters/mips) and `mesh.rs`
//!   (`TerrainVertex` → GL2 vertex, atlas UV, lightmap UV, RGBA8).
//! * PR 6 — `sections.rs`, `passes.rs` and `frame.rs`: opaque/cutout/translucent
//!   sections, multi-draw groups, draw budget and a deterministic CPU frame.
//! * PR 7–8 — entities/sky/HUD/lines in the game facade and the full
//!   `Renderer`/`Frame` facade listed in [`CONTRACT.md`](CONTRACT.md).
//!
//! The classification rules live in [`caps`] and are mirrored by
//! `tools/gl21_caps.py`, so the dev tool, the renderer and CI agree on the
//! L1 / L0 / F0 tiers.

pub mod atlas;
pub mod caps;
pub mod context;
pub mod frame;
pub mod mesh;
pub mod passes;
pub mod program;
pub mod sections;
pub mod smoke;
pub mod state;

pub use atlas::{Atlas, AtlasError, MAX_ATLAS_EDGE, MAX_LAYERS, MAX_SAMPLERS};
pub use caps::{
    EXPECTED_LIMITS, FBO_REQUIREMENT, OPTIONAL, REQUIRED_BASE, REQUIRED_FBO, Report, Tier, classify,
};
pub use context::{
    ContextError, FboKind, GlContext, MIN_GL, MIN_GLSL, ShaderDialect, TARGET_GL, TARGET_GLSL,
};
pub use frame::{GOLDEN_HEIGHT, GOLDEN_WIDTH, reference_frame};
pub use mesh::{
    GL_VERTEX_BYTES, GlVertex, MeshError, OPAQUE_LAYER, TILED_UV_STEPS, TerrainVertex, UV_STEPS,
    lightmap_uv, pack_color, wrap01,
};
pub use passes::{DrawCommand, FrameError, FramePlan, MAX_DRAWS, MultiDraw, PassKind, RasterQuad};
pub use program::{
    GlslStage, PASSES, Pass, SHADERS, ShaderError, ShaderSource, all_programs, program_for,
    smoke_program, validate, validate_all,
};
pub use sections::{
    Blend, OverlayRect, PROJECTION_SCALE, Scene, Section, SectionKind, SectionMesh, color_u32,
};
pub use smoke::{CLEAR_COLOR, gl21_clear_commands, gl21_clear_state_stream};
pub use state::{
    GL_BLEND, GL_COLOR_BUFFER_BIT, GL_CULL_FACE, GL_DEPTH_BUFFER_BIT, GL_DEPTH_TEST,
    GL_FRAMEBUFFER, GL_GEQUAL, GL_LEQUAL, GL_ONE_MINUS_SRC_ALPHA, GL_SCISSOR_TEST, GL_SRC_ALPHA,
    GL_TEXTURE_2D, GL_TRIANGLES, GL_UNSIGNED_SHORT, GlCommand, GlState,
};

/// Zero-sized placeholder for `gfx::Frame`. PR 8 replaces it with the real
/// per-frame data listed in `CONTRACT.md`; the PR 2b crate-root alias only
/// points here while the legacy build is being wired up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame;

/// Placeholder for the PR 8 facade.
///
/// The real type must expose every field and method `game.rs` reads from
/// `gfx::Renderer`; `CONTRACT.md` is the generated checklist. Until PR 8 this
/// is a zero-sized marker only, so no code can accidentally depend on a
/// half-implemented renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Renderer;

impl Renderer {
    pub fn unavailable() -> &'static str {
        "legacy GL 2.1 renderer is not wired into the game yet (PR 8)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_has_no_dependencies_on_the_wgpu_renderer() {
        assert!(Renderer::unavailable().contains("PR 8"));
        assert_eq!(Tier::L1.name(), "L1");
        assert!(!Tier::Unsupported.usable());
        assert_eq!(std::mem::size_of::<Frame>(), 0);
        assert_eq!(std::mem::size_of::<Renderer>(), 0);
    }

    #[test]
    fn contract_file_is_present() {
        // A missing checklist is a review failure, not a runtime failure.
        let source = include_str!("CONTRACT.md");
        assert!(source.contains("`renderer.<name>`"));
        assert!(source.contains("PR 8"));
        assert!(source.contains("PR 2b"));
        assert!(source.contains("PR 4"));
        assert!(source.contains("PR 5"));
        assert!(source.contains("PR 6"));
    }

    #[test]
    fn headless_smoke_stream_is_reachable() {
        let mut extensions: Vec<String> = REQUIRED_BASE
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        extensions.extend(REQUIRED_FBO.iter().map(|name| (*name).to_owned()));
        let mut limits = std::collections::BTreeMap::new();
        limits.insert("GL_MAX_TEXTURE_SIZE".to_owned(), 4096_u64);
        let ctx =
            GlContext::from_report(classify(Some((2, 1)), Some(120), &extensions, limits)).unwrap();
        let commands = gl21_clear_state_stream(&ctx, 800, 600);
        assert!(!commands.is_empty());
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, GlCommand::DrawRangeElements { .. }))
        );
    }

    #[test]
    fn shipped_glsl_sources_are_reachable() {
        assert_eq!(validate_all(), Ok(()));
        assert_eq!(SHADERS.len(), 24);
        assert_eq!(all_programs(ShaderDialect::Glsl120).len(), 6);
        assert_eq!(all_programs(ShaderDialect::Glsl110).len(), 6);
        assert!(smoke_program(ShaderDialect::Glsl120).is_some());
        assert!(smoke_program(ShaderDialect::Glsl110).is_some());
        assert!(smoke_program(ShaderDialect::FixedFunction).is_none());
    }

    #[test]
    fn pr4_to_pr6_modules_are_reachable() {
        let atlas = Atlas::new(4, 16, 4).unwrap();
        assert_eq!(atlas.columns, 2);
        assert_eq!(atlas.uv_rect(1).unwrap()[0], 28.5_f32 / 48.0_f32);
        let vertex = TerrainVertex {
            pos: [0.0; 3],
            uv: [0, 0],
            layer: 1,
            light: 0x0A_14,
            color: 0xFF_FF_FF_FF,
        };
        let gl = GlVertex::from_terrain(&vertex, &atlas, UV_STEPS).unwrap();
        assert_eq!(gl.color, [255, 255, 255, 255]);
        assert_eq!(gl.light_uv, [18.0_f32 / 256.0_f32, 28.0_f32 / 256.0_f32]);
        let plan = reference_frame();
        assert_eq!(plan.vertices.len(), 12);
        assert_eq!(plan.multi_draws.len(), 3);
        assert_eq!(plan.draw_call_count(), 5);
        assert!(plan.draw_call_count() <= MAX_DRAWS);
        assert_eq!(plan.rasterize(GOLDEN_WIDTH, GOLDEN_HEIGHT).len(), 19_200);
    }
}
