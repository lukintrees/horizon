//! A section's vertices in the layout OpenGL 2.1 and GLSL 1.20 can take.
//!
//! `mesher::TerrainVertex` is 24 bytes: `u16` UVs at 1/32768 of a sprite (`UV_STEPS`), the sprite's
//! array layer, the light as two `u16`s packed in one integer, and an RGBA8 colour. GL 2.1 has no
//! integer vertex attributes (`glVertexAttribIPointer` is GL 3.0), and its shaders have no bit
//! arithmetic, so none of that can be done in the shader: each field becomes an attribute of a type a
//! shader reads directly.
//!
//! | attribute | type | `TerrainVertex` field |
//! | --- | --- | --- |
//! | 0 `a_pos` | `GL_FLOAT` ×3 | `pos`, section-relative, as it is |
//! | 1 `a_uv` | `GL_FLOAT` ×2 | `uv / UV_STEPS`: the sprite-local `[0, 2)` UV, plain floats |
//! | 2 `a_light` | `GL_UNSIGNED_SHORT` ×2, normalized | `light`'s sky byte and block byte, as `1/255` |
//! | 3 `a_color` | `GL_UNSIGNED_BYTE` ×4, normalized | `color`'s rgb, and in the alpha 1 for a force-opaque sprite |
//!
//! Four attributes of the 16 R500 has, 32 bytes a vertex instead of 24. The sprite **layer is not
//! here**: GL 2.1 has no integer attributes to pass it and no arrays to index, so it picks the draw.
//! A section's vertices are split into one list per sprite (`batches`), and each list is drawn with
//! its sprite's atlas rectangle bound to `u_sprite`, as vanilla's own chunk renderer did before
//! texture arrays. That is what a flat atlas costs instead of the layer attribute.

use crate::mesher::TerrainVertex;

/// The bits of a `TerrainVertex`'s layer that are a sprite: the top bit marks opaque leaves
/// (`mesher::OPAQUE_LAYER`).
pub const LAYER_MASK: u16 = 0x7fff;

/// The stride between vertices, and the order of `terrain.vert`'s `attribute`s.
pub const STRIDE: u32 = 28;

/// One vertex, 28 bytes.
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct GlVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    /// Sky light in x, block light in y: the mesh's `sky * 16` and `block * 16` bytes, times 257 so
    /// that the shader's normalized read is exactly `level * 16 / 255`.
    pub light: [u16; 2],
    /// RGBA8, red first: the tint times the shading, and in the alpha 255 when the vertex's layer
    /// is force-opaque, else 0 (`terrain.frag`'s alpha test reads it as a flag, not as coverage).
    pub color: [u8; 4],
}

impl GlVertex {
    /// `vertex` for GL 2.1: its own fields, nothing dropped.
    pub fn of(vertex: &TerrainVertex) -> GlVertex {
        let color = vertex.color;
        GlVertex {
            pos: vertex.pos,
            uv: vertex.uv(),
            light: [
                // Smooth light is 16x fixed point: sky in the low byte, block in the high one, and
                // `n * 257` of 65535 is `n` of 255 for the shader.
                (vertex.light & 0xff) * 257,
                (vertex.light >> 8) * 257,
            ],
            color: [
                (color & 0xff) as u8,
                ((color >> 8) & 0xff) as u8,
                ((color >> 16) & 0xff) as u8,
                if (vertex.layer & crate::mesher::OPAQUE_LAYER) != 0 {
                    0xff
                } else {
                    0
                },
            ],
        }
    }

    /// The sprite this vertex draws: its layer without the opaque-leaves mark.
    pub fn sprite(vertex: &TerrainVertex) -> u16 {
        vertex.layer & LAYER_MASK
    }
}

/// A section's vertices as GL 2.1 draws them: one run per sprite, in the order of `SectionMesh`'s
/// own vertex list (so quads stay quads: four vertices of one sprite are four in a row).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Batches {
    /// (sprite, first vertex, count) in one vertex buffer of `Batches::vertices`'s vertices.
    pub runs: Vec<(u16, u32, u32)>,
}

impl Batches {
    /// `vertices` split by sprite, their GL layout, and the runs that draw them.
    pub fn of(vertices: &[TerrainVertex]) -> (Vec<GlVertex>, Batches) {
        let mut runs: Vec<(u16, u32, u32)> = Vec::new();
        let mut out = Vec::with_capacity(vertices.len());
        for vertex in vertices {
            let sprite = GlVertex::sprite(vertex);
            match runs.last_mut() {
                Some((last, first, count)) if *last == sprite => {
                    *count += 1;
                }
                _ => runs.push((sprite, out.len() as u32, 1)),
            }
            out.push(GlVertex::of(vertex));
        }
        (out, Batches { runs })
    }

