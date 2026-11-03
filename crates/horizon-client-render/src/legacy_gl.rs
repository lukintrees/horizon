//! CPU-side building blocks for the experimental OpenGL 2 renderer.
//!
//! This module does not create a GL context and is not connected to the game renderer yet.
//! In particular, a single-level atlas is not sufficient for mipmapped terrain or repeated UVs.

/// Prototype shaders; they are not used by the current renderer.
pub const TERRAIN_VERTEX_SHADER: &str = include_str!("legacy_gl_terrain.vert");
pub const TERRAIN_FRAGMENT_SHADER: &str = include_str!("legacy_gl_terrain.frag");

/// A sprite's unpadded dimensions in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpriteSize {
    pub width: u32,
    pub height: u32,
}

/// The sprite's content rectangle (excluding padding) on a square atlas page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub page: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A deterministic layout. Placements are in the same order as the input sprites.
#[derive(Debug, PartialEq, Eq)]
pub struct AtlasLayout {
    page_size: u32,
    padding: u32,
    pages: usize,
    sprites: Vec<Placement>,
}

impl AtlasLayout {
    pub fn page_size(&self) -> u32 {
        self.page_size
    }
    pub fn padding(&self) -> u32 {
        self.padding
    }
    pub fn page_count(&self) -> usize {
        self.pages
    }
    pub fn sprites(&self) -> &[Placement] {
        &self.sprites
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AtlasError {
    InvalidPageSize,
    EmptySprite { index: usize },
    SpriteTooLarge { index: usize },
    TooManyPages,
    InvalidPixels { index: usize },
    SizeOverflow,
}

/// Packs sprites onto fixed-size power-of-two pages using shelves.
///
/// This deliberately trades space efficiency for a simple, deterministic implementation.
/// No rotation is performed. `max_pages` bounds resource use, rather than assuming a
/// particular GPU's texture or memory limits. Padding is reserved on all four sides.
pub fn pack_atlas(
    sprites: &[SpriteSize],
    page_size: u32,
    padding: u32,
    max_pages: usize,
) -> Result<AtlasLayout, AtlasError> {
    if !page_size.is_power_of_two() {
        return Err(AtlasError::InvalidPageSize);
    }
    let border = padding.checked_mul(2).ok_or(AtlasError::SizeOverflow)?;
    let mut placements = Vec::with_capacity(sprites.len());
    let (mut pages, mut x, mut y, mut shelf_height) = (0usize, 0u32, 0u32, 0u32);

    for (index, sprite) in sprites.iter().enumerate() {
        if sprite.width == 0 || sprite.height == 0 {
            return Err(AtlasError::EmptySprite { index });
        }
        let width = sprite
            .width
            .checked_add(border)
            .ok_or(AtlasError::SizeOverflow)?;
        let height = sprite
            .height
            .checked_add(border)
            .ok_or(AtlasError::SizeOverflow)?;
        if width > page_size || height > page_size {
            return Err(AtlasError::SpriteTooLarge { index });
        }
        if pages == 0 {
            if max_pages == 0 {
                return Err(AtlasError::TooManyPages);
            }
            pages = 1;
        }
        if width > page_size - x {
            x = 0;
            y += shelf_height;
            shelf_height = 0;
        }
        if height > page_size - y {
            if pages >= max_pages {
                return Err(AtlasError::TooManyPages);
            }
            pages += 1;
            x = 0;
            y = 0;
            shelf_height = 0;
        }
        placements.push(Placement {
            page: pages - 1,
            x: x + padding,
            y: y + padding,
            width: sprite.width,
            height: sprite.height,
        });
        x += width;
        shelf_height = shelf_height.max(height);
    }
    Ok(AtlasLayout {
        page_size,
        padding,
        pages,
        sprites: placements,
    })
}

/// RGBA8 source pixels in row-major order, one slice for each input sprite.
/// Produces one page at a time and replicates edge pixels into its padding.
/// This builds only level zero: callers must not enable mipmaps based on it.
pub fn build_page(
    layout: &AtlasLayout,
    page: usize,
    pixels: &[&[u8]],
) -> Result<Vec<u8>, AtlasError> {
    if page >= layout.pages || pixels.len() != layout.sprites.len() {
        return Err(AtlasError::SizeOverflow);
    }
    let side = usize::try_from(layout.page_size).map_err(|_| AtlasError::SizeOverflow)?;
    let bytes = side
        .checked_mul(side)
        .and_then(|n| n.checked_mul(4))
        .ok_or(AtlasError::SizeOverflow)?;
    let mut output = vec![0; bytes];
    for (index, (place, source)) in layout.sprites.iter().zip(pixels).enumerate() {
        let w = usize::try_from(place.width).map_err(|_| AtlasError::SizeOverflow)?;
        let h = usize::try_from(place.height).map_err(|_| AtlasError::SizeOverflow)?;
        let expected = w
            .checked_mul(h)
            .and_then(|n| n.checked_mul(4))
            .ok_or(AtlasError::SizeOverflow)?;
        if source.len() != expected {
            return Err(AtlasError::InvalidPixels { index });
        }
        if place.page != page {
            continue;
        }
        let pad = usize::try_from(layout.padding).map_err(|_| AtlasError::SizeOverflow)?;
        let left = usize::try_from(place.x).map_err(|_| AtlasError::SizeOverflow)? - pad;
        let top = usize::try_from(place.y).map_err(|_| AtlasError::SizeOverflow)? - pad;
        for dy in 0..h + pad * 2 {
            for dx in 0..w + pad * 2 {
                let sx = dx.saturating_sub(pad).min(w - 1);
                let sy = dy.saturating_sub(pad).min(h - 1);
                let from = (sy * w + sx) * 4;
                let to = ((top + dy) * side + left + dx) * 4;
                output[to..to + 4].copy_from_slice(&source[from..from + 4]);
            }
        }
    }
    Ok(output)
}

/// Vertex format for the experimental GLSL 1.10 terrain shader. The color alpha
/// in `TerrainVertex` encodes emission, not transparency, so it is not used here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegacyTerrainVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub light_uv: [f32; 2],
    pub color: [f32; 4],
}

#[derive(Debug, PartialEq, Eq)]
pub enum TerrainConversionError {
    MissingSprite { layer: u16 },
    MixedSpriteLayers,
    RepeatingUv,
}

/// Converts one section quad, returning its atlas page and four vertices.
///
/// This initial path explicitly rejects UVs outside [0, 1]: repeating within a
/// sprite cannot be emulated by the GL texture wrap mode on an atlas page.
/// It does not handle tiled distant terrain, packed meshes or emissive lighting.
pub fn convert_terrain_quad(
    layout: &AtlasLayout,
    quad: &[crate::mesher::TerrainVertex; 4],
) -> Result<(usize, [LegacyTerrainVertex; 4]), TerrainConversionError> {
    use crate::mesher::OPAQUE_LAYER;
    let layer = quad[0].layer & !OPAQUE_LAYER;
    let sprite = layout
        .sprites
        .get(usize::from(layer))
        .ok_or(TerrainConversionError::MissingSprite { layer })?;
    if quad.iter().any(|v| v.layer & !OPAQUE_LAYER != layer) {
        return Err(TerrainConversionError::MixedSpriteLayers);
    }
    if quad.iter().any(|v| v.uv().iter().any(|&uv| uv > 1.0)) {
        return Err(TerrainConversionError::RepeatingUv);
    }
    let side = layout.page_size as f32;
    let vertices = quad.map(|v| {
        let uv = v.uv();
        let [r, g, b, _] = v.color.to_le_bytes();
        let sky = f32::from(v.light & 255);
        let block = f32::from(v.light >> 8);
        let light_coord = |value: f32| (value / 256.0 + 0.5 / 16.0).clamp(0.5 / 16.0, 15.5 / 16.0);
        LegacyTerrainVertex {
            pos: v.pos,
            uv: [
                (sprite.x as f32 + uv[0] * sprite.width as f32) / side,
                (sprite.y as f32 + uv[1] * sprite.height as f32) / side,
            ],
            light_uv: [light_coord(block), light_coord(sky)],
            color: [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                1.0,
            ],
        }
    });
    Ok((sprite.page, vertices))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_reserves_padding_and_starts_new_pages() {
        let sizes = [SpriteSize {
            width: 2,
            height: 2,
        }; 5];
        let layout = pack_atlas(&sizes, 8, 1, 2).unwrap();
        assert_eq!(layout.pages, 2);
        assert_eq!(
            layout
                .sprites
                .iter()
                .map(|p| (p.page, p.x, p.y))
                .collect::<Vec<_>>(),
            vec![(0, 1, 1), (0, 5, 1), (0, 1, 5), (0, 5, 5), (1, 1, 1)]
        );
        assert_eq!(pack_atlas(&sizes, 8, 1, 1), Err(AtlasError::TooManyPages));
    }

    #[test]
    fn page_replicates_corners_and_edges_without_touching_other_sprites() {
        let sizes = [
            SpriteSize {
                width: 2,
                height: 2,
            },
            SpriteSize {
                width: 1,
                height: 1,
            },
        ];
        let layout = pack_atlas(&sizes, 8, 1, 1).unwrap();
        let a = [1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255];
        let b = [9, 0, 0, 255];
        let page = build_page(&layout, 0, &[&a, &b]).unwrap();
        let red = |x: usize, y: usize| page[(y * 8 + x) * 4];
        assert_eq!((red(0, 0), red(3, 0), red(0, 3), red(3, 3)), (1, 2, 3, 4));
        assert_eq!((red(4, 0), red(6, 2), red(7, 0)), (9, 9, 0));
        assert_eq!(
            build_page(&layout, 0, &[&a, &[]]),
            Err(AtlasError::InvalidPixels { index: 1 })
        );
    }

    #[test]
    fn rejects_invalid_inputs_without_allocating_pages() {
        assert_eq!(pack_atlas(&[], 3, 0, 1), Err(AtlasError::InvalidPageSize));
        assert_eq!(
            pack_atlas(
                &[SpriteSize {
                    width: 0,
                    height: 1
                }],
                8,
                0,
                1
            ),
            Err(AtlasError::EmptySprite { index: 0 })
        );
        assert_eq!(
            pack_atlas(
                &[SpriteSize {
                    width: 8,
                    height: 8
                }],
                8,
                1,
                1
            ),
            Err(AtlasError::SpriteTooLarge { index: 0 })
        );
        assert_eq!(
            pack_atlas(
                &[SpriteSize {
                    width: 1,
                    height: 1
                }],
                8,
                u32::MAX,
                1
            ),
            Err(AtlasError::SizeOverflow)
        );
        assert_eq!(pack_atlas(&[], 8, 0, 0).unwrap().pages, 0);
    }

    #[test]
    fn converts_quad_uv_color_and_light_and_rejects_repetition() {
        use crate::mesher::{OPAQUE_LAYER, TerrainVertex};
        let layout = pack_atlas(
            &[SpriteSize {
                width: 2,
                height: 2,
            }],
            8,
            1,
            1,
        )
        .unwrap();
        let v = TerrainVertex::new(
            [1.0, 2.0, 3.0],
            [0.5, 1.0],
            OPAQUE_LAYER,
            0x80402010,
            0xf080,
        );
        let (page, vertices) = convert_terrain_quad(&layout, &[v; 4]).unwrap();
        assert_eq!(page, 0);
        assert_eq!(vertices[0].pos, [1.0, 2.0, 3.0]);
        assert_eq!(vertices[0].uv, [0.25, 0.375]);
        assert_eq!(vertices[0].light_uv, [0.96875, 0.53125]);
        assert_eq!(
            vertices[0].color,
            [16.0 / 255.0, 32.0 / 255.0, 64.0 / 255.0, 1.0]
        );
        let repeat = TerrainVertex::new([0.0; 3], [1.5, 0.0], 0, 0, 0);
        assert_eq!(
            convert_terrain_quad(&layout, &[repeat; 4]),
            Err(TerrainConversionError::RepeatingUv)
        );
        let other = TerrainVertex::new([0.0; 3], [0.0; 2], 1, 0, 0);
        assert_eq!(
            convert_terrain_quad(&layout, &[v, v, v, other]),
            Err(TerrainConversionError::MixedSpriteLayers)
        );
        assert_eq!(
            convert_terrain_quad(&layout, &[other; 4]),
            Err(TerrainConversionError::MissingSprite { layer: 1 })
        );
    }
}
