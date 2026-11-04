//! Paged, padded RGBA sprite atlases for hardware without texture arrays.
//!
//! Every sprite is a square layer of equal size, as in `sprites::Sprites`.
//! Each page has power-of-two dimensions for OpenGL 2.1 implementations
//! without unrestricted NPOT support. Padding duplicates edge texels; mipmaps
//! are deliberately not generated, since their gutters would need to grow per
//! level. Rendering must use non-mipmapped filtering until that is implemented.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AtlasError {
    EmptySprite,
    TooSmall,
    SizeOverflow,
    WrongPixelCount { expected: usize, actual: usize },
    LayerOutOfRange,
    AllocationFailed,
}

impl fmt::Display for AtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySprite => write!(f, "sprite side must be nonzero"),
            Self::TooSmall => write!(f, "maximum texture size cannot fit a padded sprite"),
            Self::SizeOverflow => write!(f, "atlas dimensions overflow"),
            Self::WrongPixelCount { expected, actual } => {
                write!(f, "expected {expected} RGBA bytes, got {actual}")
            }
            Self::LayerOutOfRange => write!(f, "sprite layer is outside the atlas"),
            Self::AllocationFailed => write!(f, "could not allocate atlas page"),
        }
    }
}

impl std::error::Error for AtlasError {}

/// The rectangle to pass to the GLSL shader as `(origin.xy, scale.xy)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Location {
    pub page: usize,
    pub rect: [f32; 4],
}

/// One texture page in tightly packed RGBA8, rows in the same order as input.
#[derive(Debug)]
pub struct Page {
    pub side: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug)]
pub struct Atlas {
    sprite_side: u32,
    page_side: u32,
    columns: u32,
    per_page: u32,
    layer_count: u32,
    pages: Vec<Page>,
}

