//! The oldest hardware Horizon is asked to run on, and the switches that make any machine pretend
//! to be it.
//!
//! A Radeon X1650 (R500, 2006) is an OpenGL **2.1** card with GLSL 1.20 and none of the features
//! the renderer relies on today: no texture arrays, no uniform or storage buffers, no instancing,
//! no integer vertex attributes, no compute, no vertex texture fetch, no sampler objects. It also
//! has no driver under Windows 10, so the machine that runs it runs Windows 7 or Linux.
//! `docs/opengl21-plan.md` is the plan for the renderer that would run there (a separate one: the
//! wgpu path needs OpenGL 3.3 core, `third_party/wgpu-hal/src/gles/adapter.rs`).
//!
//! Phases 0–3 of that plan live here and change no picture: the profile is a description, and
//! `HORIZON_GL_PROFILE=x1650` makes the *existing* feature detection take its poorest paths on any
//! machine, so that the code which already degrades (CPU sprite animation, no occlusion culling,
//! no flat draws, no translucent batching, the plain vertex layout) is exercised on hardware and
//! in CI that do not have the card. Nothing here is reachable from a shader.
//!
//! The pieces of the renderer itself are pure data so that they can be tested without a GPU, and so
//! that CI — which has no OpenGL 2.1 driver — still holds them: `context` is the checklist a driver
//! has to pass and the sentences to show a player whose does not, `arena` is the one vertex buffer
//! every section shares and the ranges to upload each frame, `atlas` packs the sprites into a
//! power-of-two texture GL 2.1 can clamp, `vertex` inlines the sprite layer into a per-sprite run
//! and lays a vertex out in what GLSL 1.20 can read, `draws` works out the frame's calls and their
//! order, and `program` holds the attribute and uniform tables the driver binds the shaders by, next
//! to the shaders' own declarations, so a renamed attribute is a test failure and not a picture no
//! one can explain. `*.vert` and `*.frag` here are that renderer's shaders.
//!
//! The switches are read on every call rather than cached: the game reads them when it opens the
//! device, which is long after anything could have set them, and a typo must not be lost.
//!
//! The shaders of the renderer to come live in this folder (`*.vert`, `*.frag`, `*.glsl`) and are
//! held to the profile by `tools/gl21_shaders.py`, which `tests/test_gl21_shaders.py` runs in CI:
//! no `in`/`out`, no `layout`, no `texture()`, no bit arithmetic, no vertex texture fetch. Writing
//! one that needs a newer OpenGL is a test failure, not a surprise on a player's 2006 driver.

pub mod arena;
pub mod atlas;
pub mod backend;
pub mod context;
pub mod draws;
pub mod frame;
pub mod hud;
mod profile;
pub mod program;
pub mod renderer;
pub mod state;
pub mod vertex;

pub use arena::{Arena, Slot, Upload};
pub use atlas::Atlas;
pub use backend::{Loader, Request};
pub use context::{Capabilities, glsl_name};
pub use draws::{Pass, Plan, SectionDraw, SectionRuns, draw_calls, plan, section_runs};
pub use frame::{Draw, Frame, Run, Section};
pub use hud::{GlHudVertex, HudDraws, HudRun};
pub use profile::Profile;
pub use program::{Attribute, AttributeKind, Uniform};
pub use renderer::{Device, Program, Renderer, Texture, Viewport};
pub use state::State;
pub use vertex::{Batches, GlVertex};

/// The profile named by `HORIZON_GL_PROFILE`, if the variable is set and names one. `x1650` is
/// the only profile today. An unknown name is an error, not a silent fallback: a typo in a CI
/// job that quietly measured the full path would be worse than a failure.
pub fn named(name: &str) -> Result<Profile, String> {
    match name.trim().to_ascii_lowercase().as_str() {
        "x1650" | "r500" | "gl21" => Ok(Profile::X1650),
        other => Err(format!(
            "HORIZON_GL_PROFILE={other} names no profile; the ones there are: x1650"
        )),
    }
}

