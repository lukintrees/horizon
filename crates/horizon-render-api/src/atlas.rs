//! CPU atlas primitives reused from horizon draft PR #1; single mip level only.
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
    InvalidSpriteArray,
    InvalidPage,
    InvalidSpriteIndex,
    SizeOverflow,
    OutOfMemory,
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
    if page >= layout.pages {
        return Err(AtlasError::InvalidPage);
    }
    if pixels.len() != layout.sprites.len() {
        return Err(AtlasError::InvalidSpriteArray);
    }
    let side = usize::try_from(layout.page_size).map_err(|_| AtlasError::SizeOverflow)?;
    let bytes = side
        .checked_mul(side)
        .and_then(|n| n.checked_mul(4))
        .ok_or(AtlasError::SizeOverflow)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(bytes)
        .map_err(|_| AtlasError::OutOfMemory)?;
    output.resize(bytes, 0);
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