impl Atlas {
    /// `pixels` contains `layer_count` consecutive `sprite_side` square RGBA8
    /// images. `max_texture_size` is the actual queried GPU limit, optionally
    /// capped by a smaller memory budget chosen by the caller.
    pub fn new(
        sprite_side: u32,
        layer_count: u32,
        max_texture_size: u32,
        pixels: &[u8],
    ) -> Result<Self, AtlasError> {
        if sprite_side == 0 {
            return Err(AtlasError::EmptySprite);
        }
        let cell = sprite_side.checked_add(2).ok_or(AtlasError::SizeOverflow)?;
        let limit = if max_texture_size == 0 {
            0
        } else {
            1 << (31 - max_texture_size.leading_zeros())
        };
        let mut page_side = cell
            .checked_next_power_of_two()
            .ok_or(AtlasError::SizeOverflow)?;
        if page_side > limit {
            return Err(AtlasError::TooSmall);
        }
        // A tiny atlas should not allocate a full max-sized GPU texture.
        while page_side < limit {
            let cols = u64::from(page_side / cell);
            if cols * cols >= u64::from(layer_count) {
                break;
            }
            page_side *= 2;
        }
        let columns = page_side / cell;
        let per_page = columns
            .checked_mul(columns)
            .ok_or(AtlasError::SizeOverflow)?;
        let sprite_bytes = usize::try_from(sprite_side)
            .ok()
            .and_then(|s| s.checked_mul(s))
            .and_then(|n| n.checked_mul(4))
            .ok_or(AtlasError::SizeOverflow)?;
        let expected = sprite_bytes
            .checked_mul(layer_count as usize)
            .ok_or(AtlasError::SizeOverflow)?;
        if pixels.len() != expected {
            return Err(AtlasError::WrongPixelCount {
                expected,
                actual: pixels.len(),
            });
        }
        let page_bytes = usize::try_from(page_side)
            .ok()
            .and_then(|s| s.checked_mul(s))
            .and_then(|n| n.checked_mul(4))
            .ok_or(AtlasError::SizeOverflow)?;
        let count = layer_count / per_page + u32::from(layer_count % per_page != 0);
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(count as usize)
            .map_err(|_| AtlasError::AllocationFailed)?;
        for _ in 0..count {
            let mut data = Vec::new();
            data.try_reserve_exact(page_bytes)
                .map_err(|_| AtlasError::AllocationFailed)?;
            data.resize(page_bytes, 0);
            pages.push(Page {
                side: page_side,
                pixels: data,
            });
        }
        let mut atlas = Self {
            sprite_side,
            page_side,
            columns,
            per_page,
            layer_count,
            pages,
        };
        for layer in 0..layer_count {
            let start = layer as usize * sprite_bytes;
            atlas.update(layer, &pixels[start..start + sprite_bytes])?;
        }
        Ok(atlas)
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    pub fn location(&self, layer: u32) -> Result<Location, AtlasError> {
        if layer >= self.layer_count {
            return Err(AtlasError::LayerOutOfRange);
        }
        let cell = self.sprite_side + 2;
        let slot = layer % self.per_page;
        let x = (slot % self.columns) * cell + 1;
        let y = (slot / self.columns) * cell + 1;
        let side = self.page_side as f32;
        let scale = self.sprite_side as f32 / side;
        Ok(Location {
            page: (layer / self.per_page) as usize,
            rect: [x as f32 / side, y as f32 / side, scale, scale],
        })
    }

    /// Replace an animated frame and all of its one-pixel gutters.
    pub fn update(&mut self, layer: u32, pixels: &[u8]) -> Result<(), AtlasError> {
        if layer >= self.layer_count {
            return Err(AtlasError::LayerOutOfRange);
        }
        let s = self.sprite_side as usize;
        let expected = s
            .checked_mul(s)
            .and_then(|n| n.checked_mul(4))
            .ok_or(AtlasError::SizeOverflow)?;
        if pixels.len() != expected {
            return Err(AtlasError::WrongPixelCount {
                expected,
                actual: pixels.len(),
            });
        }
        let slot = layer % self.per_page;
        let cell = self.sprite_side + 2;
        let x = ((slot % self.columns) * cell) as usize;
        let y = ((slot / self.columns) * cell) as usize;
        let page = &mut self.pages[(layer / self.per_page) as usize].pixels;
        let page_side = self.page_side as usize;
        for dy in 0..s + 2 {
            let sy = dy.saturating_sub(1).min(s - 1);
            for dx in 0..s + 2 {
                let sx = dx.saturating_sub(1).min(s - 1);
                let src = (sy * s + sx) * 4;
                let dst = ((y + dy) * page_side + x + dx) * 4;
                page[dst..dst + 4].copy_from_slice(&pixels[src..src + 4]);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_pixels_and_animation_update() {
        let pixels = [1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255];
        let mut atlas = Atlas::new(2, 1, 8, &pixels).unwrap();
        assert_eq!(atlas.pages()[0].side, 4);
        let at = |a: &Atlas, x: usize, y: usize| a.pages()[0].pixels[(y * 4 + x) * 4];
        assert_eq!(
            (0..4).map(|x| at(&atlas, x, 0)).collect::<Vec<_>>(),
            [1, 1, 2, 2]
        );
        assert_eq!(
            (0..4).map(|x| at(&atlas, x, 3)).collect::<Vec<_>>(),
            [3, 3, 4, 4]
        );
        assert_eq!(atlas.location(0).unwrap().rect, [0.25, 0.25, 0.5, 0.5]);
        atlas.update(0, &[9; 16]).unwrap();
        assert_eq!(at(&atlas, 0, 0), 9);
        assert_eq!(at(&atlas, 3, 3), 9);
    }

    #[test]
    fn updates_do_not_touch_neighbouring_sprites() {
        let pixels: Vec<u8> = (1..=4)
            .flat_map(|value| [value, 0, 0, 255].repeat(4))
            .collect();
        let mut atlas = Atlas::new(2, 4, 8, &pixels).unwrap();
        assert_eq!(atlas.pages().len(), 1);
        assert_eq!(atlas.location(3).unwrap().rect, [0.625, 0.625, 0.25, 0.25]);
        atlas.update(0, &[9; 16]).unwrap();
        let page = &atlas.pages()[0].pixels;
        let pixel = |x: usize, y: usize| page[(y * 8 + x) * 4];
        assert_eq!(
            [pixel(3, 3), pixel(4, 3), pixel(3, 4), pixel(4, 4)],
            [9, 2, 3, 4]
        );
    }

    #[test]
    fn multiple_pages_preserve_layer_order_and_limit() {
        let data: Vec<u8> = (0..5).flat_map(|i| [i, i, i, 255]).collect();
        let atlas = Atlas::new(1, 5, 7, &data).unwrap(); // page side 4, one cell
        assert_eq!(atlas.pages().len(), 5);
        for i in 0..5 {
            assert_eq!(atlas.location(i).unwrap().page, i as usize);
            assert_eq!(atlas.pages()[i as usize].pixels[0], i as u8);
        }
    }

    #[test]
    fn rejects_invalid_dimensions_and_data_without_panicking() {
        assert_eq!(
            Atlas::new(0, 0, 8, &[]).unwrap_err(),
            AtlasError::EmptySprite
        );
        assert_eq!(
            Atlas::new(u32::MAX, 0, 8, &[]).unwrap_err(),
            AtlasError::SizeOverflow
        );
        assert_eq!(Atlas::new(4, 0, 5, &[]).unwrap_err(), AtlasError::TooSmall);
        assert_eq!(
            Atlas::new(2, 1, 8, &[]).unwrap_err(),
            AtlasError::WrongPixelCount {
                expected: 16,
                actual: 0
            }
        );
        let mut atlas = Atlas::new(1, 1, 4, &[1, 2, 3, 4]).unwrap();
        let before = atlas.pages()[0].pixels.clone();
        assert!(atlas.update(0, &[]).is_err());
        assert!(atlas.update(1, &[1, 2, 3, 4]).is_err());
        assert_eq!(atlas.pages()[0].pixels, before);
    }
}
