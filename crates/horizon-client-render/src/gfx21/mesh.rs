//! PR 5: convert `mesher::TerrainVertex` into a GL2-friendly vertex.
//!
//! `TerrainVertex` is defined in `mesher.rs`; this mirror keeps the additive
//! `gfx21` module compilable without the rest of the render crate. The CPU
//! expands the packed u16 UV, u16 light and RGBA8 tint into plain floats/bytes,
//! because GLSL 110/120 has no integer attributes or bit operations. The layer
//! is resolved to 2D atlas UVs here, so the shader only needs `texture2D`.

use std::error::Error;
use std::fmt;

use super::atlas::{Atlas, AtlasError};

/// Fixed-point steps per unit of section UVs, matching `mesher.rs`.
pub const UV_STEPS: f32 = 32768.0;
/// Fixed-point steps per block of distant-terrain UVs (`TerrainVertex::tiled`).
pub const TILED_UV_STEPS: f32 = 32.0;
/// `mesher::OPAQUE_LAYER`: the high bit marks opaque leaves/quads.
pub const OPAQUE_LAYER: u16 = 1 << 15;
/// 3 floats + 2 floats + RGBA8 + lightmap UV + layer + 2 bytes of padding.
pub const GL_VERTEX_BYTES: usize = 36;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TerrainVertex {
    pub pos: [f32; 3],
    pub uv: [u16; 2],
    pub layer: u16,
    pub light: u16,
    pub color: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub color: [u8; 4],
    pub light_uv: [f32; 2],
    pub layer: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MeshError {
    BadUvStep { got: f32 },
    LayerOutOfRange { layer: u16, layers: u32 },
    Atlas(AtlasError),
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadUvStep { got } => write!(f, "unknown UV step {got}"),
            Self::LayerOutOfRange { layer, layers } => {
                write!(f, "layer {layer} is outside 0..{layers}")
            }
            Self::Atlas(err) => write!(f, "atlas: {err}"),
        }
    }
}

impl Error for MeshError {}

impl From<AtlasError> for MeshError {
    fn from(err: AtlasError) -> Self {
        Self::Atlas(err)
    }
}

