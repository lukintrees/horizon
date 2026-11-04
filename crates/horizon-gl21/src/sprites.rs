//! The sprite atlas, filled from the mip chains the game already builds.
//!
//! `horizon_client_render::mipmap::generate` makes a sprite's levels once, in the renderer's own
//! `Image`, and the wgpu path uploads them as a texture array: `(chain, levels)` where the chain is
//! every layer's levels packed end to end. The GL 2.1 path cannot upload an array (no
//! `texture_2d_array` on an X1650), so the same chain has to be laid out as one flat, padded atlas
//! and uploaded as a single texture with a shortened mip chain — `gl21::Atlas` is the layout,
//! `horizon_gl21::Driver::upload` the upload, and this is the step between them: it takes the
//! renderer's chain and produces the `resources::Upload` levels.
//!
//! Two things make that less direct than it sounds:
//!
//! - **The chain is per layer, not per level.** `chain` holds layer 0's levels, then layer 1's, and
//!   the atlas holds every layer's level 0, then every layer's level 1. So only the chain's base
//!   levels are read — one sprite per layer, blitted into its cell — and the atlas's own levels
//!   under it are averaged from the assembled base, exactly as `gl21::Atlas` was written to do
//!   (its padding is what those averages do not cross).
//! - **The atlas's chain is shorter than the sprite's.** A level's padding is `pad / 2^k` texels, so
//!   past `log2(pad)` levels the padding is gone and the level would average a sprite into its
//!   neighbour. `Atlas::mip_levels` says how many levels the padding still protects, and the levels
//!   under that are dropped rather than uploaded wrong: a sprite sampled at the smallest size is a
//!   smear, a sprite sampled across a seam is a wrong colour on a wall.

use horizon_client_render::gl21::Atlas;

use crate::resources::{Level, TextureId, Upload};

/// The atlas's texels for every level the padding protects, and the side of each: what
/// `Driver::upload` takes.
///
/// The pixels are owned rather than borrowed from the chain, because the atlas's levels under the
/// base are not in the chain at all: they are averaged from the assembled base, and that buffer is
/// made here. `levels()` hands out the borrows a single upload needs, and the struct lives as long
/// as that upload call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtlasLevels {
    /// The side of each level, base first: the atlas's own `level_size`.
    sides: Vec<u32>,
    /// Every level's RGBA8 texels, one level after another.
    pixels: Vec<u8>,
}

impl AtlasLevels {
    /// The levels as `glTexImage2D` arguments: one breadcrumb per level, in order.
    pub fn levels(&self) -> Vec<Level<'_>> {
        let mut out = Vec::with_capacity(self.sides.len());
        let mut at = 0usize;
        for (level, side) in self.sides.iter().enumerate() {
            let bytes = (*side as usize) * (*side as usize) * 4;
            out.push(Level {
                level: level as u32,
                width: *side,
                height: *side,
                pixels: &self.pixels[at..at + bytes],
            });
            at += bytes;
        }
        out
    }

    /// How many levels there are: `Atlas::mip_levels` of the atlas this was made for.
    pub fn depth(&self) -> u32 {
        self.sides.len() as u32
    }
}

/// `chain` as an atlas's levels, or the sentence saying why it cannot be.
///
/// `chain` is `levels` per layer, each level's pixels `side × side × 4` for the layer at the level's
/// size, in the order `gfx::mip_chain` writes them; `tile` is the sprite's side at level 0 and
/// `layers` how many sprites the chain holds.
pub fn levels(atlas: &Atlas, chain: &[u8], layers: u32, tile: u32) -> Result<AtlasLevels, String> {
    let levels = atlas.mip_levels();
    // The chain's own size, from the sprite's side down to its last level: what `mip_chain` wrote.
    let chain_levels = tile.max(1).trailing_zeros() + 1;
    let chain_bytes: usize = (0..chain_levels)
        .map(|level| {
            let side = (tile >> level).max(1) as usize;
            side * side * 4 * layers as usize
        })
        .sum();
    if tile != atlas.tile() {
        return Err(format!(
            "the chain's sprites are {tile} texels and the atlas's are {}, so the cells do not line \
             up",
            atlas.tile()
        ));
    }
    if layers > atlas.capacity() {
        return Err(format!(
            "{layers} sprite sheets in an atlas of {} cells: the atlas is too small for them",
            atlas.capacity()
        ));
    }
    if chain.len() != chain_bytes {
        return Err(format!(
            "the chain is {} bytes and {levels} levels of {layers} {tile}-texel sprites come to \
             {chain_bytes}: a level is missing or the sprite count is wrong",
            chain.len()
        ));
    }
    let pixels = assemble(atlas, chain, layers, tile)?;
    Ok(levels_of(atlas, &pixels))
}

