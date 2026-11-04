//! A section's solid quads for OpenGL 2: each quad becomes two triangles of plain float vertices,
//! and the quads are grouped by the page their sprite sits in, so a section takes one draw per page
//! run instead of one per quad. The mesher's own vertex is kept as it is (`TerrainVertex`); only its
//! packing is undone: light in levels, the colour in floats, the sprite as a place in its page.

use super::atlas::Atlas;
use crate::mesher::{TerrainVertex, OPAQUE_LAYER};

/// One vertex as the GL 2 terrain shader reads it: 14 floats, 56 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// Section-relative position, in blocks.
    pub pos: [f32; 3],
    /// Position inside the sprite, 1.0 being one sprite. Above 1.0 the sprite repeats (merged faces).
    pub uv: [f32; 2],
    /// The sprite's top-left texel in its page.
    pub origin: [f32; 2],
    /// The sprite's side in texels.
    pub sprite: f32,
    /// Sky light, then block light, in levels 0 to 15.
    pub light: [f32; 2],
    /// The colour the mesher multiplied the sprite by, 0 to 1 per channel.
    pub tint: [f32; 3],
    /// 1.0 for an opaque leaf or a quad over opaque texels: its alpha is not cut out.
    pub opaque: f32,
}

/// A draw of consecutive vertices that all sample one page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub page: u32,
    pub first: u32,
    pub count: u32,
}

/// A section's vertices and their page runs, in the order they are drawn.
#[derive(Debug, Default, PartialEq)]
pub struct Batch {
    pub vertices: Vec<Vertex>,
    pub runs: Vec<Run>,
}

/// Corners of a quad, as two triangles.
const TRIANGLES: [usize; 6] = [0, 1, 2, 0, 2, 3];

/// Batches the solid quads of a section (four vertices each, `SectionMesh::solid`). A quad whose layer
/// the atlas does not have is dropped: its sprite cannot be drawn.
pub fn solid(quads: &[TerrainVertex], atlas: &Atlas) -> Batch {
    batch(quads, atlas, true)
}

/// Batches the translucent quads of a section (`SectionMesh::translucent`, the fluids) in the order
/// the mesher wrote them, so that the order of the quads holds. A run breaks where the page changes.
pub fn translucent(quads: &[TerrainVertex], atlas: &Atlas) -> Batch {
    batch(quads, atlas, false)
}

fn batch(quads: &[TerrainVertex], atlas: &Atlas, by_page: bool) -> Batch {
    let mut order: Vec<(u32, usize)> = Vec::with_capacity(quads.len() / 4);
    for quad in 0..quads.len() / 4 {
        let layer = usize::from(quads[quad * 4].layer & !OPAQUE_LAYER);
        if let Some(placement) = atlas.placements.get(layer) {
            order.push((placement.page, quad));
        }
    }
    if by_page {
        order.sort_by_key(|&(page, _)| page);
    }

    let mut batch = Batch { vertices: Vec::with_capacity(order.len() * 6), runs: Vec::new() };
    for &(page, quad) in &order {
        let first = batch.vertices.len() as u32;
        for &corner in &TRIANGLES {
            batch.vertices.push(vertex(&quads[quad * 4 + corner], atlas));
        }
        match batch.runs.last_mut() {
            Some(run) if run.page == page => run.count += 6,
            _ => batch.runs.push(Run { page, first, count: 6 }),
        }
    }
    batch
}

fn vertex(v: &TerrainVertex, atlas: &Atlas) -> Vertex {
    let layer = usize::from(v.layer & !OPAQUE_LAYER);
    let place = atlas.placements[layer];
    let colour = v.color.to_le_bytes();
    Vertex {
        pos: v.pos,
        uv: v.uv(),
        origin: [place.origin[0] as f32, place.origin[1] as f32],
        sprite: atlas.sprite as f32,
        light: [f32::from(v.light & 0xff) / 16.0, f32::from(v.light >> 8) / 16.0],
        tint: [colour[0], colour[1], colour[2]].map(|c| f32::from(c) / 255.0),
        opaque: if v.layer & OPAQUE_LAYER != 0 { 1.0 } else { 0.0 },
    }
}

