//! The GL state each pass of a frame sets, as data, in the order it is set.
//!
//! `draws.rs` decides *what* a pass means (depth writes, blending, the alpha test's threshold); this
//! turns that into the calls `glow` makes, so that the file that owns the context is a loop over
//! these lists rather than a place where state is set by hand and forgotten. Two of the X1650's
//! habits are the reason it is written down:
//!
//! - **State leaks between passes.** `glDepthMask` stays off after the see-through pass, so the next
//!   frame's solid faces write no depth and the world is drawn inside out — a bug that looks like a
//!   driver's. Every pass lists every piece of state it depends on, not only what it changes.
//! - **The alpha test is not a shader.** GLSL 1.20 has `discard`, but a *cut-out* face is
//!   vanilla's 0.5 threshold applied after the lightmap and the fog, so it is `glAlphaFunc` with
//!   `GL_GREATER`: the same comparison, decided by fixed-function hardware that on R500 costs
//!   nothing, where a branch per fragment costs a quarter of the frame.
//!
//! The lists are compared with `Pass`'s own answers in `tests`, so the two cannot drift: a pass that
//! says it writes depth and a state list that disables `glDepthMask` is a test failure.

use crate::gl21::draws::Pass;

/// One state change, and the arguments of the GL call it stands for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// `glEnable`/`glDisable(GL_DEPTH_TEST)`.
    DepthTest(bool),
    /// `glDepthMask(GL_TRUE/GL_FALSE)`.
    DepthWrite(bool),
    /// `glEnable`/`glDisable(GL_BLEND)`.
    Blend(bool),
    /// `glBlendFunc(src, dst)`, applied only when blending is on.
    BlendFunc(Factor, Factor),
    /// `glEnable/glDisable(GL_ALPHA_TEST)` with `glAlphaFunc(GL_GREATER, threshold)`.
    AlphaTest(Option<f32>),
    /// `glEnable`/`glDisable(GL_CULL_FACE)`: vanilla's chunk renderer culls back faces, and the sky
    /// quad is seen from the inside, so it does not.
    CullFace(bool),
    /// `glDepthFunc`, as a name: the GL constant is the driver's.
    DepthFunc(&'static str),
}

/// A `glBlendFunc` factor, by the name the driver wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Factor {
    SrcAlpha,
    OneMinusSrcAlpha,
    One,
    Zero,
}

impl Factor {
    /// The GL constant's name.
    pub fn named(self) -> &'static str {
        match self {
            Factor::SrcAlpha => "GL_SRC_ALPHA",
            Factor::OneMinusSrcAlpha => "GL_ONE_MINUS_SRC_ALPHA",
            Factor::One => "GL_ONE",
            Factor::Zero => "GL_ZERO",
        }
    }
}

/// The state `pass` needs, in the order to set it: depth first, then blending, then the test the
/// fragment stage relies on.
///
/// The three passes of `draws::Pass` and why each line is there:
///
/// | call | solid | see-through | sky |
/// | --- | --- | --- | --- |
/// | `glEnable(GL_DEPTH_TEST)` | ✔ | ✔ | ✘ |
/// | `glDepthMask(true)` | ✔ | ✘ | ✘ |
/// | `glBlendFunc(SRC_ALPHA, ONE_MINUS_SRC_ALPHA)` | ✘ | ✔ | ✔ |
/// | `glAlphaFunc(GL_GREATER, 0.5 / 0.01)` | 0.5 | 0.01 | ✘ |
/// | `glCullFace` | ✔ | ✔ | ✘ |
pub fn setup(pass: Pass) -> Vec<State> {
    let mut states = vec![State::DepthTest(pass != Pass::Sky)];
    states.push(State::DepthWrite(pass.writes_depth()));
    states.push(State::DepthFunc("GL_LEQUAL"));
    states.push(State::Blend(pass.blends()));
    if pass.blends() {
        // Vanilla's `TRANSLUCENT` and the sky's gradient both use straight alpha.
        states.push(State::BlendFunc(Factor::SrcAlpha, Factor::OneMinusSrcAlpha));
    }
    let threshold = pass.alpha_test();
    states.push(State::AlphaTest((threshold > 0.0).then_some(threshold)));
    // The sky is a quad seen from the inside: culling it would draw nothing.
    states.push(State::CullFace(pass != Pass::Sky));
    states
}

/// What a frame leaves behind for the HUD and the next frame's first pass: everything a `Pass`
/// turns on is turned off, blending included, so that the 2D pass starts from a known state.
pub fn reset() -> Vec<State> {
    vec![
        State::Blend(false),
        State::AlphaTest(None),
        State::DepthTest(false),
        State::DepthWrite(true),
        State::CullFace(false),
    ]
}

/// The state a HUD run needs, from `gl21::hud::State`: the HUD draws in the window's own
/// coordinates, with no depth and no culling, and blends per run.
pub fn hud(blend: bool, alpha_test: bool, depth: bool) -> Vec<State> {
    vec![
        State::DepthTest(depth),
        State::DepthWrite(depth),
        State::CullFace(false),
        State::Blend(blend),
        State::AlphaTest(alpha_test.then_some(0.0)),
    ]
}

