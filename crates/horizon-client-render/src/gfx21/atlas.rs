//! PR 5: a GL2-compatible 2D atlas for the block sprites.
//!
//! `terrain.wgsl` binds `texture_2d_array`s, but GL 2.1 has no texture arrays.
//! The legacy path copies layers into a grid of one 2D texture instead. A
//! gutter around every sprite keeps bilinear filtering from bleeding into the
//! neighbouring layer; mip levels stop before the gutter shrinks below one
//! texel. This module is pure Rust: it computes rectangles and UVs, not pixels.

use std::error::Error;
use std::fmt;

/// The X1650's `GL_MAX_TEXTURE_SIZE` is 4096; 2048² leaves room for gutters.
pub const MAX_ATLAS_EDGE: u32 = 2048;
/// Atlas, lightmap, sky and HUD samplers: four texture units, below the
/// eight multitexture units reported by Catalyst 9.3.
pub const MAX_SAMPLERS: u32 = 4;
pub const MAX_LAYERS: u32 = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtlasError {
    EmptyLayers,
    TooManyLayers,
    ZeroSprite,
    ZeroGutter,
    NonPowerOfTwo { value: u32 },
    TooLarge { width: u32, height: u32 },
    LayerOutOfRange { layer: u32, layers: u32 },
    LevelOutOfRange { level: u32, levels: u32 },
    GutterTooSmall,
}

impl fmt::Display for AtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyLayers => write!(f, "atlas needs at least one layer"),
            Self::TooManyLayers => write!(f, "atlas exceeds {MAX_LAYERS} layers"),
            Self::ZeroSprite => write!(f, "sprite edge must be positive"),
            Self::ZeroGutter => write!(f, "gutter must be positive"),
            Self::NonPowerOfTwo { value } => write!(f, "{value} is not a power of two"),
            Self::TooLarge { width, height } => {
                write!(f, "atlas {width}x{height} exceeds {MAX_ATLAS_EDGE}")
            }
            Self::LayerOutOfRange { layer, layers } => {
                write!(f, "layer {layer} is outside 0..{layers}")
            }
            Self::LevelOutOfRange { level, levels } => {
                write!(f, "mip level {level} is outside 0..{levels}")
            }
            Self::GutterTooSmall => write!(f, "gutter does not survive the mip chain"),
        }
    }
}

impl Error for AtlasError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atlas {
    pub layers: u32,
    pub sprite: u32,
    pub gutter: u32,
    pub columns: u32,
    pub rows: u32,
    pub cell: u32,
    pub width: u32,
    pub height: u32,
    pub levels: u32,
}

impl Atlas {
    /// Lay out `layers` square sprites in a near-square grid. `sprite` and
    /// `gutter` are powers of two so every mip level stays aligned.
    pub fn new(layers: u32, sprite: u32, gutter: u32) -> Result<Self, AtlasError> {
        if layers == 0 {
            return Err(AtlasError::EmptyLayers);
        }
        if layers > MAX_LAYERS {
            return Err(AtlasError::TooManyLayers);
        }
        if sprite == 0 {
            return Err(AtlasError::ZeroSprite);
        }
        if gutter == 0 {
            return Err(AtlasError::ZeroGutter);
        }
        if !sprite.is_power_of_two() {
            return Err(AtlasError::NonPowerOfTwo { value: sprite });
        }
        if !gutter.is_power_of_two() {
            return Err(AtlasError::NonPowerOfTwo { value: gutter });
        }
        let columns = f64::from(layers).sqrt().ceil() as u32;
        let rows = layers.div_ceil(columns);
        let cell = sprite + 2 * gutter;
        let width = columns * cell;
        let height = rows * cell;
        if width > MAX_ATLAS_EDGE || height > MAX_ATLAS_EDGE {
            return Err(AtlasError::TooLarge { width, height });
        }
        let mip_sprite = sprite.ilog2() + 1;
        let mip_gutter = gutter.ilog2() + 1;
        let levels = mip_sprite.min(mip_gutter);
        let atlas = Self {
            layers,
            sprite,
            gutter,
            columns,
            rows,
            cell,
            width,
            height,
            levels,
        };
        atlas.validate()?;
        Ok(atlas)
    }

    pub fn validate(&self) -> Result<(), AtlasError> {
        if self.width > MAX_ATLAS_EDGE || self.height > MAX_ATLAS_EDGE {
            return Err(AtlasError::TooLarge {
                width: self.width,
                height: self.height,
            });
        }
        if self.gutter >> (self.levels - 1) < 1 {
            return Err(AtlasError::GutterTooSmall);
        }
        Ok(())
    }

    pub fn sampler_count(&self) -> u32 {
        MAX_SAMPLERS
    }

    /// The inner rectangle of one sprite at mip level zero: `[x0, y0, x1, y1]`.
    pub fn layer_rect(&self, layer: u32) -> Result<[u32; 4], AtlasError> {
        if layer >= self.layers {
            return Err(AtlasError::LayerOutOfRange {
                layer,
                layers: self.layers,
            });
        }
        let column = layer % self.columns;
        let row = layer / self.columns;
        let x0 = column * self.cell + self.gutter;
        let y0 = row * self.cell + self.gutter;
        Ok([x0, y0, x0 + self.sprite, y0 + self.sprite])
    }

