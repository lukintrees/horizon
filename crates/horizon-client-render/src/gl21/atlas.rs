//! A flat sprite atlas: the OpenGL 2.1 replacement for a texture array.
//!
//! `texture_2d_array` is GL 3.0, and the X1650 has none. The renderer's block sprites live in
//! arrays of up to 2048 layers, 16 or 32 texels a side each (`gfx.rs`'s `SPRITE_SPLIT`); here the
//! same sprites are laid out in a square, power-of-two 2D texture, so that a shader names a sprite by
//! a rectangle of one texture instead of a layer of an array.
//!
//! Three things decide the layout, and the renderer above must respect all three:
//!
//! - **Padding**: each sprite sits in a cell with `pad` texels of its own edge colours around it
//!   (`blit`). Without them a mip level averages the sprite with its neighbour, and a grass block's
//!   top row turns the colour of the stone beside it. This is the `docs/opengl21-plan.md` §5.2
//!   requirement, and what the layout is for.
//! - **A mip chain that stops before the padding does**: at mip level `k` the padding is
//!   `pad / 2^k` texels, so past `log2(pad)` texels there is no padding left and the chain must stop
//!   (`mip_levels`). One texel of padding at the last level is the least that still keeps a sprite's
//!   edges its own.
//! - **Power of two**: R300/R500 cannot mipmap a non-power-of-two texture (`LimitedNPOT`), so the
//!   texture's side is a power of two. The cells inside it need not be: `size` is the next power of
//!   two past `cols × cell`, and a texture with a margin right and bottom is still a fine texture.
//!
//! The layout is arithmetic and the blit is a clamped copy, so both are checked without a GPU
//! (`tests` at the bottom).

/// A square atlas of `tile`×`tile` sprites, each in a `tile + 2 × pad` cell, `pixels` RGBA8,
/// `size`×`size` texels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atlas {
    size: u32,
    tile: u32,
    pad: u32,
    /// Cells across, the fewest square-ish count that holds the sprites.
    cols: u32,
    pixels: Vec<u8>,
}

impl Atlas {
    /// An atlas holding `layers` sprites of `tile`×`tile` texels, each with `pad` texels of its own
    /// edges around it, in a texture no larger than `max_size`. Errors when a side is not a power of
    /// two, when a cell cannot fit the texture, or when one texture cannot hold `layers` cells.
    pub fn new(layers: u32, tile: u32, pad: u32, max_size: u32) -> Result<Atlas, String> {
        if tile == 0 || !tile.is_power_of_two() {
            return Err(format!("a sprite of {tile} texels is not a power of two"));
        }
        if pad != 0 && !pad.is_power_of_two() {
            return Err(format!("a padding of {pad} texels is not a power of two"));
        }
        if pad > tile {
            return Err(format!(
                "a padding of {pad} texels around a {tile}-texel sprite is wider than the sprite"
            ));
        }
        let cell = tile + 2 * pad;
        if cell > max_size {
            return Err(format!(
                "a {cell}-texel cell does not fit a {max_size}-texel texture"
            ));
        }
        let mut cols = 1;
        while cols * cols < layers {
            cols += 1;
        }
        let size = (cols * cell).next_power_of_two();
        if size > max_size {
            return Err(format!(
                "{layers} sprites of {tile} texels need more than one {max_size}-texel atlas"
            ));
        }
        Ok(Atlas {
            size,
            tile,
            pad,
            cols,
            pixels: vec![0; (size * size * 4) as usize],
        })
    }

    /// The side of the texture, in texels.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// The side of one sprite, in texels, without its padding.
    pub fn tile(&self) -> u32 {
        self.tile
    }

    /// The padding around one sprite, in texels.
    pub fn pad(&self) -> u32 {
        self.pad
    }

    /// The side of one sprite's cell, padding included.
    pub fn cell(&self) -> u32 {
        self.tile + 2 * self.pad
    }

    /// How many sprites fit.
    pub fn capacity(&self) -> u32 {
        self.cols * self.cols
    }