/// The lines that describe `states` for a log: what a crash report on a player's machine shows.
pub fn describe(states: &[State]) -> String {
    let mut out = Vec::with_capacity(states.len());
    for state in states {
        out.push(match state {
            State::DepthTest(on) => on_call("DEPTH_TEST", *on),
            State::DepthWrite(on) => format!("glDepthMask({})", u8::from(*on)),
            State::Blend(on) => on_call("BLEND", *on),
            State::BlendFunc(src, dst) => {
                format!("glBlendFunc({}, {})", src.named(), dst.named())
            }
            State::AlphaTest(Some(threshold)) => {
                format!("glAlphaFunc(GL_GREATER, {threshold})")
            }
            State::AlphaTest(None) => "glDisable(ALPHA_TEST)".to_string(),
            State::CullFace(on) => on_call("CULL_FACE", *on),
            State::DepthFunc(name) => format!("glDepthFunc({name})"),
        });
    }
    out.join("; ")
}

/// `glEnable(NAME)` or `glDisable(NAME)`.
fn on_call(name: &str, on: bool) -> String {
    let verb = if on { "glEnable" } else { "glDisable" };
    format!("{verb}({name})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_lists_are_the_passes_own_answers() {
        for pass in [Pass::Solid, Pass::Translucent, Pass::Sky] {
            let states = setup(pass);
            let has = |wanted: State| states.contains(&wanted);
            assert_eq!(
                has(State::DepthWrite(true)),
                pass.writes_depth(),
                "{pass:?} writes depth"
            );
            assert_eq!(has(State::Blend(true)), pass.blends(), "{pass:?} blends");
            assert_eq!(
                has(State::DepthTest(false)),
                pass == Pass::Sky,
                "{pass:?} depth test"
            );
            let threshold = states.iter().find_map(|state| match state {
                State::AlphaTest(threshold) => Some(*threshold),
                _ => None,
            });
            let wanted = (pass.alpha_test() > 0.0).then_some(pass.alpha_test());
            assert_eq!(threshold, Some(wanted));
        }
    }

    #[test]
    fn the_solid_pass_writes_depth_and_tests_the_vanilla_threshold() {
        let states = setup(Pass::Solid);
        assert_eq!(states[0], State::DepthTest(true));
        assert_eq!(states[1], State::DepthWrite(true));
        assert!(states.contains(&State::Blend(false)), "no blending");
        assert!(states.contains(&State::AlphaTest(Some(0.5))));
        assert!(states.contains(&State::CullFace(true)));
        // And no `glBlendFunc` at all when blending is off: it would be state set for nothing.
        let blend_func = states.iter().any(|state| matches!(state, State::BlendFunc(..)));
        assert!(!blend_func);
    }

    #[test]
    fn the_see_through_pass_blends_and_keeps_the_depth_it_tested() {
        let states = setup(Pass::Translucent);
        assert_eq!(states[0], State::DepthTest(true));
        assert_eq!(states[1], State::DepthWrite(false), "sorted back to front");
        let blend = states.iter().find(|state| matches!(state, State::BlendFunc(..)));
        let expected = State::BlendFunc(Factor::SrcAlpha, Factor::OneMinusSrcAlpha);
        assert_eq!(blend, Some(&expected));
        assert!(states.contains(&State::AlphaTest(Some(0.01))));
    }

    #[test]
    fn the_sky_is_drawn_without_a_depth_test_and_without_culling() {
        let states = setup(Pass::Sky);
        assert!(states.contains(&State::DepthTest(false)));
        assert!(states.contains(&State::DepthWrite(false)));
        assert!(states.contains(&State::Blend(true)));
        assert!(states.contains(&State::CullFace(false)), "seen from the inside");
        assert!(states.contains(&State::AlphaTest(None)), "no test at 0");
    }

    #[test]
    fn a_pass_lists_every_piece_of_state_it_depends_on() {
        // The bug this prevents: `glDepthMask(false)` left over from the see-through pass makes the
        // next frame's world draw inside out. Every pass says what the mask is.
        for pass in [Pass::Solid, Pass::Translucent, Pass::Sky] {
            let states = setup(pass);
            let mask = states
                .iter()
                .filter(|state| matches!(state, State::DepthWrite(_)))
                .count();
            assert_eq!(mask, 1, "{pass:?} sets glDepthMask exactly once");
            let test = states
                .iter()
                .filter(|state| matches!(state, State::DepthTest(_)))
                .count();
            assert_eq!(test, 1, "{pass:?} sets GL_DEPTH_TEST exactly once");
        }
    }

    #[test]
    fn the_hud_turns_the_world_state_off() {
        for state in reset() {
            match state {
                State::Blend(on) | State::DepthTest(on) | State::CullFace(on) => {
                    assert!(!on, "{state:?} is off for the HUD")
                }
                State::AlphaTest(None) => {}
                State::DepthWrite(on) => assert!(on, "the window's depth buffer is the HUD's"),
                other => panic!("{other:?} is not part of a reset"),
            }
        }
        // A plain HUD run: no depth, no culling, blending on, no alpha test.
        let plain = hud(true, false, false);
        assert!(plain.contains(&State::Blend(true)));
        assert!(plain.contains(&State::DepthTest(false)));
        assert!(plain.contains(&State::AlphaTest(None)));
        // The HUD model's own run tests depth.
        assert!(hud(true, false, true).contains(&State::DepthTest(true)));
    }

    #[test]
    fn a_state_list_describes_itself_for_a_log() {
        let line = describe(&setup(Pass::Translucent));
        assert!(line.contains("glEnable(DEPTH_TEST)"), "{line}");
        assert!(line.contains("glDepthMask(0)"), "{line}");
        assert!(line.contains("glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA)"), "{line}");
        assert!(line.contains("glAlphaFunc(GL_GREATER, 0.01)"), "{line}");
        let sky = describe(&setup(Pass::Sky));
        assert!(sky.contains("glDisable(DEPTH_TEST)"), "{sky}");
        assert!(sky.contains("glDisable(ALPHA_TEST)"), "{sky}");
    }
}