    /// Atlas UVs with a half-texel inset on both sides, so `GL_LINEAR` cannot
    /// read the gutter. `[u0, v0, u1, v1]`.
    pub fn uv_rect(&self, layer: u32) -> Result<[f32; 4], AtlasError> {
        let [x0, y0, x1, y1] = self.layer_rect(layer)?;
        let w = self.width as f32;
        let h = self.height as f32;
        Ok([
            (x0 as f32 + 0.5) / w,
            (y0 as f32 + 0.5) / h,
            (x1 as f32 - 0.5) / w,
            (y1 as f32 - 0.5) / h,
        ])
    }

    /// The same rectangle at `level`: the whole atlas shrinks by `2^level`.
    pub fn mip_rect(&self, layer: u32, level: u32) -> Result<[u32; 4], AtlasError> {
        if level >= self.levels {
            return Err(AtlasError::LevelOutOfRange {
                level,
                levels: self.levels,
            });
        }
        let _ = self.layer_rect(layer)?;
        let scale = 1_u32 << level;
        let cell = self.cell / scale;
        let gutter = self.gutter / scale;
        let edge = self.sprite / scale;
        let column = layer % self.columns;
        let row = layer / self.columns;
        let x0 = column * cell + gutter;
        let y0 = row * cell + gutter;
        Ok([x0, y0, x0 + edge, y0 + edge])
    }

    pub fn mip_uv_rect(&self, layer: u32, level: u32) -> Result<[f32; 4], AtlasError> {
        let [x0, y0, x1, y1] = self.mip_rect(layer, level)?;
        let scale = 1_u32 << level;
        let w = (self.width / scale) as f32;
        let h = (self.height / scale) as f32;
        Ok([
            (x0 as f32 + 0.5) / w,
            (y0 as f32 + 0.5) / h,
            (x1 as f32 - 0.5) / w,
            (y1 as f32 - 0.5) / h,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1.0e-6
    }

    #[test]
    fn four_sprites_make_a_two_by_two_grid() {
        let atlas = Atlas::new(4, 16, 4).unwrap();
        assert_eq!(atlas.columns, 2);
        assert_eq!(atlas.rows, 2);
        assert_eq!(atlas.cell, 24);
        assert_eq!(atlas.width, 48);
        assert_eq!(atlas.height, 48);
        assert_eq!(atlas.levels, 3);
        assert_eq!(atlas.sampler_count(), 4);
        assert_eq!(atlas.validate(), Ok(()));
    }

    #[test]
    fn layer_rectangles_have_explicit_gutters() {
        let atlas = Atlas::new(4, 16, 4).unwrap();
        assert_eq!(atlas.layer_rect(0).unwrap(), [4, 4, 20, 20]);
        assert_eq!(atlas.layer_rect(1).unwrap(), [28, 4, 44, 20]);
        assert_eq!(atlas.layer_rect(2).unwrap(), [4, 28, 20, 44]);
        assert_eq!(atlas.layer_rect(3).unwrap(), [28, 28, 44, 44]);
        assert_eq!(
            atlas.layer_rect(4),
            Err(AtlasError::LayerOutOfRange {
                layer: 4,
                layers: 4
            })
        );
    }

    #[test]
    fn uv_rect_has_a_half_texel_inset() {
        let atlas = Atlas::new(4, 16, 4).unwrap();
        let rect = atlas.uv_rect(1).unwrap();
        let want = [28.5 / 48.0, 4.5 / 48.0, 43.5 / 48.0, 19.5 / 48.0];
        assert!(rect.iter().zip(want).all(|(a, b)| close(*a, b)));
    }

    #[test]
    fn mip_levels_stop_while_a_one_texel_gutter_remains() {
        let atlas = Atlas::new(4, 16, 4).unwrap();
        assert_eq!(atlas.mip_rect(0, 0).unwrap(), [4, 4, 20, 20]);
        assert_eq!(atlas.mip_rect(0, 1).unwrap(), [2, 2, 10, 10]);
        assert_eq!(atlas.mip_rect(0, 2).unwrap(), [1, 1, 5, 5]);
        assert_eq!(
            atlas.mip_rect(0, 3),
            Err(AtlasError::LevelOutOfRange {
                level: 3,
                levels: 3
            })
        );
        let rect = atlas.mip_uv_rect(0, 1).unwrap();
        let want = [2.5 / 24.0, 2.5 / 24.0, 9.5 / 24.0, 9.5 / 24.0];
        assert!(rect.iter().zip(want).all(|(a, b)| close(*a, b)));
    }

    #[test]
    fn invalid_layouts_are_rejected() {
        assert_eq!(Atlas::new(0, 16, 4), Err(AtlasError::EmptyLayers));
        assert_eq!(Atlas::new(4, 0, 4), Err(AtlasError::ZeroSprite));
        assert_eq!(Atlas::new(4, 16, 0), Err(AtlasError::ZeroGutter));
        assert_eq!(
            Atlas::new(4, 15, 4),
            Err(AtlasError::NonPowerOfTwo { value: 15 })
        );
        assert_eq!(
            Atlas::new(4, 16, 3),
            Err(AtlasError::NonPowerOfTwo { value: 3 })
        );
        assert_eq!(
            Atlas::new(MAX_LAYERS + 1, 16, 4),
            Err(AtlasError::TooManyLayers)
        );
    }
}