    /// How many mip levels may be built: the levels whose padding is still a whole texel, so that no
    /// level averages a sprite with its neighbour. A level's padding is `pad / 2^k`, and past the
    /// sprite's own last texel there is nothing left to average anyway.
    pub fn mip_levels(&self) -> u32 {
        if self.pad == 0 {
            return 1;
        }
        self.pad.trailing_zeros().min(self.tile.trailing_zeros()) + 1
    }

    /// The side of the texture at `level`: half of the level before, as a mip chain is. Past the
    /// last level `mip_levels` allows, the last level's side.
    pub fn level_size(&self, level: u32) -> u32 {
        let last = self.mip_levels().saturating_sub(1);
        (self.size >> level.min(last)).max(1)
    }

    /// Where `layer`'s sprite sits, padding excluded: (u0, v0, u1, v1) in the atlas's `[0, 1]`, or
    /// `None` past `capacity`. These four numbers are a draw's `u_sprite` (`terrain.vert`).
    pub fn rect(&self, layer: u32) -> Option<[f32; 4]> {
        if layer >= self.capacity() {
            return None;
        }
        let side = self.size as f32;
        let (x, y) = self.origin(layer);
        let x = x + self.pad;
        let y = y + self.pad;
        Some([
            x as f32 / side,
            y as f32 / side,
            (x + self.tile) as f32 / side,
            (y + self.tile) as f32 / side,
        ])
    }

    /// The texel column and row of `layer`'s cell corner, padding included.
    fn origin(&self, layer: u32) -> (u32, u32) {
        let cell = self.cell();
        let col = layer % self.cols;
        let row = layer / self.cols;
        (col * cell, row * cell)
    }

    /// Copy `layer` from one sprite's `source_side`×`source_side` RGBA8 texels, in rows: the
    /// `tile`×`tile`×4 bytes of one layer of the mip chains the renderer already builds. The cell's
    /// padding is filled by repeating the sprite's own edge texels, which is what keeps a mip level
    /// from averaging across the seam.
    pub fn blit(&mut self, layer: u32, source: &[u8], source_side: u32) -> Result<(), String> {
        let expected = (source_side * source_side * 4) as usize;
        if source.len() != expected {
            return Err(format!(
                "layer {layer}: {} bytes for a {source_side}-texel sheet, {expected} expected",
                source.len()
            ));
        }
        if source_side != self.tile {
            return Err(format!(
                "layer {layer}: a {source_side}-texel sheet in a {}-texel sprite",
                self.tile
            ));
        }
        if layer >= self.capacity() {
            return Err(format!(
                "layer {layer} past the atlas's {} sprites",
                self.capacity()
            ));
        }
        let (x0, y0) = self.origin(layer);
        let cell = self.cell();
        let last = self.tile as i32 - 1;
        for row in 0..cell {
            // The source row this cell row shows: the sprite's own, clamped into its edges, so that
            // the padding repeats the edge instead of bleeding in from outside the sprite.
            let source_row = (row as i32 - self.pad as i32).clamp(0, last) as u32;
            let to = ((y0 + row) * self.size + x0) as usize * 4;
            for col in 0..cell {
                let source_col = (col as i32 - self.pad as i32).clamp(0, last) as u32;
                let from = (source_row * self.tile + source_col) as usize * 4;
                let at = to + col as usize * 4;
                let texel = &source[from..from + 4];
                self.pixels[at..at + 4].copy_from_slice(texel);
            }
        }
        Ok(())
    }

