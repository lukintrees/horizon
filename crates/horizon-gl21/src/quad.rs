//! The present pass's quad, as vertices the driver uploads once.
//!
//! `present.vert` draws one quad of the window, sampling the frame texture at the same point: four
//! corners, a triangle strip, `gl_Position = (a_pos, 0, 1)`. It is the only geometry of the GL 2.1
//! renderer that is not the mesher's, so it lives here, in the crate that owns the device, and the
//! numbers are checked without a driver (`tests` at the bottom).
//!
//! Two things are worth knowing before reading it:
//!
//! - **A texture's rows are bottom-up, the window's are not.** A GL texture's first row is its
//!   bottom one, so `v_uv = a_pos * 0.5 + 0.5` samples the frame texture with v = 0 at the bottom of
//!   the window. That is the right way round for a frame the renderer drew into a texture whose own
//!   rows the framebuffer wrote bottom-up (every GL framebuffer does), so the quad's v is the frame
//!   texture's own row order, and the corners below are named for what they show.
//! - **The quad is not the world's vertex layout.** `program::PRESENT` is one `vec2`; the terrain's
//!   is four attributes over 28 bytes. A device binds one pointer layout at a time, which is why
//!   the device has a `present` call of its own rather than a `draw(0, 4)`.

use horizon_client_render::gl21;

/// One corner of the present quad: the clip-space position `present.vert` takes, and the texture
/// coordinate the same corner samples. The two are equal up to the `* 0.5 + 0.5` the shader does, so
/// they are written out rather than computed twice.
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct PresentVertex {
    /// `a_pos`: clip space, -1..1.
    pub pos: [f32; 2],
}

/// The quad's vertices, as `glDrawArrays(GL_TRIANGLE_STRIP, 0, 4)` takes them: a strip's order is
/// bottom left, bottom right, top left, top right, which is the two triangles the window needs.
pub const QUAD: [PresentVertex; 4] = [
    PresentVertex { pos: [-1.0, -1.0] },
    PresentVertex { pos: [1.0, -1.0] },
    PresentVertex { pos: [-1.0, 1.0] },
    PresentVertex { pos: [1.0, 1.0] },
];

/// The quad's bytes, for `glBufferData`.
pub fn bytes() -> &'static [u8] {
    bytemuck::cast_slice(&QUAD)
}

/// How many vertices the present call draws: the quad, as the renderer's summary counts it.
pub const VERTICES: u32 = 4;

/// The attribute layout the present program is read with: one `vec2` of floats at offset zero
/// (`program::PRESENT`).
pub const ATTRIBUTES: [gl21::Attribute; 1] = gl21::program::PRESENT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quad_covers_the_window_in_a_strip_s_own_order() {
        // A strip of four: the first triangle is 0, 1, 2 and the second is 1, 3, 2, which covers
        // both halves of the window exactly once.
        assert_eq!(QUAD[0].pos, [-1.0, -1.0]);
        assert_eq!(QUAD[1].pos, [1.0, -1.0]);
        assert_eq!(QUAD[2].pos, [-1.0, 1.0]);
        assert_eq!(QUAD[3].pos, [1.0, 1.0]);
        assert_eq!(QUAD.len(), VERTICES as usize);
        // Every corner is on the window's edge: a quad inside it would leave a border of the clear
        // colour, and one past it would be clipped.
        for vertex in QUAD {
            assert_eq!(vertex.pos[0].abs(), 1.0);
            assert_eq!(vertex.pos[1].abs(), 1.0);
        }
    }

    #[test]
    fn the_vertices_are_sixteen_bytes_and_the_layout_says_so() {
        assert_eq!(std::mem::size_of::<PresentVertex>(), 8);
        assert_eq!(bytes().len(), 4 * 8);
        assert_eq!(ATTRIBUTES.len(), 1);
        let attribute = ATTRIBUTES[0];
        assert_eq!(attribute.name, "a_pos");
        assert_eq!(attribute.size, 2);
        assert_eq!(attribute.offset, 0);
        // The stride the device binds: one vertex is its one attribute.
        assert_eq!(attribute.end(), 8);
        assert_eq!(bytes().len(), VERTICES as usize * 8);
    }
}
