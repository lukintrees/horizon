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

/// One mip level's bounded sub-update: where level `k` of the page holds the
/// sprite, and the sprite's level-`k` pixels with the nearest content texel
/// repeated into the border that level still has room for.
pub struct SpritePatchLevel {
    pub at: [u32; 2],
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
}

/// Where one sprite's level-`level` texels go on that level of the page.
///
/// Level `k` of a texture holds level-0 texel `(x, y)` at `(x >> k, y >> k)`, so
/// the sprite's content has to start at exactly `place.x >> k` there; the border
/// around it is whatever the halved padding still leaves, on each side
/// separately. A box that was rounded instead would move a sprite whose page x
/// is odd by one texel and seam its distant levels.
pub struct MipLevelBox {
    /// The box's top-left corner, padding included.
    pub at: [u32; 2],
    /// The box's size, padding included.
    pub size: [u32; 2],
    /// Where the content starts inside the box.
    pub pad: [u32; 2],
}

pub fn mip_level_box(
    layout: &AtlasLayout,
    place: &Placement,
    level: u32,
) -> Option<MipLevelBox> {
    let page = layout.page_size >> level;
    let size = [place.width >> level, place.height >> level];
    if page < 2 || size[0] == 0 || size[1] == 0 {
        return None;
    }
    let mut at = [0u32; 2];
    let mut full = [0u32; 2];
    let mut pad = [0u32; 2];
    for axis in 0..2 {
        let origin = if axis == 0 { place.x } else { place.y };
        let span = if axis == 0 { place.width } else { place.height };
        // Packing keeps `padding` around every sprite, so the content ends inside
        // the page at every level and the box only has to cover it.
        let content = origin >> level;
        let low = (origin.saturating_sub(layout.padding) >> level).min(content);
        let high = (origin + span + layout.padding) >> level;
        at[axis] = low;
        full[axis] = high.min(page) - low;
        pad[axis] = content - low;
    }
    Some(MipLevelBox { at, size: full, pad })
}

/// One sprite's box at mip level `level` from its unpadded `size`-sized pixels at
/// that level: `full` texels each way, the content starting `pad` texels in and
/// the rest repeating the nearest content texel, as `build_page` pads level zero.
pub fn padded_mip_level(
    pixels: &[u8],
    size: [u32; 2],
    pad: [u32; 2],
    full: [u32; 2],
) -> Vec<u8> {
    let mut out = vec![0u8; (full[0] as usize) * (full[1] as usize) * 4];
    for y in 0..full[1] {
        let sy = y.saturating_sub(pad[1]).min(size[1] - 1);
        for x in 0..full[0] {
            let sx = x.saturating_sub(pad[0]).min(size[0] - 1);
            let from = ((sy * size[0] + sx) * 4) as usize;
            let to = ((y * full[0] + x) * 4) as usize;
            out[to..to + 4].copy_from_slice(&pixels[from..from + 4]);
        }
    }
    out
}

