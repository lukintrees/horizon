//! Legacy OpenGL 2.1/2.0 renderer scaffold (PR 2a/2b, PR 3 and PR 4 stage 1).
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
//! * PR 4 — `program.rs` + `shaders/`: GLSL 110/120 sources and static
//!   validation; the example links the selected dialect.
//! * PR 5–7 — atlas, mesh converter, sections, passes, HUD.
//! * PR 8 — the `Renderer`/`Frame` facade listed in [`CONTRACT.md`](CONTRACT.md).
//!
//! The classification rules live in [`caps`] and are mirrored by
//! `tools/gl21_caps.py`, so the dev tool, the renderer and CI agree on the
//! L1 / L0 / F0 tiers.

pub mod caps;
pub mod context;
pub mod program;
pub mod smoke;
pub mod state;

pub use caps::{
    EXPECTED_LIMITS, FBO_REQUIREMENT, OPTIONAL, REQUIRED_BASE, REQUIRED_FBO, Report, Tier, classify,
};
pub use context::{
    ContextError, FboKind, GlContext, MIN_GL, MIN_GLSL, ShaderDialect, TARGET_GL, TARGET_GLSL,
};
pub use program::{
    GlslStage, SHADERS, ShaderError, ShaderSource, smoke_program, validate, validate_all,
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
        assert!(smoke_program(ShaderDialect::Glsl120).is_some());
        assert!(smoke_program(ShaderDialect::Glsl110).is_some());
        assert!(smoke_program(ShaderDialect::FixedFunction).is_none());
    }
}