    /// How many draw calls the runs are.
    pub fn draws(&self) -> usize {
        self.runs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vertex at `x` (y 2, z 3) — `TerrainVertex::new` rounds the UV to `1/32768`.
    fn vertex(x: f32, uv: [f32; 2], layer: u16, light: u16, color: u32) -> TerrainVertex {
        TerrainVertex::new([x, 2.0, 3.0], uv, layer, color, light)
    }

    #[test]
    fn a_vertex_carries_its_fields_as_floats_and_normalized_ints() {
        // Sky 15, block 4 (16 levels are 16x fixed point: the bytes are 240 and 64) in the mesh's
        // packing: `sky * 16` in the low byte, `block * 16` in the high one
        // (`mesher::pack_coords`).
        let packed_light = (4 * 16) << 8 | 15 * 16;
        // A colour of #00804020: red 0x20 in the low byte, blue 0x80 in the third.
        let v = GlVertex::of(&vertex(1.5, [0.25, 0.75], 7, packed_light, 0x0080_4020));
        assert_eq!(v.pos, [1.5, 2.0, 3.0]);
        // The UV is the sprite-local one, not the fixed point the mesh holds.
        assert_eq!(v.uv, [0.25, 0.75]);
        // 240 of 255 is what the shader reads as `240 / 255`, and 240 * 257 of 65535 is the same.
        assert_eq!(v.light, [240 * 257, 64 * 257]);
        assert_eq!(v.light[0], 61680, "240 * 257 fits a u16");
        assert!((f32::from(v.light[0]) / 65535.0 - 240.0 / 255.0).abs() < 1e-6);
        // Not a force-opaque sprite: the alpha is the flag, 0.
        assert_eq!(v.color, [0x20, 0x40, 0x80, 0]);

        // The same vertex on a layer marked force-opaque.
        let opaque = GlVertex::of(&vertex(0.0, [0.0, 0.0], 7 | crate::mesher::OPAQUE_LAYER, 0, 0));
        assert_eq!(opaque.color[3], 0xff);
        // And the HDR emission the wgpu path keeps in that byte is not carried: 0xFF as an ordinary
        // opaque colour still says "alpha test me".
        let lit = GlVertex::of(&vertex(0.0, [0.0, 0.0], 7, 0, 0xFF00_0000));
        assert_eq!(lit.color[3], 0);
    }

    #[test]
    fn the_stride_is_what_the_attribute_pointers_step() {
        // No padding: bytemuck checks that the layout has none, and a GL stride must be exact.
        assert_eq!(std::mem::size_of::<GlVertex>() as u32, STRIDE);
        // 3 + 2 floats, 2 shorts, 4 bytes.
        assert_eq!(STRIDE, 3 * 4 + 2 * 4 + 2 * 2 + 4);
        // Where each attribute starts, for `glVertexAttribPointer`.
        assert_eq!(std::mem::offset_of!(GlVertex, uv), 12);
        assert_eq!(std::mem::offset_of!(GlVertex, light), 20);
        assert_eq!(std::mem::offset_of!(GlVertex, color), 24);
    }

    #[test]
    fn runs_split_by_sprite_and_keep_the_order() {
        let vertices = [
            vertex(0.0, [0.0, 0.0], 3, 0, 0xFF00_0000),
            vertex(1.0, [1.0, 0.0], 3, 0, 0xFF00_0000),
            vertex(1.0, [1.0, 1.0], 5, 0, 0xFF00_0000),
            vertex(0.0, [0.0, 1.0], 3, 0, 0xFF00_0000),
            // The opaque-leaves mark is not a sprite of its own.
            vertex(0.0, [0.0, 1.0], 3 | crate::mesher::OPAQUE_LAYER, 0, 0xFF00_0000),
        ];
        let (out, batches) = Batches::of(&vertices);
        assert_eq!(out.len(), 5);
        assert_eq!(
            batches.runs,
            vec![(3, 0, 2), (5, 2, 1), (3, 3, 2)],
            "a sprite that comes back is a run of its own, as the mesh's order is the quad order"
        );
        assert_eq!(batches.draws(), 3);
    }

    #[test]
    fn an_empty_mesh_draws_nothing() {
        let (out, batches) = Batches::of(&[]);
        assert!(out.is_empty());
        assert_eq!(batches.draws(), 0);
    }
}
