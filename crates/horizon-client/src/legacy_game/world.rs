//! A deterministic, streamed block world for the `legacy-gl` single-player loop.
//!
//! It is deliberately independent of `wgpu`, `naga`, the 22k-line `game.rs` and
//! the modern client: the GL2 client can stream chunks, collide with them, edit
//! them and persist edits while the modern build is unchanged.

use std::collections::HashMap;

pub const CHUNK_SIZE: i32 = 16;
pub const WORLD_HEIGHT: i32 = 64;
pub const SEA_LEVEL: i32 = 26;
pub const ATLAS_TILES: u32 = 4;
pub const ATLAS_TILE_PIXELS: u32 = 16;
pub const ATLAS_EDGE: u32 = ATLAS_TILES * ATLAS_TILE_PIXELS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Block {
    Air,
    Grass,
    Dirt,
    Stone,
    Sand,
    Water,
    Wood,
    Leaves,
    Planks,
    Cobble,
}

impl Block {
    pub const ALL: [Self; 10] = [
        Self::Air,
        Self::Grass,
        Self::Dirt,
        Self::Stone,
        Self::Sand,
        Self::Water,
        Self::Wood,
        Self::Leaves,
        Self::Planks,
        Self::Cobble,
    ];

    pub const fn from_u8(value: u8) -> Self {
        match value % 10 {
            0 => Self::Air,
            1 => Self::Grass,
            2 => Self::Dirt,
            3 => Self::Stone,
            4 => Self::Sand,
            5 => Self::Water,
            6 => Self::Wood,
            7 => Self::Leaves,
            8 => Self::Planks,
            _ => Self::Cobble,
        }
    }

    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Air => 0,
            Self::Grass => 1,
            Self::Dirt => 2,
            Self::Stone => 3,
            Self::Sand => 4,
            Self::Water => 5,
            Self::Wood => 6,
            Self::Leaves => 7,
            Self::Planks => 8,
            Self::Cobble => 9,
        }
    }

    pub const fn solid(self) -> bool {
        !matches!(self, Self::Air | Self::Water)
    }

    pub const fn opaque(self) -> bool {
        !matches!(self, Self::Air | Self::Water | Self::Leaves)
    }

    pub const fn name_key(self) -> &'static str {
        match self {
            Self::Air => "horizon.legacyGl.game.block.air",
            Self::Grass => "horizon.legacyGl.game.block.grass",
            Self::Dirt => "horizon.legacyGl.game.block.dirt",
            Self::Stone => "horizon.legacyGl.game.block.stone",
            Self::Sand => "horizon.legacyGl.game.block.sand",
            Self::Water => "horizon.legacyGl.game.block.water",
            Self::Wood => "horizon.legacyGl.game.block.wood",
            Self::Leaves => "horizon.legacyGl.game.block.leaves",
            Self::Planks => "horizon.legacyGl.game.block.planks",
            Self::Cobble => "horizon.legacyGl.game.block.cobble",
        }
    }

    pub const fn tile(self) -> u32 {
        self.as_u8() as u32
    }

    pub const fn color(self) -> [u8; 4] {
        match self {
            Self::Air => [0, 0, 0, 0],
            Self::Grass => [104, 160, 72, 255],
            Self::Dirt => [134, 96, 67, 255],
            Self::Stone => [128, 128, 128, 255],
            Self::Sand => [220, 208, 160, 255],
            Self::Water => [48, 96, 200, 160],
            Self::Wood => [120, 88, 52, 255],
            Self::Leaves => [56, 128, 56, 220],
            Self::Planks => [176, 140, 88, 255],
            Self::Cobble => [104, 104, 104, 255],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Chunk {
    pub x: i32,
    pub z: i32,
    pub blocks: Vec<u8>,
}

impl Chunk {
    fn index(x: i32, y: i32, z: i32) -> usize {
        ((y * CHUNK_SIZE + z) * CHUNK_SIZE + x) as usize
    }

    fn new(x: i32, z: i32, seed: u64) -> Self {
        let mut blocks =
            vec![Block::Air.as_u8(); (CHUNK_SIZE * WORLD_HEIGHT * CHUNK_SIZE) as usize];
        for lz in 0..CHUNK_SIZE {
            for lx in 0..CHUNK_SIZE {
                let wx = x * CHUNK_SIZE + lx;
                let wz = z * CHUNK_SIZE + lz;
                let height = height_at(seed, wx, wz);
                for y in 0..WORLD_HEIGHT {
                    let block = if y == 0 || y < height - 4 {
                        Block::Stone
                    } else if y < height {
                        Block::Dirt
                    } else if y == height {
                        if y <= SEA_LEVEL {
                            Block::Sand
                        } else {
                            Block::Grass
                        }
                    } else if y <= SEA_LEVEL {
                        Block::Water
                    } else {
                        Block::Air
                    };
                    blocks[Self::index(lx, y, lz)] = block.as_u8();
                }
                if height > SEA_LEVEL && tree_at(seed, wx, wz) {
                    for dy in 1..=4 {
                        let y = height + dy;
                        if y < WORLD_HEIGHT {
                            blocks[Self::index(lx, y, lz)] = Block::Wood.as_u8();
                        }
                    }
                    for dy in 3..=5 {
                        for ox in -2..=2 {
                            for oz in -2..=2 {
                                if ox * ox + oz * oz > 4 {
                                    continue;
                                }
                                let px = lx + ox;
                                let pz = lz + oz;
                                let py = height + dy;
                                if px >= 0
                                    && px < CHUNK_SIZE
                                    && pz >= 0
                                    && pz < CHUNK_SIZE
                                    && py < WORLD_HEIGHT
                                {
                                    let at = Self::index(px, py, pz);
                                    if blocks[at] == Block::Air.as_u8() {
                                        blocks[at] = Block::Leaves.as_u8();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Self { x, z, blocks }
    }

    fn get_local(&self, x: i32, y: i32, z: i32) -> Block {
        if y < 0 {
            return Block::Stone;
        }
        if y >= WORLD_HEIGHT || x < 0 || x >= CHUNK_SIZE || z < 0 || z >= CHUNK_SIZE {
            return Block::Air;
        }
        Block::from_u8(self.blocks[Self::index(x, y, z)])
    }

    fn set_local(&mut self, x: i32, y: i32, z: i32, block: Block) {
        if (0..WORLD_HEIGHT).contains(&y)
            && (0..CHUNK_SIZE).contains(&x)
            && (0..CHUNK_SIZE).contains(&z)
        {
            self.blocks[Self::index(x, y, z)] = block.as_u8();
        }
    }
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn hash2(seed: u64, x: i32, z: i32) -> f32 {
    let value =
        splitmix64(seed ^ ((x as u64) << 32) ^ (z as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
    (value & 0x00FF_FFFF) as f32 / 16_777_215.0
}

fn smooth(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}

fn noise(seed: u64, x: f32, z: f32) -> f32 {
    let x0 = x.floor();
    let z0 = z.floor();
    let fx = smooth(x - x0);
    let fz = smooth(z - z0);
    let a = hash2(seed, x0 as i32, z0 as i32);
    let b = hash2(seed, x0 as i32 + 1, z0 as i32);
    let c = hash2(seed, x0 as i32, z0 as i32 + 1);
    let d = hash2(seed, x0 as i32 + 1, z0 as i32 + 1);
    (a * (1.0 - fx) + b * fx) * (1.0 - fz) + (c * (1.0 - fx) + d * fx) * fz
}

pub fn height_at(seed: u64, x: i32, z: i32) -> i32 {
    let broad = noise(seed, x as f32 / 48.0, z as f32 / 48.0);
    let detail = noise(seed ^ 0xA5A5, x as f32 / 12.0, z as f32 / 12.0);
    (SEA_LEVEL as f32 + (broad - 0.5) * 18.0 + (detail - 0.5) * 5.0)
        .round()
        .clamp(4.0, (WORLD_HEIGHT - 12) as f32) as i32
}

fn tree_at(seed: u64, x: i32, z: i32) -> bool {
    hash2(seed ^ 0x5EED, x / 3, z / 3) > 0.985 && (x.rem_euclid(3) == 1) && (z.rem_euclid(3) == 1)
}

#[derive(Clone, Debug)]
pub struct World {
    pub seed: u64,
    pub chunks: HashMap<(i32, i32), Chunk>,
    pub edits: HashMap<(i32, i32, i32), Block>,
    pub day_time: u32,
}

impl World {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            chunks: HashMap::new(),
            edits: HashMap::new(),
            day_time: 1000,
        }
    }

    pub fn chunk_pos(x: i32, z: i32) -> (i32, i32) {
        (x.div_euclid(CHUNK_SIZE), z.div_euclid(CHUNK_SIZE))
    }

    pub fn ensure_chunk(&mut self, cx: i32, cz: i32) {
        let key = (cx, cz);
        if self.chunks.contains_key(&key) {
            return;
        }
        let mut chunk = Chunk::new(cx, cz, self.seed);
        for (&(x, y, z), &block) in &self.edits {
            let (ex, ez) = Self::chunk_pos(x, z);
            if (ex, ez) == key {
                chunk.set_local(x.rem_euclid(CHUNK_SIZE), y, z.rem_euclid(CHUNK_SIZE), block);
            }
        }
        self.chunks.insert(key, chunk);
    }

    pub fn get(&mut self, x: i32, y: i32, z: i32) -> Block {
        if y < 0 {
            return Block::Stone;
        }
        if y >= WORLD_HEIGHT {
            return Block::Air;
        }
        let (cx, cz) = Self::chunk_pos(x, z);
        self.ensure_chunk(cx, cz);
        self.chunks[&(cx, cz)].get_local(x.rem_euclid(CHUNK_SIZE), y, z.rem_euclid(CHUNK_SIZE))
    }

    pub fn set(&mut self, x: i32, y: i32, z: i32, block: Block) {
        if !(0..WORLD_HEIGHT).contains(&y) {
            return;
        }
        let (cx, cz) = Self::chunk_pos(x, z);
        self.ensure_chunk(cx, cz);
        self.chunks.get_mut(&(cx, cz)).unwrap().set_local(
            x.rem_euclid(CHUNK_SIZE),
            y,
            z.rem_euclid(CHUNK_SIZE),
            block,
        );
        self.edits.insert((x, y, z), block);
    }

    pub fn apply_edits(&mut self, edits: &[(i32, i32, i32, u8)]) {
        for &(x, y, z, value) in edits {
            self.set(x, y, z, Block::from_u8(value));
        }
    }

    pub fn saved_edits(&self) -> Vec<(i32, i32, i32, u8)> {
        let mut out: Vec<_> = self
            .edits
            .iter()
            .map(|(&(x, y, z), &block)| (x, y, z, block.as_u8()))
            .collect();
        out.sort_unstable();
        out
    }

    /// Keep the chunk window around the player and return its sorted keys.
    pub fn stream_around(&mut self, x: f32, z: f32, distance: i32) -> Vec<(i32, i32)> {
        let (cx, cz) = Self::chunk_pos(x.floor() as i32, z.floor() as i32);
        let mut keep = Vec::new();
        for dz in -distance..=distance {
            for dx in -distance..=distance {
                if dx * dx + dz * dz > (distance + 1) * (distance + 1) {
                    continue;
                }
                let key = (cx + dx, cz + dz);
                self.ensure_chunk(key.0, key.1);
                keep.push(key);
            }
        }
        self.chunks.retain(|key, _| keep.contains(key));
        keep.sort_unstable();
        keep
    }

    fn tile_uv(tile: u32, u: f32, v: f32) -> [f32; 2] {
        let step = 1.0 / ATLAS_TILES as f32;
        let inset = 0.5 / ATLAS_EDGE as f32;
        let col = tile % ATLAS_TILES;
        let row = tile / ATLAS_TILES;
        [
            col as f32 * step + inset + u * (step - inset * 2.0),
            row as f32 * step + inset + v * (step - inset * 2.0),
        ]
    }

    /// Build one interleaved terrain VBO for a chunk: position3, uv2, color4,
    /// lightmap-uv2 (11 floats per vertex, matching `terrain_*.vert`).
    pub fn mesh_chunk(&mut self, cx: i32, cz: i32) -> Vec<f32> {
        self.ensure_chunk(cx, cz);
        let origin_x = cx * CHUNK_SIZE;
        let origin_z = cz * CHUNK_SIZE;
        let mut out = Vec::new();
        const FACES: [([i32; 3], [[f32; 3]; 4], f32); 6] = [
            (
                [0, 1, 0],
                [[0., 1., 0.], [1., 1., 0.], [1., 1., 1.], [0., 1., 1.]],
                1.0,
            ),
            (
                [0, -1, 0],
                [[0., 0., 1.], [1., 0., 1.], [1., 0., 0.], [0., 0., 0.]],
                0.55,
            ),
            (
                [1, 0, 0],
                [[1., 0., 1.], [1., 1., 1.], [1., 1., 0.], [1., 0., 0.]],
                0.80,
            ),
            (
                [-1, 0, 0],
                [[0., 0., 0.], [0., 1., 0.], [0., 1., 1.], [0., 0., 1.]],
                0.72,
            ),
            (
                [0, 0, 1],
                [[1., 0., 1.], [0., 0., 1.], [0., 1., 1.], [1., 1., 1.]],
                0.86,
            ),
            (
                [0, 0, -1],
                [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]],
                0.78,
            ),
        ];
        const UVS: [[f32; 2]; 4] = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
        const ORDER: [usize; 6] = [0, 1, 2, 0, 2, 3];
        for ly in 0..WORLD_HEIGHT {
            for lz in 0..CHUNK_SIZE {
                for lx in 0..CHUNK_SIZE {
                    let block = self.chunks[&(cx, cz)].get_local(lx, ly, lz);
                    if block == Block::Air {
                        continue;
                    }
                    let wx = origin_x + lx;
                    let wz = origin_z + lz;
                    let base = block.color();
                    for (normal, corners, shade) in FACES {
                        let nx = wx + normal[0];
                        let ny = ly + normal[1];
                        let nz = wz + normal[2];
                        let neighbour = self.get(nx, ny, nz);
                        if neighbour.opaque() && neighbour != Block::Leaves {
                            continue;
                        }
                        if block == Block::Water && neighbour == Block::Water {
                            continue;
                        }
                        let tile = block.tile();
                        let light_u = (ly as f32 / WORLD_HEIGHT as f32).clamp(0.05, 0.95);
                        let light_v = 0.82;
                        let shade_color = [
                            (base[0] as f32 * shade) as u8,
                            (base[1] as f32 * shade) as u8,
                            (base[2] as f32 * shade) as u8,
                            base[3],
                        ];
                        let rgba = [
                            shade_color[0] as f32 / 255.0,
                            shade_color[1] as f32 / 255.0,
                            shade_color[2] as f32 / 255.0,
                            shade_color[3] as f32 / 255.0,
                        ];
                        for at in ORDER {
                            let p = corners[at];
                            out.extend_from_slice(&[
                                lx as f32 + p[0],
                                ly as f32 + p[1],
                                lz as f32 + p[2],
                            ]);
                            let uv = Self::tile_uv(tile, UVS[at][0], UVS[at][1]);
                            out.extend_from_slice(&uv);
                            out.extend_from_slice(&rgba);
                            out.extend_from_slice(&[light_u, light_v]);
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic_and_bounded() {
        let seed = 0x51DE;
        assert_eq!(height_at(seed, 10, -20), height_at(seed, 10, -20));
        assert!((4..WORLD_HEIGHT - 12).contains(&height_at(seed, 10, -20)));
        let mut a = World::new(seed);
        let mut b = World::new(seed);
        assert_eq!(a.get(3, 10, 3), b.get(3, 10, 3));
        assert_eq!(a.mesh_chunk(0, 0), b.mesh_chunk(0, 0));
    }

    #[test]
    fn edits_are_applied_and_streamed() {
        let mut world = World::new(7);
        world.set(3, 30, 3, Block::Planks);
        assert_eq!(world.get(3, 30, 3), Block::Planks);
        let keys = world.stream_around(0.0, 0.0, 1);
        assert!(!keys.is_empty());
        assert!(keys.contains(&(0, 0)));
        assert_eq!(world.saved_edits(), vec![(3, 30, 3, Block::Planks.as_u8())]);
    }

    #[test]
    fn mesh_vertices_match_the_terrain_layout() {
        let mut world = World::new(11);
        let mesh = world.mesh_chunk(0, 0);
        assert!(!mesh.is_empty());
        assert_eq!(mesh.len() % 66, 0, "six vertices of eleven floats per face");
        assert!(mesh.len() / 66 <= (CHUNK_SIZE * WORLD_HEIGHT * CHUNK_SIZE * 6) as usize);
    }
}
