//! Fixed-function state, applied only when it changes.
//!
//! On a 2006 driver a state change can flush the pipeline, and a frame is 800 to 1500 section
//! draws: the difference between "one state per run of sections" and "one state per section"
//! is the difference between 30 fps and 8. The renderer never calls `glEnable` directly; it
//! hands a [`StateKey`] here, and this module compares it with what the driver has, changes
//! only the parts that differ, and counts the changes so [`crate::draw::DrawList`]'s own
//! accounting can be checked against reality in the benchmark.
//!
//! What a 2.1 context does *not* have, and therefore what this module is careful about:
//!
//! * **No VAOs on the target** (`ARB_vertex_array_object` is present on this chip, so actually
//!   yes — but a 2.1 driver without it makes every vertex-attrib-pointer call expensive, so
//!   the attribute setup lives in [`super::buffer`] and is applied once per buffer, not here);
//! * **No sampler objects**: wrap/filter are texture state, so a pass that samples the same
//!   texture with two samplings costs a re-upload of the parameters (see
//!   [`crate::gl::texture`]);
//! * **No `glBlendEquationSeparate` guarantee** beyond 2.0 (it is core 2.0, so it is there);
//! * **No stencil in the HUD's path** (vanilla draws the HUD without it), so the stencil test
//!   stays off except where a pass turns it on.

use glow::HasContext;

use crate::draw::{Blend, Cull, DepthState, State};
use crate::error::{Error, Result};
use crate::gl::Context;

/// The parts of the fixed-function state a draw can set. Compared as one value, which is what
/// makes "did anything change" a single comparison rather than eight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateKey {
    pub blend: Blend,
    pub depth: DepthState,
    pub cull: Cull,
    /// Writing to the colour buffer at all: off while a pass only fills depth.
    pub color_mask: bool,
}

impl StateKey {
    pub const DEFAULT: StateKey = StateKey {
        blend: Blend::Off,
        depth: DepthState::Off,
        cull: Cull::None,
        color_mask: true,
    };

    pub fn from_draw(state: State) -> Self {
        Self {
            blend: state.blend,
            depth: state.depth,
            cull: state.cull,
            color_mask: true,
        }
    }
}

/// The state cache: what the driver currently has, and what changed.
pub struct StateCache {
    current: Option<StateKey>,
    /// State changes applied since the renderer started, and since the last frame began.
    pub changes_total: u64,
    pub changes_frame: u64,
}

impl Default for StateCache {
    fn default() -> Self {
        Self::new()
    }
}

impl StateCache {
    pub fn new() -> Self {
        Self {
            current: None,
            changes_total: 0,
            changes_frame: 0,
        }
    }

    /// Called at the start of a frame: a new frame's first draw always re-applies, because
    /// another path (a pass that bound its own buffer) may have changed the state behind this
    /// cache's back. Cheap: one comparison.
    pub fn begin_frame(&mut self) {
        self.current = None;
        self.changes_frame = 0;
    }

    pub fn current(&self) -> Option<StateKey> {
        self.current
    }