/// The bounded sub-updates for a sprite's mip levels, for
/// [`crate::scene::WorldScene::update_sprite_mipped`]: entry `k - 1` is level `k`,
/// built from that level's own `mips[k - 1]` pixels the way the page's level was.
/// Level zero is [`sprite_patch`]'s.
pub fn sprite_patch_levels(
    layout: &AtlasLayout,
    index: usize,
    mips: &[&[u8]],
    max_bytes: usize,
) -> Result<Vec<SpritePatchLevel>, AtlasError> {
    let place = layout
        .sprites()
        .get(index)
        .ok_or(AtlasError::InvalidSpriteIndex)?;
    let mut out = Vec::with_capacity(mips.len());
    let mut budget = 0usize;
    for (level, pixels) in mips.iter().enumerate() {
        let level = level as u32 + 1;
        let size = [place.width >> level, place.height >> level];
        let expected = (size[0] as usize)
            .checked_mul(size[1] as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(AtlasError::SizeOverflow)?;
        if pixels.len() != expected {
            return Err(AtlasError::InvalidPixels { index });
        }
        let bounds = mip_level_box(layout, place, level).ok_or(AtlasError::InvalidPageSize)?;
        let rgba = padded_mip_level(pixels, size, bounds.pad, bounds.size);
        budget = budget
            .checked_add(rgba.len())
            .filter(|n| *n <= max_bytes)
            .ok_or(AtlasError::SizeOverflow)?;
        out.push(SpritePatchLevel {
            at: bounds.at,
            size: bounds.size,
            rgba,
        });
    }
    Ok(out)
}

/// A bounded sprite sub-update, including extruded padding. Does not touch
/// neighbours or change placement/layer IDs; only level zero is represented.
pub struct SpritePatch {
    pub page: usize,
    pub at: [u32; 2],
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
}
pub fn sprite_patch(
    layout: &AtlasLayout,
    index: usize,
    rgba: &[u8],
    max_bytes: usize,
) -> Result<SpritePatch, AtlasError> {
    let place = layout
        .sprites
        .get(index)
        .ok_or(AtlasError::InvalidSpriteIndex)?;
    let expected = (place.width as usize)
        .checked_mul(place.height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or(AtlasError::SizeOverflow)?;
    if rgba.len() != expected {
        return Err(AtlasError::InvalidPixels { index });
    }
    let border = layout
        .padding
        .checked_mul(2)
        .ok_or(AtlasError::SizeOverflow)?;
    let width = place
        .width
        .checked_add(border)
        .ok_or(AtlasError::SizeOverflow)?;
    let height = place
        .height
        .checked_add(border)
        .ok_or(AtlasError::SizeOverflow)?;
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n <= max_bytes)
        .ok_or(AtlasError::SizeOverflow)?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(bytes)
        .map_err(|_| AtlasError::OutOfMemory)?;
    pixels.resize(bytes, 0);
    for y in 0..height {
        for x in 0..width {
            let sx = x.saturating_sub(layout.padding).min(place.width - 1);
            let sy = y.saturating_sub(layout.padding).min(place.height - 1);
            let from = (sy as usize * place.width as usize + sx as usize) * 4;
            let to = (y as usize * width as usize + x as usize) * 4;
            pixels[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
        }
    }
    Ok(SpritePatch {
        page: place.page,
        at: [place.x - layout.padding, place.y - layout.padding],
        size: [width, height],
        rgba: pixels,
    })
}

#[cfg(test)]
mod mip_box_tests {
    use super::*;
    /// A level's box must put the sprite's content exactly where the level's own
    /// halving puts it (`place >> level`), or a sprite placed at an odd coordinate
    /// shifts a texel at distance and its mips seam against the level-zero page.
    #[test]
    fn every_level_places_content_where_that_levels_halving_does() {
        let layout = pack_atlas(
            &[SpriteSize {
                width: 4,
                height: 4,
            }; 2],
            16,
            1,
            1,
        )
        .unwrap();
        // The second sprite starts at an odd page coordinate, the case rounding breaks.
        assert_eq!(layout.sprites()[1].x % 2, 1);
        for place in layout.sprites() {
            for level in 1..=2 {
                let bounds = mip_level_box(&layout, place, level).unwrap();
                assert_eq!(
                    bounds.at[0] + bounds.pad[0],
                    place.x >> level,
                    "level {level} x"
                );
                assert_eq!(
                    bounds.at[1] + bounds.pad[1],
                    place.y >> level,
                    "level {level} y"
                );
                let page = layout.page_size() >> level;
                for axis in 0..2 {
                    assert!(bounds.at[axis] + bounds.size[axis] <= page);
                    assert!(bounds.size[axis] >= place.width >> level);
                }
            }
        }
        // One level past a 4 px sprite has nothing to hold.
        assert!(mip_level_box(&layout, &layout.sprites()[0], 3).is_none());
    }
    /// The padding repeats the nearest content texel, as level zero's does, so a
    /// sprite's border never samples a neighbour at distance.
    #[test]
    fn the_border_repeats_the_edge_texel_on_every_side() {
        let content = [1u8, 2, 3, 4];
        let padded = padded_mip_level(&content, [1, 1], [1, 1], [3, 3]);
        assert_eq!(padded, content.repeat(9));
    }
}
