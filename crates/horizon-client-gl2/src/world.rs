//! The terrain's blocks and its section meshes (docs/plans/opengl21-radeon-x1650.md, phase 2).
//!
//! The world is split into 16x16x16 sections, as vanilla does. A section's mesh has one quad per
//! block face that touches air, and the faces against solid neighbours are left out, neighbours in
//! other sections included. Each vertex is six floats: position, atlas UV, and the face's shade.
//! The mesh is plain float data, so OpenGL 2.1 can draw it from a vertex buffer. The game's own
//! mesher (horizon-client-render) packs its vertices into u32 for wgpu and cannot run here.

/// Blocks per section along each axis.
pub const SECTION: i32 = 16;
/// The procedural world: 8 by 8 sections across, and 2 sections high.
pub const WORLD_XZ: i32 = 128;
pub const WORLD_Y: i32 = 32;
/// Floats per vertex: x, y, z, u, v, shade.
pub const FLOATS_PER_VERTEX: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    Air,
    Grass,
    Dirt,
    Stone,
}

impl Block {
    pub fn is_solid(self) -> bool {
        self != Block::Air
    }
}

/// The atlas tiles, numbered in the order they sit in the atlas: four to a row, from the top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tile {
    GrassTop = 0,
    GrassSide = 1,
    Dirt = 2,
    Stone = 3,
}

pub struct World {
    size_xz: i32,
    size_y: i32,
    blocks: Vec<Block>,
}

impl World {
    /// A world of `size_xz` by `size_y` by `size_xz` blocks, each from `block_at(x, y, z)`.
    pub fn new(size_xz: i32, size_y: i32, block_at: impl Fn(i32, i32, i32) -> Block) -> Self {
        let mut blocks = Vec::with_capacity((size_xz * size_y * size_xz) as usize);
        for y in 0..size_y {
            for z in 0..size_xz {
                for x in 0..size_xz {
                    blocks.push(block_at(x, y, z));
                }
            }
        }
        Self { size_xz, size_y, blocks }
    }

    /// The hilly ground the terrain check runs on: grass over three layers of dirt over stone.
    pub fn procedural() -> Self {
        Self::new(WORLD_XZ, WORLD_Y, |x, y, z| {
            let top = surface_height(x, z);
            if y >= top {
                Block::Air
            } else if y == top - 1 {
                Block::Grass
            } else if y >= top - 3 {
                Block::Dirt
            } else {
                Block::Stone
            }
        })
    }

    pub fn size_xz(&self) -> i32 {
        self.size_xz
    }

    pub fn size_y(&self) -> i32 {
        self.size_y
    }

    /// The block at (x, y, z); anything outside the world is air.
    pub fn block(&self, x: i32, y: i32, z: i32) -> Block {
        if x < 0 || z < 0 || y < 0 || x >= self.size_xz || z >= self.size_xz || y >= self.size_y {
            return Block::Air;
        }
        self.blocks[((y * self.size_xz + z) * self.size_xz + x) as usize]
    }
}

/// The height of the ground in blocks, between about 7 and 17.
fn surface_height(x: i32, z: i32) -> i32 {
    let (x, z) = (x as f32, z as f32);
    let h = 12.0 + 3.0 * (x * 0.19).sin() + 2.0 * (z * 0.23).cos() + 2.0 * ((x + z) * 0.07).sin();
    h.round() as i32
}

/// A section's vertices and how many faces they make up.
pub struct Mesh {
    pub vertices: Vec<f32>,
    pub faces: u32,
}

impl Mesh {
    pub fn vertex_count(&self) -> usize {
        self.vertices.len() / FLOATS_PER_VERTEX
    }
}

/// Faces as (axis, sign, shade). Shades follow vanilla's directional light: top 1.0, bottom 0.5,
/// north and south 0.8, east and west 0.6.
const FACES: [(usize, i32, f32); 6] = [
    (1, 1, 1.0),
    (1, -1, 0.5),
    (2, 1, 0.8),
    (2, -1, 0.8),
    (0, 1, 0.6),
    (0, -1, 0.6),
];

/// The corners of a face, as (s, t) on its two tangent axes, in triangle order (two triangles).
const QUAD: [(f32, f32); 4] = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
const TRIANGLES: [usize; 6] = [0, 1, 2, 0, 2, 3];
/// Half a texel of a 64-texel atlas, so the sampling never reaches the next tile.
const INSET: f32 = 0.5 / 64.0;

/// The mesh of section (sx, sy, sz), in blocks: its blocks are `sx * SECTION` and so on.
pub fn build_section(world: &World, sx: i32, sy: i32, sz: i32) -> Mesh {
    let mut vertices = Vec::new();
    let mut faces = 0;
    for ly in 0..SECTION {
        for lz in 0..SECTION {
            for lx in 0..SECTION {
                let (x, y, z) = (sx * SECTION + lx, sy * SECTION + ly, sz * SECTION + lz);
                let block = world.block(x, y, z);
                if !block.is_solid() {
                    continue;
                }
                for &(axis, sign, shade) in &FACES {
                    let mut neighbour = [x, y, z];
                    neighbour[axis] += sign;
                    if world.block(neighbour[0], neighbour[1], neighbour[2]).is_solid() {
                        continue;
                    }
                    let tile = tile_for(block, axis, sign);
                    push_face(&mut vertices, [x as f32, y as f32, z as f32], axis, sign, shade, tile);
                    faces += 1;
                }
            }
        }
    }
    Mesh { vertices, faces }
}