/// `color` is the little-endian RGBA8 value the mesher writes: red in bits
/// 0..7, green in 8..15, blue in 16..23 and alpha in 24..31.
pub fn pack_color(color: u32) -> [u8; 4] {
    [
        (color & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        ((color >> 16) & 0xFF) as u8,
        ((color >> 24) & 0xFF) as u8,
    ]
}

/// `terrain.wgsl` samples the 16x16 lightmap at `(block + 8) / 256` and
/// `(sky + 8) / 256`, clamped to the half-texel border. Sky is the low byte,
/// block light the high byte of `TerrainVertex::light`.
pub fn lightmap_uv(light: u16) -> [f32; 2] {
    let sky = u32::from(light & 0xFF);
    let block = u32::from(light >> 8);
    let u = (block + 8).clamp(8, 248) as f32 / 256.0;
    let v = (sky + 8).clamp(8, 248) as f32 / 256.0;
    [u, v]
}

pub fn wrap01(value: f32) -> f32 {
    value - value.floor()
}

impl GlVertex {
    /// Expand one terrain vertex. `uv_step` is [`UV_STEPS`] for sections or
    /// [`TILED_UV_STEPS`] for distant tiles; tiled UVs wrap inside one sprite.
    pub fn from_terrain(
        vertex: &TerrainVertex,
        atlas: &Atlas,
        uv_step: f32,
    ) -> Result<Self, MeshError> {
        if uv_step != UV_STEPS && uv_step != TILED_UV_STEPS {
            return Err(MeshError::BadUvStep { got: uv_step });
        }
        let layer = vertex.layer & !OPAQUE_LAYER;
        if u32::from(layer) >= atlas.layers {
            return Err(MeshError::LayerOutOfRange {
                layer,
                layers: atlas.layers,
            });
        }
        let rect = atlas.uv_rect(u32::from(layer))?;
        let su = wrap01(f32::from(vertex.uv[0]) / uv_step);
        let sv = wrap01(f32::from(vertex.uv[1]) / uv_step);
        Ok(Self {
            pos: vertex.pos,
            uv: [
                rect[0] + su * (rect[2] - rect[0]),
                rect[1] + sv * (rect[3] - rect[1]),
            ],
            color: pack_color(vertex.color),
            light_uv: lightmap_uv(vertex.light),
            layer,
        })
    }

    /// Explicit little-endian layout for a `GL_UNSIGNED_SHORT` index buffer.
    /// Two trailing bytes pad the record to a four-byte boundary.
    pub fn to_bytes(&self) -> [u8; GL_VERTEX_BYTES] {
        let mut out = [0_u8; GL_VERTEX_BYTES];
        let mut at = 0;
        for value in self.pos {
            out[at..at + 4].copy_from_slice(&value.to_le_bytes());
            at += 4;
        }
        for value in self.uv {
            out[at..at + 4].copy_from_slice(&value.to_le_bytes());
            at += 4;
        }
        out[at..at + 4].copy_from_slice(&self.color);
        at += 4;
        for value in self.light_uv {
            out[at..at + 4].copy_from_slice(&value.to_le_bytes());
            at += 4;
        }
        out[at..at + 2].copy_from_slice(&self.layer.to_le_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_and_light_pack_with_independent_values() {
        assert_eq!(pack_color(0x80_44_33_22), [0x22, 0x33, 0x44, 0x80]);
        assert_eq!(lightmap_uv(0x0A_14), [18.0 / 256.0, 28.0 / 256.0]);
        assert_eq!(lightmap_uv(0xFF_FF), [248.0 / 256.0, 248.0 / 256.0]);
        assert_eq!(wrap01(2.5), 0.5);
    }

    #[test]
    fn section_vertex_expands_to_explicit_gl_bytes() {
        let atlas = Atlas::new(4, 16, 4).unwrap();
        let vertex = TerrainVertex {
            pos: [1.0, 2.0, 3.0],
            uv: [16384, 32768],
            layer: 1,
            light: 0x0A_14,
            color: 0x80_44_33_22,
        };
        let gl = GlVertex::from_terrain(&vertex, &atlas, UV_STEPS).unwrap();
        assert_eq!(gl.pos, [1.0, 2.0, 3.0]);
        assert_eq!(gl.uv, [0.75, 0.09375]);
        assert_eq!(gl.color, [0x22, 0x33, 0x44, 0x80]);
        assert_eq!(gl.light_uv, [18.0 / 256.0, 28.0 / 256.0]);
        assert_eq!(gl.layer, 1);
        const EXPECTED: [u8; GL_VERTEX_BYTES] = [
            0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x40, 0x40, 0x00, 0x00,
            0x40, 0x3f, 0x00, 0x00, 0xc0, 0x3d, 0x22, 0x33, 0x44, 0x80, 0x00, 0x00, 0x90, 0x3d,
            0x00, 0x00, 0xe0, 0x3d, 0x01, 0x00, 0x00, 0x00,
        ];
        assert_eq!(gl.to_bytes(), EXPECTED);
        assert_eq!(gl.to_bytes().len(), GL_VERTEX_BYTES);
    }

    #[test]
    fn tiled_uv_step_wraps_and_uses_the_layer_rect() {
        let atlas = Atlas::new(4, 16, 4).unwrap();
        let vertex = TerrainVertex {
            pos: [0.0; 3],
            uv: [64, 32],
            layer: 2,
            light: 0,
            color: 0,
        };
        let gl = GlVertex::from_terrain(&vertex, &atlas, TILED_UV_STEPS).unwrap();
        assert_eq!(gl.uv, [0.09375, 0.59375]);
        assert_eq!(gl.layer, 2);
    }

    #[test]
    fn opaque_layer_flag_is_masked_before_atlas_lookup() {
        let atlas = Atlas::new(4, 16, 4).unwrap();
        let vertex = TerrainVertex {
            pos: [0.0; 3],
            uv: [0, 0],
            layer: 1 | OPAQUE_LAYER,
            light: 0,
            color: 0,
        };
        let gl = GlVertex::from_terrain(&vertex, &atlas, UV_STEPS).unwrap();
        assert_eq!(gl.layer, 1);
        assert_eq!(gl.uv, [28.5 / 48.0, 4.5 / 48.0]);
    }

    #[test]
    fn bad_uv_step_and_out_of_range_layer_are_rejected() {
        let atlas = Atlas::new(4, 16, 4).unwrap();
        let vertex = TerrainVertex::default();
        assert!(matches!(
            GlVertex::from_terrain(&vertex, &atlas, 64.0),
            Err(MeshError::BadUvStep { got }) if got == 64.0
        ));
        let out = TerrainVertex {
            layer: 9,
            ..TerrainVertex::default()
        };
        assert!(matches!(
            GlVertex::from_terrain(&out, &atlas, UV_STEPS),
            Err(MeshError::LayerOutOfRange {
                layer: 9,
                layers: 4
            })
        ));
    }
}