    /// The atlas's pixels, RGBA8 rows.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The RGBA of one texel: `x` and `y` from the top left.
    pub fn texel(&self, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * self.size + x) * 4) as usize;
        [
            self.pixels[at],
            self.pixels[at + 1],
            self.pixels[at + 2],
            self.pixels[at + 3],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `side`×`side` sheet whose texel `(x, y)` is `(x, y, 0, 255)`: enough to see which texel a
    /// cell holds where.
    fn sheet(side: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((side * side * 4) as usize);
        for y in 0..side {
            for x in 0..side {
                out.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
        }
        out
    }

    #[test]
    fn the_atlas_is_a_power_of_two_and_holds_what_it_says() {
        let one = Atlas::new(1, 16, 4, 4096).unwrap();
        assert_eq!((one.size(), one.tile(), one.cell()), (32, 16, 24));
        assert_eq!(one.capacity(), 1);
        // 256 sprites of 24-texel cells: 16 across is 384 texels, rounded up to 512.
        let many = Atlas::new(256, 16, 4, 4096).unwrap();
        assert_eq!((many.size(), many.capacity()), (512, 256));
        // Without padding a cell is the sprite: 256 of 16 fit 256 texels, as a texture array would.
        let tight = Atlas::new(256, 16, 0, 4096).unwrap();
        assert_eq!((tight.size(), tight.cell()), (256, 16));
        for atlas in [&one, &many, &tight] {
            assert!(atlas.size().is_power_of_two());
        }
    }

    #[test]
    fn another_sprite_side_changes_how_many_fit() {
        // 32-texel sprites, 4 texels of padding: 40-texel cells, 128 across is 5120 texels: more
        // than one 4096 texture.
        assert!(Atlas::new(16_384, 32, 4, 4096).is_err());
        // 16 texels of padding around a 32-texel sprite: 64-texel cells, 64 across in 4096.
        let atlas = Atlas::new(4096, 32, 16, 4096).unwrap();
        assert_eq!((atlas.size(), atlas.capacity()), (4096, 4096));
        // Vanilla's sprites without padding in a 4096-texel texture still leave one atlas.
        assert!(Atlas::new(65_536, 16, 0, 4096).is_ok());
        assert!(Atlas::new(65_537, 16, 0, 4096).is_err());
    }

    /// The error a refused atlas gives, with the sentence that says why it matters.
    fn refusal(layers: u32, tile: u32, pad: u32, max_size: u32) -> String {
        match Atlas::new(layers, tile, pad, max_size) {
            Ok(_) => panic!("a {tile}-texel sprite with {pad} of padding was accepted"),
            Err(message) => message,
        }
    }

    #[test]
    fn wrong_sides_are_refused() {
        assert!(refusal(1, 24, 0, 4096).contains("power of two"));
        assert!(refusal(1, 0, 0, 4096).contains("power of two"));
        assert!(refusal(1, 16, 3, 4096).contains("padding"));
        let wide = refusal(1, 16, 32, 4096);
        assert!(wide.contains("wider than the sprite"), "{wide}");
        assert!(refusal(1, 8192, 0, 4096).contains("does not fit"));
    }

    #[test]
    fn a_sprite_sits_inside_its_cell_with_the_padding_outside_it() {
        // One 2-texel sprite with 1 texel of padding: a 4-texel cell in a 4-texel texture.
        let atlas = Atlas::new(1, 2, 1, 4096).unwrap();
        assert_eq!((atlas.size(), atlas.cell()), (4, 4));
        // The sprite is the middle half; the padding is the ring around it.
        assert_eq!(atlas.rect(0), Some([0.25, 0.25, 0.75, 0.75]));
        // Four sprites of 2 with no padding: a 4-texel texture, one quadrant each.
        let tight = Atlas::new(4, 2, 0, 4096).unwrap();
        assert_eq!(tight.rect(0), Some([0.0, 0.0, 0.5, 0.5]));
        assert_eq!(tight.rect(2), Some([0.0, 0.5, 0.5, 1.0]));
        assert_eq!(tight.rect(4), None);
        // What a draw of one sprite binds as `u_sprite`: its rectangle. `terrain.vert` maps the
        // vertex's sprite-local UV into it: a point `f` of the sprite is `r.xy + f * (r.zw - r.xy)`.
        let sprite = tight.rect(3).unwrap();
        let into = |f: f32| sprite[0] + f * (sprite[2] - sprite[0]);
        assert_eq!((into(0.0), into(0.5), into(1.0)), (0.5, 0.75, 1.0));
    }

    #[test]
    fn the_mip_chain_stops_where_the_padding_would() {
        // No padding: one level, the sprite's own texels.
        assert_eq!(Atlas::new(1, 16, 0, 4096).unwrap().mip_levels(), 1);
        // 1 texel: levels 0 only, since level 1 would have half a texel of padding.
        assert_eq!(Atlas::new(1, 16, 1, 4096).unwrap().mip_levels(), 1);
        // 2 texels: levels 0 and 1 (1 texel of padding left).
        assert_eq!(Atlas::new(1, 16, 2, 4096).unwrap().mip_levels(), 2);
        // 4: levels 0, 1, 2.
        assert_eq!(Atlas::new(1, 16, 4, 4096).unwrap().mip_levels(), 3);
        // Padding can never outlast the sprite: 16 texels of padding around 16 texels is four
        // levels of sprite, and the fifth would be a single texel.
        assert_eq!(Atlas::new(1, 16, 16, 4096).unwrap().mip_levels(), 5);
        // And the levels halve the texture, as a mip chain does.
        let atlas = Atlas::new(1, 16, 4, 4096).unwrap();
        assert_eq!(
            (0..atlas.mip_levels())
                .map(|level| atlas.level_size(level))
                .collect::<Vec<_>>(),
            vec![32, 16, 8]
        );
    }

    #[test]
    fn blitting_puts_the_sprite_in_and_its_edges_around_it() {
        // Two 2-texel sprites with 1 texel of padding: cells of 4, two across, a texture of 8.
        let mut atlas = Atlas::new(2, 2, 1, 4096).unwrap();
        assert_eq!((atlas.size(), atlas.origin(1)), (8, (4, 0)));
        atlas.blit(0, &sheet(2), 2).unwrap();
        // The sprite's own texels, at cell corner (0, 0) plus the padding.
        assert_eq!(atlas.texel(1, 1), [0, 0, 0, 255]);
        assert_eq!(atlas.texel(2, 2), [1, 1, 0, 255]);
        // The padding repeats them: the left column is the sprite's left column, the top row its
        // top row, and the corner texel both.
        assert_eq!(atlas.texel(0, 0), [0, 0, 0, 255]);
        assert_eq!(atlas.texel(0, 2), [0, 1, 0, 255]);
        assert_eq!(atlas.texel(3, 0), [1, 0, 0, 255]);
        assert_eq!(atlas.texel(3, 3), [1, 1, 0, 255]);
        // The next cell is untouched until its own blit.
        assert_eq!(atlas.texel(4, 0), [0, 0, 0, 0]);
        atlas.blit(1, &sheet(2), 2).unwrap();
        assert_eq!(atlas.texel(4, 0), [0, 0, 0, 255]);
    }

    /// The error a refused blit gives.
    fn blit_refusal(atlas: &mut Atlas, layer: u32, source: &[u8], side: u32) -> String {
        match atlas.blit(layer, source, side) {
            Ok(()) => panic!("layer {layer} took a {side}-texel sheet"),
            Err(message) => message,
        }
    }

    #[test]
    fn a_blit_of_the_wrong_size_or_layer_is_refused() {
        // One sprite of 2 texels: the only layer there is.
        let mut atlas = Atlas::new(1, 2, 0, 4096).unwrap();
        assert_eq!(atlas.capacity(), 1);
        assert!(atlas.blit(0, &sheet(2), 2).is_ok());
        assert!(blit_refusal(&mut atlas, 0, &[0; 8], 2).contains("bytes"));
        let wide = blit_refusal(&mut atlas, 0, &sheet(4), 4);
        assert!(wide.contains("texel sprite"), "{wide}");
        let past = blit_refusal(&mut atlas, 1, &sheet(2), 2);
        assert!(past.contains("past the atlas"), "{past}");
    }
}