fn tile_for(block: Block, axis: usize, sign: i32) -> Tile {
    match block {
        Block::Grass if axis == 1 && sign > 0 => Tile::GrassTop,
        Block::Grass if axis == 1 => Tile::Dirt,
        Block::Grass => Tile::GrassSide,
        Block::Dirt => Tile::Dirt,
        // Air never reaches here: `build_section` skips it.
        Block::Stone | Block::Air => Tile::Stone,
    }
}

fn push_face(out: &mut Vec<f32>, base: [f32; 3], axis: usize, sign: i32, shade: f32, tile: Tile) {
    let tile = tile as u32;
    let (col, row) = ((tile % 4) as f32, (tile / 4) as f32);
    let span = 0.25 - 2.0 * INSET;
    let corners: Vec<[f32; 3]> = QUAD.iter().map(|&(s, t)| local_corner(axis, sign, s, t)).collect();
    for &i in &TRIANGLES {
        let local = corners[i];
        // Side faces show the block's height along v, so grass sits on top; top and bottom faces use x and z.
        let (u, v) = if axis == 1 {
            (local[0], local[2])
        } else {
            let horizontal = if axis == 0 { local[2] } else { local[0] };
            (horizontal, 1.0 - local[1])
        };
        out.extend_from_slice(&[
            base[0] + local[0],
            base[1] + local[1],
            base[2] + local[2],
            col / 4.0 + INSET + u * span,
            row / 4.0 + INSET + v * span,
            shade,
        ]);
    }
}

/// A corner of the unit block on the face (axis, sign), with (s, t) along its tangent axes.
fn local_corner(axis: usize, sign: i32, s: f32, t: f32) -> [f32; 3] {
    let mut p = [0.0; 3];
    p[axis] = if sign > 0 { 1.0 } else { 0.0 };
    p[(axis + 1) % 3] = s;
    p[(axis + 2) % 3] = t;
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single_block(at: (i32, i32, i32)) -> World {
        World::new(32, 16, move |x, y, z| if (x, y, z) == at { Block::Stone } else { Block::Air })
    }

    #[test]
    fn a_lone_block_makes_six_faces() {
        let mesh = build_section(&single_block((5, 5, 5)), 0, 0, 0);
        assert_eq!(mesh.faces, 6);
        assert_eq!(mesh.vertex_count(), 36);
    }

    #[test]
    fn every_vertex_of_a_lone_block_lies_on_its_surface_box() {
        let mesh = build_section(&single_block((5, 5, 5)), 0, 0, 0);
        for v in mesh.vertices.chunks(FLOATS_PER_VERTEX) {
            for axis in 0..3 {
                assert!((5.0..=6.0).contains(&v[axis]), "vertex {v:?} is outside the block");
            }
        }
        let top = mesh.vertices.chunks(FLOATS_PER_VERTEX).filter(|v| v[5] == 1.0).count();
        assert_eq!(top, 6, "the top face is two triangles, shaded 1.0");
    }

    #[test]
    fn touching_blocks_hide_the_faces_between_them() {
        let world = World::new(32, 16, |x, y, z| if (x, y, z) == (5, 5, 5) || (x, y, z) == (6, 5, 5) { Block::Dirt } else { Block::Air });
        assert_eq!(build_section(&world, 0, 0, 0).faces, 10, "12 faces less the 2 that touch");
    }

    #[test]
    fn a_neighbour_in_the_next_section_hides_the_face_across_the_seam() {
        // x = 15 is the last block of section 0, x = 16 the first of section 1.
        let world = World::new(32, 16, |x, y, z| if (x, y, z) == (15, 0, 0) || (x, y, z) == (16, 0, 0) { Block::Stone } else { Block::Air });
        assert_eq!(build_section(&world, 0, 0, 0).faces, 5);
        assert_eq!(build_section(&world, 1, 0, 0).faces, 5);
    }

    #[test]
    fn an_empty_section_has_no_faces() {
        let world = World::new(32, 16, |_, _, _| Block::Air);
        let mesh = build_section(&world, 1, 0, 1);
        assert_eq!(mesh.faces, 0);
        assert!(mesh.vertices.is_empty());
    }

    #[test]
    fn the_procedural_ground_is_grass_over_dirt_and_stone_with_air_above() {
        let world = World::procedural();
        let column = (10, 10);
        let top = (0..WORLD_Y).rev().find(|&y| world.block(column.0, y, column.1).is_solid()).expect("ground");
        assert_eq!(world.block(column.0, top, column.1), Block::Grass);
        assert_eq!(world.block(column.0, top - 1, column.1), Block::Dirt);
        assert_eq!(world.block(column.0, top - 4, column.1), Block::Stone);
        assert_eq!(world.block(column.0, WORLD_Y - 1, column.1), Block::Air);
    }

    #[test]
    fn the_procedural_ground_reaches_into_the_upper_section() {
        let world = World::procedural();
        let upper = (0..WORLD_XZ / SECTION)
            .flat_map(|sx| (0..WORLD_XZ / SECTION).map(move |sz| (sx, sz)))
            .filter(|&(sx, sz)| build_section(&world, sx, 1, sz).faces > 0)
            .count();
        assert!(upper > 0, "some columns are higher than 16 blocks");
    }
}