#[cfg(test)]
mod tests {
    use super::super::atlas::{self, Atlas};
    use super::*;

    /// A page-less atlas that places each layer at its own origin in page 0 or 1.
    fn two_pages() -> Atlas {
        let sprite = 16;
        let pixels = vec![0u8; (sprite * sprite * 4) as usize * 2];
        let mut atlas = atlas::pack(&pixels, sprite, 2).unwrap();
        atlas.placements[1] = atlas::Placement { page: 1, origin: [32, 48] };
        atlas
    }

    fn quad(layer: u16, light: u16, color: u32) -> [TerrainVertex; 4] {
        let corners = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        corners.map(|pos| TerrainVertex::new(pos, [0.5, 0.25], layer, color, light))
    }

    #[test]
    fn a_quad_becomes_two_triangles_in_the_mesher_corner_order() {
        let atlas = two_pages();
        let batch = solid(&quad(0, 0, 0xFF_FFFF), &atlas);
        assert_eq!(batch.vertices.len(), 6);
        let positions: Vec<[f32; 3]> = batch.vertices.iter().map(|v| v.pos).collect();
        assert_eq!(
            positions,
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]
        );
    }

    #[test]
    fn translucent_quads_keep_the_order_the_mesher_wrote_them_in() {
        let atlas = two_pages();
        let mut quads = quad(1, 0, 0xFF_FFFF).to_vec();
        quads.extend(quad(0, 0, 0xFF_FFFF));
        let batch = translucent(&quads, &atlas);
        assert_eq!(batch.runs, vec![Run { page: 1, first: 0, count: 6 }, Run { page: 0, first: 6, count: 6 }]);
    }

    #[test]
    fn light_is_sky_in_the_low_byte_and_block_in_the_high_byte_in_levels() {
        let atlas = two_pages();
        // The mesher stores 16x fixed point in each byte: sky 3 in the low byte (0x30), block 15 in the high one (0xF0).
        let batch = solid(&quad(0, 0xF030, 0xFF_FFFF), &atlas);
        assert_eq!(batch.vertices[0].light, [3.0, 15.0]);
    }

    #[test]
    fn the_colour_is_the_mesher_rgb_and_the_sprite_its_page_place() {
        let atlas = two_pages();
        let batch = solid(&quad(1, 0, 0x0010_2030), &atlas);
        let v = batch.vertices[0];
        assert_eq!(v.tint, [48.0 / 255.0, 32.0 / 255.0, 16.0 / 255.0]);
        assert_eq!(v.origin, [32.0, 48.0]);
        assert_eq!(v.sprite, 16.0);
        assert_eq!(v.uv, [0.5, 0.25]);
    }

    #[test]
    fn the_opaque_bit_is_read_and_masked_off_the_layer() {
        let atlas = two_pages();
        let batch = solid(&quad(OPAQUE_LAYER, 0, 0xFF_FFFF), &atlas);
        assert_eq!(batch.vertices[0].opaque, 1.0);
        assert_eq!(batch.vertices[0].origin, [0.0, 0.0]);
    }

    #[test]
    fn quads_are_grouped_into_one_run_per_page_in_page_order() {
        let atlas = two_pages();
        let mut quads = Vec::new();
        quads.extend_from_slice(&quad(1, 0, 0xFF_FFFF));
        quads.extend_from_slice(&quad(0, 0, 0xFF_FFFF));
        quads.extend_from_slice(&quad(0, 0, 0xFF_FFFF));
        let batch = solid(&quads, &atlas);
        assert_eq!(batch.vertices.len(), 18);
        assert_eq!(
            batch.runs,
            [Run { page: 0, first: 0, count: 12 }, Run { page: 1, first: 12, count: 6 }]
        );
    }

    #[test]
    fn a_quad_whose_layer_the_atlas_lacks_is_dropped() {
        let atlas = two_pages();
        let batch = solid(&quad(7, 0, 0xFF_FFFF), &atlas);
        assert_eq!(batch, Batch::default());
    }
}
