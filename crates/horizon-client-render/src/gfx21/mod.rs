//! Legacy OpenGL 2.1/2.0 renderer scaffold (PR 2a/2b, PR 3, PR 4, PR 5, PR 6,
//! PR 7, PR 8, PR 9; PR 8 full integration).
//!
//! Deliberately additive: this module has no `wgpu`, `naga`, `winit`, `glow`
//! or window dependency, so adding it cannot change the default build. The
//! `gl21_clear` example (PR 3 stage 2) is compiled only with `legacy-gl` and
//! uses `glutin`/`glow` outside this module; the library module itself stays
//! dependency-free.
//! `gfx::Renderer` remains the only active renderer: PR 8 ships an additive
//! `gfx21::Renderer` facade and settings, while the `#[cfg]` split of
//! `game.rs` and the removal of `modern-gl` stay out of this continuation.
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
//! * PR 7 — `blend.rs`, `entities.rs`, `sky.rs`, `hud.rs`, `lines.rs` and
//!   `render.rs`: full blend table, CPU posing, readable HUD and a combined
//!   reference plan without touching the PR 6 golden frame.
//! * PR 8 — `facade.rs`: the contract field/method surface, zero-sized
//!   `Frame`/`Renderer` and `LegacyGraphicsApi`/`LegacySettings`/`LegacyArgs`.
//! * PR 9 — `diagnostics.rs`: startup report, budget checks and a debug
//!   `glGetError` log; no wgpu image diff is attempted here.
//! * PR 10 — docs, packaging notes and the additive CI/report fixtures.
//!
//! The classification rules live in [`caps`] and are mirrored by
//! `tools/gl21_caps.py`, so the dev tool, the renderer and CI agree on the
//! L1 / L0 / F0 tiers.

pub mod atlas;
pub mod blend;
pub mod caps;
pub mod context;
pub mod diagnostics;
pub mod entities;
pub mod facade;
pub mod frame;
pub mod hud;
pub mod lines;
pub mod mesh;
pub mod passes;
pub mod program;
pub mod render;
pub mod sections;
pub mod sky;
pub mod smoke;
pub mod state;

pub use atlas::{Atlas, AtlasError, MAX_ATLAS_EDGE, MAX_LAYERS, MAX_SAMPLERS};
pub use blend::{BlendMode, BlendState, CUTOUT_ALPHA_REF, SortState};
pub use caps::{
    EXPECTED_LIMITS, FBO_REQUIREMENT, OPTIONAL, REQUIRED_BASE, REQUIRED_FBO, Report, Tier, classify,
};
pub use context::{
    ContextError, FboKind, GlContext, MIN_GL, MIN_GLSL, ShaderDialect, TARGET_GL, TARGET_GLSL,
    classify_strings, parse_extensions, parse_gl_version, parse_glsl_version,
};
pub use diagnostics::{
    DebugGlErrorLog, DiagnosticReport, GL_INVALID_ENUM, GL_INVALID_OPERATION, GL_INVALID_VALUE,
    GL_NO_ERROR, GL_OUT_OF_MEMORY, GL_STACK_OVERFLOW, GL_STACK_UNDERFLOW, MAX_VRAM_MIB,
    UNAVAILABLE_METHODS, gl_error_name, unavailable_methods,
};
pub use entities::{EntityBatch, EntityCamera, EntityDraw, EntityError, EntityPose};
pub use facade::{Frame, LegacyArgs, LegacyGraphicsApi, LegacySettings, Renderer};
pub use frame::{GOLDEN_HEIGHT, GOLDEN_WIDTH, reference_frame};
pub use hud::{HudBatch, HudElement};
pub use lines::{LineBatch, LineSegment};
pub use mesh::{
    GL_VERTEX_BYTES, GlVertex, MeshError, OPAQUE_LAYER, TILED_UV_STEPS, TerrainVertex, UV_STEPS,
    lightmap_uv, pack_color, wrap01,
};
pub use passes::{DrawCommand, FrameError, FramePlan, MAX_DRAWS, MultiDraw, PassKind, RasterQuad};
pub use program::{
    GlslStage, PASSES, Pass, SHADERS, ShaderError, ShaderSource, all_programs, program_for,
    smoke_program, validate, validate_all,
};
pub use render::FullFramePlan;
pub use sections::{
    Blend, OverlayRect, PROJECTION_SCALE, Scene, Section, SectionKind, SectionMesh, color_u32,
};
pub use sky::SkyState;
pub use smoke::{CLEAR_COLOR, gl21_clear_commands, gl21_clear_state_stream};
pub use state::{
    GL_ALPHA_TEST, GL_BLEND, GL_COLOR_BUFFER_BIT, GL_CULL_FACE, GL_DEPTH_BUFFER_BIT, GL_DEPTH_TEST,
    GL_DST_COLOR, GL_FRAMEBUFFER, GL_GEQUAL, GL_GREATER, GL_LEQUAL, GL_ONE, GL_ONE_MINUS_DST_COLOR,
    GL_ONE_MINUS_SRC_ALPHA, GL_ONE_MINUS_SRC_COLOR, GL_SCISSOR_TEST, GL_SRC_ALPHA, GL_SRC_COLOR,
    GL_TEXTURE_2D, GL_TRIANGLES, GL_UNSIGNED_SHORT, GL_ZERO, GlCommand, GlState,
};

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
    fn integration_helpers_are_reachable() {
        assert_eq!(parse_gl_version("2.1.8545 Release"), Some((2, 1)));
        assert_eq!(parse_glsl_version("1.20"), Some(120));
        assert_eq!(parse_extensions("GL_ARB_multitexture junk").len(), 1);
        assert_eq!(unavailable_methods().len(), UNAVAILABLE_METHODS.len());
        assert!(Renderer::unavailable().contains("PR 8"));
    }

    #[test]
    fn contract_file_is_present() {
        // A missing checklist is a review failure, not a runtime failure.
        let source = include_str!("CONTRACT.md");
        assert!(source.contains("`renderer.<name>`"));
        assert!(source.contains("PR 2b"));
        assert!(source.contains("PR 4"));
        assert!(source.contains("PR 5"));
        assert!(source.contains("PR 6"));
        assert!(source.contains("PR 7"));
        assert!(source.contains("PR 8"));
        assert!(source.contains("PR 9"));
        assert!(source.contains("PR 10"));
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

    #[test]
    fn pr7_to_pr10_modules_are_reachable() {
        assert_eq!(BlendMode::ALL.len(), 6);
        assert_eq!(EntityBatch::reference().draw_call_count(), 2);
        assert_eq!(HudBatch::reference().rasterize(80, 60).len(), 19_200);
        assert_eq!(LineBatch::reference().draw_call_count(), 4);
        assert_eq!(FullFramePlan::reference().draw_call_count(), 16);
        assert_eq!(LegacyGraphicsApi::ALL.len(), 5);
        assert_eq!(LegacySettings::reference().texture_budget_mib, 48);
        assert!(DiagnosticReport::reference().within_budget());
        assert_eq!(gl_error_name(GL_NO_ERROR), "GL_NO_ERROR");
    }
}