/// The atlas's texels for the levels the padding protects: level 0 whole, then each level below it
/// halved from the one above.
///
/// Laid out level by level, so that each `Level` is one contiguous run: `glTexImage2D` takes a level
/// at a time, and the three of a chain are three uploads, not one interleaved buffer.
fn assemble(atlas: &Atlas, chain: &[u8], layers: u32, tile: u32) -> Result<Vec<u8>, String> {
    let mut atlas = atlas.clone();
    let sprite = (tile * tile * 4) as usize;
    for layer in 0..layers as usize {
        let from = layer * sprite;
        atlas.blit(layer as u32, &chain[from..from + sprite], tile)?;
    }
    // The levels under the base are the atlas's own: averaging a padded cell keeps the padding,
    // which is the whole reason `mip_levels` stops where it does.
    let mut levels = vec![atlas.pixels().to_vec()];
    while levels.len() < atlas.mip_levels() as usize {
        let from_side = atlas.level_size(levels.len() as u32 - 1);
        let above = levels.last().cloned().expect("the base level is there");
        levels.push(halve(&above, from_side));
    }
    Ok(levels.concat())
}

/// `pixels` at half its side: the box filter a mip chain is, over four texels of `above`.
fn halve(above: &[u8], from_side: u32) -> Vec<u8> {
    let side = (from_side / 2).max(1);
    let from = from_side as usize;
    let mut out = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side as usize {
        for x in 0..side as usize {
            for channel in 0..4 {
                let at = |x: usize, y: usize| above[(y * from + x) * 4 + channel] as u32;
                let sum = at(x * 2, y * 2)
                    + at(x * 2 + 1, y * 2)
                    + at(x * 2, y * 2 + 1)
                    + at(x * 2 + 1, y * 2 + 1);
                out.push(((sum + 2) / 4) as u8);
            }
        }
    }
    out
}

/// `pixels`, laid out level by level, as the levels an upload takes: each level's side, and the
/// buffer they are one after another in.
fn levels_of(atlas: &Atlas, pixels: &[u8]) -> AtlasLevels {
    let sides: Vec<u32> = (0..atlas.mip_levels())
        .map(|level| atlas.level_size(level))
        .collect();
    let expected: usize = sides
        .iter()
        .map(|side| (*side as usize) * (*side as usize) * 4)
        .sum();
    debug_assert_eq!(pixels.len(), expected);
    AtlasLevels {
        sides,
        pixels: pixels.to_vec(),
    }
}