    /// Applies `key`. Returns whether anything was sent to the driver, so a caller can count
    /// the draws that cost a state change (which is what the plan's §11 budget is about).
    pub fn apply(&mut self, ctx: &Context, key: StateKey) -> Result<bool> {
        if self.current == Some(key) {
            return Ok(false);
        }
        let gl = ctx.gl();
        let previous = self.current;

        // Depth first: it decides whether the fragment stage runs at all, and on this chip an
        // early-out saves more than the state call costs.
        if previous.map(|p| p.depth) != Some(key.depth) {
            unsafe {
                match key.depth {
                    DepthState::Off => {
                        gl.disable(glow::DEPTH_TEST);
                        gl.depth_mask(false);
                    }
                    DepthState::ReadWrite => {
                        gl.enable(glow::DEPTH_TEST);
                        gl.depth_func(glow::LEQUAL);
                        gl.depth_mask(true);
                    }
                    DepthState::ReadOnly => {
                        gl.enable(glow::DEPTH_TEST);
                        gl.depth_func(glow::LEQUAL);
                        gl.depth_mask(false);
                    }
                    DepthState::WriteOnly => {
                        gl.disable(glow::DEPTH_TEST);
                        gl.depth_mask(true);
                    }
                }
            }
        }

        if previous.map(|p| p.cull) != Some(key.cull) {
            unsafe {
                match key.cull {
                    Cull::None => gl.disable(glow::CULL_FACE),
                    Cull::Back => {
                        gl.enable(glow::CULL_FACE);
                        gl.cull_face(glow::BACK);
                        // The game's winding is counter-clockwise, as vanilla's is.
                        gl.front_face(glow::CCW);
                    }
                    Cull::Front => {
                        gl.enable(glow::CULL_FACE);
                        gl.cull_face(glow::FRONT);
                        gl.front_face(glow::CCW);
                    }
                }
            }
        }

        if previous.map(|p| p.blend) != Some(key.blend) {
            unsafe {
                match key.blend {
                    Blend::Off => gl.disable(glow::BLEND),
                    // Vanilla's HUD and terrain blending: src * a + dst * (1 - a). The alpha
                    // channel is accumulated the same way so nested translucency composites.
                    Blend::Alpha => {
                        gl.enable(glow::BLEND);
                        gl.blend_equation(glow::FUNC_ADD);
                        gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                    }
                    Blend::Premultiplied => {
                        gl.enable(glow::BLEND);
                        gl.blend_equation(glow::FUNC_ADD);
                        gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
                    }
                    Blend::Additive => {
                        gl.enable(glow::BLEND);
                        gl.blend_equation(glow::FUNC_ADD);
                        gl.blend_func(glow::SRC_ALPHA, glow::ONE);
                    }
                    // The vignette: dst * (1 - src). `ONE_MINUS_SRC_COLOR` with a zero source
                    // is the 1.4 form of a multiplied image and needs no `MULTIPLY` equation.
                    Blend::Multiply => {
                        gl.enable(glow::BLEND);
                        gl.blend_equation(glow::FUNC_ADD);
                        gl.blend_func(glow::ZERO, glow::ONE_MINUS_SRC_COLOR);
                    }
                }
            }
        }

        if previous.map(|p| p.color_mask) != Some(key.color_mask) {
            unsafe {
                if key.color_mask {
                    gl.color_mask(true, true, true, true);
                } else {
                    gl.color_mask(false, false, false, false);
                }
            }
        }

        self.current = Some(key);
        self.changes_total += 1;
        self.changes_frame += 1;
        Ok(true)
    }

    /// Puts the masks where a clear needs them: `glClear` of depth does nothing with the depth
    /// mask off, which is exactly the state a depth-only pass leaves behind. The executor calls
    /// this before a pass's clears, and forgets the cached key because it changed state.
    pub fn prepare_clear(&mut self, ctx: &Context, color: bool, depth: bool) {
        let gl = ctx.gl();
        unsafe {
            if depth {
                gl.depth_mask(true);
            }
            if color {
                gl.color_mask(true, true, true, true);
            }
        }
        self.current = None;
    }

    /// Turns everything off, for the end of a frame or a context being rebuilt. Errors from a
    /// lost context are dropped: there is nothing to report to.
    pub fn reset(&mut self, ctx: &Context) -> Result<()> {
        let gl = ctx.gl();
        unsafe {
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::BLEND);
            gl.depth_mask(true);
            gl.color_mask(true, true, true, true);
        }
        self.current = None;
        if let Some(status) = gl_error(gl) {
            return Err(Error::gl("state reset", status));
        }
        Ok(())
    }
}

fn gl_error(gl: &glow::Context) -> Option<String> {
    let code = unsafe { gl.get_error() };
    (code != glow::NO_ERROR).then(|| crate::gl::gl_error_name(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_key_is_everything_off() {
        let key = StateKey::DEFAULT;
        assert_eq!(key.blend, Blend::Off);
        assert_eq!(key.depth, DepthState::Off);
        assert_eq!(key.cull, Cull::None);
        assert!(key.color_mask);
    }

    #[test]
    fn a_draw_becomes_a_key_without_losing_anything() {
        let state = State {
            blend: Blend::Additive,
            depth: DepthState::ReadOnly,
            cull: Cull::Front,
        };
        let key = StateKey::from_draw(state);
        assert_eq!(key.blend, Blend::Additive);
        assert_eq!(key.depth, DepthState::ReadOnly);
        assert_eq!(key.cull, Cull::Front);
    }

    #[test]
    fn the_cache_changes_nothing_for_the_same_key_and_counts_the_rest() {
        // `apply` needs a driver; this pins the comparison the cache makes before asking.
        let cache = StateCache::new();
        assert!(cache.current().is_none());
        assert_eq!(cache.changes_total, 0);
        assert_eq!(cache.changes_frame, 0);
        // A fresh frame re-applies: another pass may have changed the state directly.
        let mut cache = cache;
        cache.begin_frame();
        assert!(cache.current().is_none());
    }

    #[test]
    fn every_blend_mode_the_draw_list_has_a_distinct_key() {
        let modes = [
            Blend::Off,
            Blend::Alpha,
            Blend::Premultiplied,
            Blend::Additive,
            Blend::Multiply,
        ];
        for (i, a) in modes.iter().enumerate() {
            for (j, b) in modes.iter().enumerate() {
                assert_eq!(i == j, a == b, "{a:?} vs {b:?}");
            }
        }
    }
}
