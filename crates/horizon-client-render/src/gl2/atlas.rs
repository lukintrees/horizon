//! The block sprites for OpenGL 2: the texture array wgpu takes (one layer per sprite) repacked into
//! 2D pages, because GL 2.x has no texture arrays on the X1650. Every sprite is `sprite` texels square;
//! a page is `PAGE` texels square and holds a grid of them, filled in layer order.

/// The side of one page in texels. GL 2.x drivers of the X1650's generation take 2048 at least.
pub const PAGE: u32 = 2048;

/// Where a layer sits: its page and its top-left texel in that page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub page: u32,
    pub origin: [u32; 2],
}

/// Every layer of the array, repacked. `pages[i]` is `PAGE * PAGE` RGBA8 texels, row by row from the top.
#[derive(Debug)]
pub struct Atlas {
    pub sprite: u32,
    pub pages: Vec<Vec<u8>>,
    pub placements: Vec<Placement>,
}

/// Repacks `layers` square RGBA8 sprites of `sprite` texels a side, stored layer after layer as
/// `pixels`. The sprite must divide `PAGE`, so that no sprite straddles a page.
pub fn pack(pixels: &[u8], sprite: u32, layers: u32) -> Result<Atlas, String> {
    if sprite == 0 || sprite > PAGE || !PAGE.is_multiple_of(sprite) {
        return Err(format!("a sprite of {sprite} texels does not divide the {PAGE}-texel page"));
    }
    let row = (sprite * 4) as usize;
    let layer_bytes = row * sprite as usize;
    if pixels.len() != layer_bytes * layers as usize {
        return Err(format!("{} bytes for {layers} sprites of {sprite} texels", pixels.len()));
    }
    let per_row = PAGE / sprite;
    let per_page = per_row * per_row;
    let mut pages: Vec<Vec<u8>> = Vec::new();
    let mut placements = Vec::with_capacity(layers as usize);
    for layer in 0..layers {
        let slot = layer % per_page;
        if slot == 0 {
            pages.push(vec![0; (PAGE * PAGE * 4) as usize]);
        }
        let origin = [(slot % per_row) * sprite, (slot / per_row) * sprite];
        let page = pages.last_mut().expect("a page is pushed for every first slot");
        let source = &pixels[layer as usize * layer_bytes..][..layer_bytes];
        for (y, source_row) in source.chunks_exact(row).enumerate() {
            let at = (((origin[1] + y as u32) * PAGE + origin[0]) * 4) as usize;
            page[at..at + row].copy_from_slice(source_row);
        }
        placements.push(Placement { page: layer / per_page, origin });
    }
    Ok(Atlas { sprite, pages, placements })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Layer `layer` of `sprite` texels, every texel `[layer, x, y, 255]`.
    fn layers(sprite: u32, count: u32) -> Vec<u8> {
        let mut out = Vec::new();
        for layer in 0..count {
            for y in 0..sprite {
                for x in 0..sprite {
                    out.extend_from_slice(&[layer as u8, x as u8, y as u8, 255]);
                }
            }
        }
        out
    }

    fn texel(atlas: &Atlas, page: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * PAGE + x) * 4) as usize;
        let p = &atlas.pages[page as usize];
        [p[at], p[at + 1], p[at + 2], p[at + 3]]
    }

    #[test]
    fn each_layer_lands_at_its_placement_with_its_own_texels() {
        let atlas = pack(&layers(4, 3), 4, 3).unwrap();
        assert_eq!(atlas.pages.len(), 1);
        assert_eq!(atlas.placements[0], Placement { page: 0, origin: [0, 0] });
        assert_eq!(atlas.placements[2], Placement { page: 0, origin: [8, 0] });
        // Texel (1, 2) of layer 2 sits at the origin plus (1, 2) in the page.
        assert_eq!(texel(&atlas, 0, 8 + 1, 2), [2, 1, 2, 255]);
        // The texel after layer 2's last column belongs to nobody: it stays empty.
        assert_eq!(texel(&atlas, 0, 12, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn layers_past_a_page_go_to_the_next_page_from_its_top_left() {
        // 1024-texel sprites: four to a page, so layer 4 is the first of page 1.
        let atlas = pack(&layers(1024, 5), 1024, 5).unwrap();
        assert_eq!(atlas.pages.len(), 2);
        assert_eq!(atlas.placements[4], Placement { page: 1, origin: [0, 0] });
        assert_eq!(texel(&atlas, 1, 3, 5), [4, 3, 5, 255]);
    }

    #[test]
    fn a_sprite_that_does_not_divide_the_page_is_refused() {
        assert!(pack(&layers(3, 1), 3, 1).is_err());
        assert!(pack(&[], 0, 0).is_err());
    }

    #[test]
    fn a_pixel_buffer_of_the_wrong_size_is_refused() {
        assert!(pack(&layers(4, 2)[..60], 4, 2).is_err());
    }
}