/// `levels` as the sprite atlas's upload: the breadcrumbs `AtlasLevels::levels` just made, with the
/// texture's own name. Bound to a name first, because the `Upload` borrows them:
///
/// ```ignore
/// let breadcrumbs = atlas_levels.levels();
/// driver.upload(sprites::upload(&breadcrumbs))?;
/// ```
///
pub fn upload<'a>(levels: &'a [Level<'a>]) -> Upload<'a> {
    Upload {
        texture: TextureId::SpriteAtlas,
        levels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sprite sheet of one flat colour per layer, with a one-texel border of a different colour,
    /// so that a blit that reads past the sprite shows up as the border colour.
    fn sheet(layer: u8, tile: u32) -> Vec<u8> {
        let mut out = Vec::new();
        for y in 0..tile {
            for x in 0..tile {
                let edge = x == 0 || y == 0 || x == tile - 1 || y == tile - 1;
                out.extend_from_slice(&[layer, layer, layer, if edge { 0 } else { 255 }]);
            }
        }
        out
    }

    /// `levels` per layer, packed as `gfx::mip_chain` packs them.
    fn chain(layers: u32, tile: u32) -> Vec<u8> {
        let mut out = Vec::new();
        for layer in 0..layers {
            let mut side = tile;
            let mut base = sheet(layer as u8, tile);
            loop {
                out.extend_from_slice(&base);
                if side == 1 {
                    break;
                }
                base = halve(&base, side);
                side = (side / 2).max(1);
            }
        }
        out
    }

    #[test]
    fn a_chain_becomes_the_atlas_s_own_shortened_levels() {
        // Four 16-texel sprites with a texel of padding: the padding protects a level for every
        // doubling of it, and the sprite stops it at its own last texel.
        let atlas = Atlas::new(4, 16, 1, 4096).expect("an atlas");
        let chain = chain(4, 16);
        let atlas_levels = levels(&atlas, &chain, 4, 16).expect("the chain fits");
        assert_eq!(atlas_levels.depth(), atlas.mip_levels());
        // Each level is its own side's worth of texels, and they are handed over in order.
        for (level, made) in atlas_levels.levels().iter().enumerate() {
            let side = atlas.level_size(level as u32);
            assert_eq!(made.width, side, "level {level}");
            assert_eq!(made.height, side, "level {level}");
            assert_eq!(made.level, level as u32);
            assert_eq!(made.pixels.len(), (side * side * 4) as usize);
            assert!(made.fits());
        }
        // And an upload of them is a valid one: the check that stops a driver reading past a level.
        let breadcrumbs = atlas_levels.levels();
        let problems = upload(&breadcrumbs).problems();
        assert_eq!(problems, Vec::<String>::new());
    }

    #[test]
    fn the_base_level_holds_every_layer_and_repeats_its_edge() {
        // Two sprites with a texel of padding, which is what an atlas of 16-texel block sprites
        // gets: enough for the first level, and `mip_levels` stops there.
        let atlas = Atlas::new(2, 4, 1, 4096).expect("an atlas");
        let chain = chain(2, 4);
        let made = levels(&atlas, &chain, 2, 4).expect("the chain fits");
        let base = made.levels()[0].pixels;
        let side = atlas.size() as usize;
        let cell = atlas.cell() as usize;
        // Each cell's corner is the padding, which repeats the sprite's own edge: layer 0's edge is
        // layer 0's colour, and layer 1's is layer 1's, so a blit that read across the seam would
        // show the wrong number here.
        assert_eq!(base[0], 0, "layer 0's padding repeats its own edge");
        assert_eq!(base[cell * 4], 1, "layer 1's padding repeats its own edge");
        // And the padded border is transparent on the sprite's own edge row, as `sheet` drew it: the
        // alpha of a padded texel comes from the sprite, not from the zeroed texture.
        assert_eq!(base[3], 0, "the sprite's own edge alpha, in its padding");
        // The interior of layer 1 sits `pad` texels into its cell.
        let inside = cell + atlas.pad() as usize;
        let at = (inside * side + inside) * 4;
        assert_eq!(base[at], 1, "layer 1's interior, inside its cell");
    }

    #[test]
    fn a_chain_of_the_wrong_size_is_a_sentence_not_a_read_past_the_end() {
        let atlas = Atlas::new(4, 16, 1, 4096).expect("an atlas");
        let chain = chain(4, 16);
        // One sprite short.
        let short = levels(&atlas, &chain[..chain.len() - 64], 4, 16).unwrap_err();
        assert!(short.contains("come to"), "{short}");
        // Sprites the atlas has no cells for.
        let crowded = levels(&atlas, &chain, 5, 16).unwrap_err();
        assert!(crowded.contains("too small"), "{crowded}");
        // And a chain of a different sprite size than the atlas's cells.
        let wrong = levels(&atlas, &chain, 4, 8).unwrap_err();
        assert!(wrong.contains("do not line up"), "{wrong}");
    }

    #[test]
    fn a_half_sided_level_is_the_box_of_four_texels() {
        // A 2×2 of 0s and 255s halves to their mean, and a chain of one colour stays that colour.
        let grey = halve(
            &[
                0, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0, 255, 255, 255, 255,
            ],
            2,
        );
        assert_eq!(grey, vec![128, 128, 128, 128]);
        let flat = vec![7u8; 2 * 2 * 4];
        assert_eq!(halve(&flat, 2), vec![7u8; 4]);
    }
}