/// The profile `HORIZON_GL_PROFILE` selects, else `None` (the machine's own capabilities).
pub fn selected() -> Option<Profile> {
    selected_from(std::env::var("HORIZON_GL_PROFILE").ok().as_deref())
}

/// `selected`, from the variable's value rather than from the environment: the crate forbids
/// unsafe code, so a test cannot set one.
pub fn selected_from(profile: Option<&str>) -> Option<Profile> {
    let name = profile?;
    if name.trim().is_empty() {
        return None;
    }
    match named(name) {
        Ok(profile) => Some(profile),
        Err(e) => {
            log::warn!("Horizon: {e}");
            None
        }
    }
}

/// Whether the poorest path the existing renderer has is being forced, by `HORIZON_GL_PROFILE`
/// or by `HORIZON_GL21_PATHS=1` (the switch on its own, for a run that keeps the driver's real
/// limits for the atlas and the pipelines).
pub fn poorest_paths() -> bool {
    poorest_paths_from(
        std::env::var("HORIZON_GL_PROFILE").ok().as_deref(),
        std::env::var("HORIZON_GL21_PATHS").ok().as_deref(),
    )
}

/// `poorest_paths`, from the variables' values rather than from the environment. A profile name
/// only counts when it names a profile: a typo must not quietly change which paths a run takes.
pub fn poorest_paths_from(profile: Option<&str>, paths: Option<&str>) -> bool {
    let named_profile = profile.is_some_and(|name| named(name).is_ok());
    named_profile || paths == Some("1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_select_the_r500_profile_and_unknown_ones_fail() {
        for name in ["x1650", "R500", " gl21 "] {
            assert_eq!(named(name), Ok(Profile::X1650), "{name}");
        }
        let unknown = named("rtx4090").unwrap_err();
        assert!(unknown.contains("rtx4090"), "{unknown}");
        assert!(
            unknown.contains("x1650"),
            "the message lists the profiles: {unknown}"
        );
    }

    #[test]
    fn the_target_card_cannot_run_the_wgpu_renderer() {
        assert!(!Profile::X1650.runs_horizons_renderer());
        // The floor the renderer asks for (README: OpenGL 3.3, with texture arrays, uniform
        // buffers and instancing from GL 3.0/3.1/3.0) is what makes the card impossible, not a
        // setting.
        let floor = Profile {
            gl: (3, 3),
            texture_arrays: true,
            uniform_buffers: true,
            instancing: true,
            ..Profile::X1650
        };
        assert!(floor.runs_horizons_renderer());
    }

    /// The state of a run that sets neither variable, which is every player's run and the one
    /// the other tests in this binary share.
    #[test]
    fn the_switches_are_off_unless_set() {
        assert_eq!(selected(), None);
        assert!(!poorest_paths());
    }

    /// What each value of each variable does, an unknown profile name included: it selects
    /// nothing and forces nothing, so a typo in a CI job cannot pass silently.
    #[test]
    fn the_variables_values_select_and_force() {
        assert_eq!(selected_from(None), None);
        assert_eq!(selected_from(Some("")), None);
        assert_eq!(selected_from(Some("   ")), None);
        assert_eq!(selected_from(Some("x1650")), Some(Profile::X1650));
        assert_eq!(selected_from(Some(" RTX4090 ")), None);
        assert!(poorest_paths_from(Some("x1650"), None));
        assert!(poorest_paths_from(Some("x1650"), Some("0")));
        assert!(poorest_paths_from(None, Some("1")));
        assert!(!poorest_paths_from(None, Some("0")));
        assert!(!poorest_paths_from(None, None));
        // An unknown or empty profile forces nothing: the run keeps the machine's capabilities.
        assert!(!poorest_paths_from(Some("rtx4090"), None));
        assert!(!poorest_paths_from(Some(""), None));
    }
}
