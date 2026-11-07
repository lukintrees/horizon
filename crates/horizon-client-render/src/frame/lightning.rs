//! CPU lightning inputs. Retained GPU meshes stay in `gfx::lightning`.

/// A bolt's render state. Simulation lifetime and flashes stay in the game.
#[derive(Clone, Copy)]
pub struct Bolt {
    pub at: [f64; 3],
    pub seed: i64,
}

#[derive(Default)]
pub struct Input<'a> {
    pub bolts: &'a [Bolt],
    /// Current atlas white texel (the centre of the white sprite); a changed
    /// rect invalidates retained vertices.
    pub uv: [f32; 2],
}

/// The atlas point an untextured quad (`RenderTypes.lightning` has no
/// texture) samples: the middle of the white sprite. Its corner (`u0`, `v0`)
/// is on the sprite's border, where the atlas's nearest sampling picks the
/// neighbouring, transparent texels for most fragments.
pub fn white_uv(white: &crate::hud::Rect) -> [f32; 2] {
    [(white.u0 + white.u1) / 2.0, (white.v0 + white.v1) / 2.0]
}
