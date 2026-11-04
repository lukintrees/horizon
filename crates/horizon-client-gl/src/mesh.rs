//! CPU-side terrain vertex writing: the mesher's `TerrainVertex` to this path's 32-byte vertex.
//!
//! **This module is pure CPU and does not touch GL.** It produces the bytes
//! [`VertexFormat::legacy`](crate::gl::buffer::VertexFormat::legacy) describes; uploading them
//! into a [`Buffer`](crate::gl::buffer::Buffer) is the caller's job, so the conversion can be
//! tested here without a driver and the target machine (which has no Rust toolchain here) is
//! not in the loop for a layout question.
//!
//! The translation, field by field, against `horizon-client-render`'s `mesher.rs`:
//!
//! ```text
//! TerrainVertex (24 bytes)        this path's vertex (32 bytes)
//! pos: [f32; 3]                   a_position   f32 x3        copied
//! uv:  [u16; 2]  / UV_STEPS       a_texcoord   f32 x2        widened to 0..1
//! layer: u16                      a_extra      f32           the atlas, 0 or 1
//! light: u16 (sky | block << 8)   a_light      u8 x2         two bytes, normalized
//! color: u32 (RGBA8)              a_color      u8 x4         copied, alpha = emission code
//! ```
//!
//! Three things about that table are load-bearing:
//!
//!  * **The UV is float here, but `TerrainVertex` is `Copy`.** The original UVs stay in the
//!    mesher's vertex (it keeps them for its own arithmetic), so this path *could* take them
//!    from there rather than dividing the `u16` back. It divides instead, because a vertex
//!    buffer is built from the `u16` the GPU will page through: reading the float would make
//!    the two paths disagree if the mesher ever rounds differently, and `UV_STEPS` is a power
//!    of two so the division is exact. The `u16` is the contract, one `u16` step being 1/32768
//!    of a sprite.
//!  * **`layer` is not a layer any more.** `texture.rs` replaced the sprite array with a UV
//!    inside an atlas, so the vertex carries only which atlas. **The mapping is the atlas
//!    builder's, not a formula**: how many sprites an atlas holds depends on their sizes
//!    (a 2048^2 atlas is 16 384 16x16 sprites, but a single 2048^2 sky texture is one), so a
//!    `layer / N` here would be a guess that is wrong for the first pack that is not 16x16.
//!    [`AtlasPlacement`] is where the builder's decision arrives. A sprite the builder never
//!    placed is an error, not a clamp: silently drawing something else is how a mod's block
//!    renders with another block's texture.
//!  * **The colour's alpha byte is not alpha.** `mesher.rs` puts the HDR emission code in
//!    bits 24..28 (`EMISSION_BITS`), and the fragment stage reads transparency from the
//!    lightmap instead. Copying the word verbatim keeps the encoding in one place.
//!
//! One more thing the *format* cannot carry, stated here rather than discovered later:
//!
//!  * **The emission code arrives but nothing reads it.** There is no tenth attribute to hold
//!    it, so a face that glows in HDR does not glow on this backend until a HDR path exists
//!    here. What matters is that it is not mistaken for alpha: the fragment stage takes
//!    transparency from the lightmap, as the WGSL does.

use crate::gl::texture::ATLAS_COUNT;

/// Fixed-point steps per unit of the mesher's UVs: `horizon-client-render`'s `UV_STEPS`.
pub const UV_STEPS: f32 = 32768.0;

/// Fixed-point steps per block of distant-terrain UVs: the mesher's `TILED_UV_STEPS`. A
/// merged face repeats its sprite once per block, so its UV arrives in blocks, not in 0..1.
pub const TILED_UV_STEPS: f32 = 32.0;

/// The flag the mesher sets above the sprite index (`mesher.rs`'s `OPAQUE_LAYER`): opaque
/// leaves and quads over opaque texels. It is not part of the sprite index.
pub const OPAQUE_LAYER: u16 = 1 << 15;

/// Bytes per vertex: whatever the format says, so the writer and the binding cannot drift.
pub const VERTEX_BYTES: usize = 32;

/// One terrain vertex, in the exact order the GPU reads it.
///
/// `#[repr(C)]` and nothing padded by hand: the offsets are the ones
/// [`VertexFormat::legacy`] declares (0, 12, 20, 24, 28) and
/// [`tests::the_struct_matches_the_format`] checks that against the format itself.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    /// RGBA8: tint * face shade * ambient occlusion. The top byte is the emission code.
    pub color: [u8; 4],
    /// Sky, block: the mesher's 0..15 levels, normalized here so the shader gets 0..1.
    pub light: [u8; 2],
    /// Which of the two atlases: 0 or 1.
    pub atlas: f32,
}

impl GpuVertex {
    /// The bytes of one vertex, in the format's order.
    pub fn to_bytes(&self) -> [u8; VERTEX_BYTES] {
        let mut out = [0u8; VERTEX_BYTES];
        let mut at = 0;
        for v in self.pos {
            out[at..at + 4].copy_from_slice(&v.to_le_bytes());
            at += 4;
        }
        for v in self.uv {
            out[at..at + 4].copy_from_slice(&v.to_le_bytes());
            at += 4;
        }
        out[at..at + 4].copy_from_slice(&self.color);
        at += 4;
        out[at..at + 2].copy_from_slice(&self.light);
        at += 2;
        out[at..at + 4].copy_from_slice(&self.atlas.to_le_bytes());
        out
    }
}

/// The mesher's vertex, as this crate sees it.
///
/// A mirror rather than a re-export: `horizon-client-gl` does not depend on
/// `horizon-client-render` (that crate owns wgpu, and this one exists to run where wgpu
/// cannot), and a `#[repr(C)]` copy of three numbers is a smaller coupling than a dependency
/// that drags the whole wgpu stack in. The field order and sizes are `TerrainVertex`'s.
#[repr(C)]
// No `Eq`: `pos` is `f32`, and `Eq` on a struct holding one does not compile (`f32` is only
// `PartialEq`). The mesher's own `TerrainVertex` is the same shape for the same reason.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TerrainVertex {
    pub pos: [f32; 3],
    /// `round(uv * UV_STEPS)`, for a `uv` in 0..1.
    pub uv: [u16; 2],
    /// Sprite index, with [`OPAQUE_LAYER`] set for opaque leaves.
    pub layer: u16,
    /// Sky in the low byte, block in the high one, 0..15 each.
    pub light: u16,
    /// RGBA8 with the emission code in the top byte.
    pub color: u32,
}

impl TerrainVertex {
    /// The sky level, 0..15.
    pub fn sky(&self) -> u8 {
        (self.light & 0xFF) as u8
    }

    /// The block level, 0..15.
    pub fn block(&self) -> u8 {
        (self.light >> 8) as u8
    }

    /// The sprite index without the flag bit.
    pub fn sprite(&self) -> u16 {
        self.layer & !OPAQUE_LAYER
    }

    /// Whether this vertex draws opaque leaves: the mesher's flag, and the fragment stage's
    /// `discard` check in `terrain.wgsl` (a layer with this bit never discards).
    pub fn opaque(&self) -> bool {
        self.layer & OPAQUE_LAYER != 0
    }
}

/// Where the atlas builder put each sprite: `atlas[sprite]` is the atlas that sprite samples,
/// or [`UNPLACED`] for one the builder never wrote.
///
/// The builder is `horizon-client-render`'s (`block_sprites.rs`): it knows the sprite sizes and
/// how they fit, and it is the only thing that should decide which atlas a sprite is in. This
/// is that decision, carried over in the form the vertex needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AtlasPlacement {
    atlas: Vec<u8>,
}

/// What [`AtlasPlacement`] holds for a sprite that was never placed. Not a valid atlas index.
pub const UNPLACED: u8 = u8::MAX;

impl AtlasPlacement {
    /// A table with room for `sprites` indices, none of them placed yet.
    pub fn with_capacity(sprites: usize) -> Self {
        Self {
            atlas: vec![UNPLACED; sprites],
        }
    }

    /// Records that `sprite` samples `atlas`. An atlas index the format does not have is
    /// refused here rather than written into vertices and read as a wrong half of the texture.
    pub fn place(&mut self, sprite: u16, atlas: u8) -> Result<(), MeshError> {
        if usize::from(atlas) >= ATLAS_COUNT as usize {
            return Err(MeshError::NoSuchAtlas { atlas });
        }
        let slot = usize::from(sprite);
        if slot >= self.atlas.len() {
            // Grow rather than refuse: the builder's sprite table is the authority on how many
            // there are, and a caller that writes them in order should not have to size this.
            self.atlas.resize(slot + 1, UNPLACED);
        }
        self.atlas[slot] = atlas;
        Ok(())
    }

    /// The atlas a sprite samples. `None` for a sprite the builder never placed.
    pub fn atlas_of(&self, sprite: u16) -> Option<u8> {
        match self.atlas.get(usize::from(sprite)) {
            Some(&atlas) if atlas != UNPLACED => Some(atlas),
            _ => None,
        }
    }

    /// How many sprites the table has an entry for.
    pub fn len(&self) -> usize {
        self.atlas.len()
    }

    /// Whether no sprite has been placed.
    pub fn is_empty(&self) -> bool {
        self.atlas.iter().all(|&atlas| atlas == UNPLACED)
    }
}

/// Why a vertex could not be converted. A separate type from `crate::Error` because this
/// module has no GL context to fail with, and because a caller wants to name the sprite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshError {
    /// The atlas builder never placed this sprite. Either it is a sprite the pack does not
    /// have, or the atlas builder and the mesher disagree about the sprite table.
    UnplacedSprite { sprite: u16 },
    /// A caller named an atlas the path does not have (there are `ATLAS_COUNT` of them).
    NoSuchAtlas { atlas: u8 },
}

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MeshError::UnplacedSprite { sprite } => write!(
                f,
                "sprite {sprite} is in no atlas: the atlas builder never placed it"
            ),
            MeshError::NoSuchAtlas { atlas } => write!(
                f,
                "atlas {atlas} does not exist: this path has {ATLAS_COUNT}"
            ),
        }
    }
}

impl std::error::Error for MeshError {}

/// Whether a vertex's UV arrived in blocks (`TerrainVertex::tiled`) rather than in 0..1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Uv {
    /// `round(uv * UV_STEPS)` in the `u16`, spanning 0..1.
    Sprite,
    /// `round(uv * TILED_UV_STEPS)`: a merged face, one sprite repeat per block.
    Tiled,
}

impl Uv {
    /// The steps per unit this UV was written with.
    pub fn steps(self) -> f32 {
        match self {
            Uv::Sprite => UV_STEPS,
            Uv::Tiled => TILED_UV_STEPS,
        }
    }
}

/// Converts one mesher vertex to the GPU's layout. Errors only on a sprite the atlas builder
/// never placed, which is a load-time condition the caller reports with the pack's name.
pub fn to_gpu(
    vertex: &TerrainVertex,
    uv: Uv,
    placement: &AtlasPlacement,
) -> Result<GpuVertex, MeshError> {
    let atlas = placement
        .atlas_of(vertex.sprite())
        .ok_or(MeshError::UnplacedSprite {
            sprite: vertex.sprite(),
        })?;
    let steps = uv.steps();
    Ok(GpuVertex {
        pos: vertex.pos,
        uv: [
            f32::from(vertex.uv[0]) / steps,
            f32::from(vertex.uv[1]) / steps,
        ],
        // Little-endian on every target this path runs on (x86 and ARM are both), and the
        // byte order a normalized RGBA8 attribute reads in: R, G, B, A from the low byte.
        color: [
            vertex.color as u8,
            (vertex.color >> 8) as u8,
            (vertex.color >> 16) as u8,
            (vertex.color >> 24) as u8,
        ],
        // The mesher's packed `sky | block << 8`, split into two normalized bytes. The
        // shader scales by 15 (the lightmap is 16x16), so a level of 15 is 255/255 = 1.0.
        light: [
            scale_light(vertex.sky()),
            scale_light(vertex.block()),
        ],
        atlas: f32::from(atlas),
    })
}

/// A 0..15 light level as the normalized byte the attribute reads: 15 -> 255.
fn scale_light(level: u8) -> u8 {
    // 15 * 17 = 255 and 1 * 17 = 17, so level * 17 is exact for every level and needs no
    // rounding. (level as u32 * 255 / 15 would round the same way; this is one multiply.)
    level.min(15) * 17
}

/// Converts a run of vertices into the bytes of one buffer upload.
///
/// One allocation for the whole run, not one per vertex: a section is a few thousand
/// vertices, and on the target's single core the per-vertex `Vec` is the cost that shows up in
/// a frame. On the first layer the atlases cannot hold this stops and names it.
pub fn to_bytes(
    vertices: &[TerrainVertex],
    uv: Uv,
    placement: &AtlasPlacement,
) -> Result<Vec<u8>, MeshError> {
    let mut out = Vec::with_capacity(vertices.len() * VERTEX_BYTES);
    for vertex in vertices {
        out.extend_from_slice(&to_gpu(vertex, uv, placement)?.to_bytes());
    }
    Ok(out)
}

/// A sprite's UVs inside one atlas, as the mesher writes them, for a rectangle the atlas
/// builder placed. The layer-to-`SpriteRect` half of `block_sprites.rs` belongs to the atlas
/// builder; this is only the arithmetic both sides have to agree on.
pub fn sprite_uv(rect: SpriteRect, corner: [f32; 2]) -> [f32; 2] {
    [
        rect.u0 + (rect.u1 - rect.u0) * corner[0],
        rect.v0 + (rect.v1 - rect.v0) * corner[1],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl::buffer::{AttributeType, VertexFormat};
    use crate::gl::texture::{SpriteRect, ATLAS_TEXELS};

    /// A placement with the first `n` sprites in atlas 0, as a 16x16-only pack would have.
    fn placed(n: u16) -> AtlasPlacement {
        let mut placement = AtlasPlacement::with_capacity(usize::from(n));
        for sprite in 0..n {
            placement.place(sprite, 0).unwrap();
        }
        placement
    }

    /// The structural check the whole module exists for: the struct's layout is the format's
    /// layout, byte for byte. `size_of` catches a padded struct, and the offsets catch a field
    /// in the wrong place.
    #[test]
    fn the_struct_matches_the_format() {
        assert_eq!(std::mem::size_of::<GpuVertex>(), VERTEX_BYTES);
        let format = VertexFormat::legacy();
        assert_eq!(format.stride as usize, VERTEX_BYTES);
        let find = |location: u32| {
            format
                .attributes
                .iter()
                .find(|a| a.location == location)
                .copied()
                .expect("attribute")
        };
        use crate::gl::program::attribs;
        assert_eq!(find(attribs::POSITION).offset, 0);
        assert_eq!(find(attribs::TEXCOORD).offset, 12);
        assert_eq!(find(attribs::COLOR).offset, 20);
        assert_eq!(find(attribs::LIGHT).offset, 24);
        assert_eq!(find(attribs::EXTRA).offset, 28);
        // And the components agree with the struct's field sizes.
        assert_eq!(find(attribs::POSITION).components, 3);
        assert_eq!(find(attribs::TEXCOORD).components, 2);
        assert_eq!(find(attribs::COLOR).components, 4);
        assert_eq!(find(attribs::LIGHT).components, 2);
        assert_eq!(find(attribs::EXTRA).components, 1);
        assert_eq!(find(attribs::COLOR).ty, AttributeType::UnsignedByte);
        assert_eq!(find(attribs::LIGHT).ty, AttributeType::UnsignedByte);
        assert!(find(attribs::COLOR).normalized);
        assert!(find(attribs::LIGHT).normalized);
    }

    /// `to_bytes` writes every field where the format says it is. This is the test that would
    /// have caught the `a_extra` stride bug: it reads the bytes back at the declared offsets.
    #[test]
    fn the_bytes_land_on_the_declared_offsets() {
        let vertex = TerrainVertex {
            pos: [1.0, 2.0, 3.0],
            uv: [UV_STEPS as u16, 0],
            layer: 1,
            light: 15 | 4 << 8,
            color: 0x0A0B_0C0D,
        };
        let bytes = to_gpu(&vertex, Uv::Sprite, &placed(2)).unwrap().to_bytes();
        assert_eq!(bytes.len(), VERTEX_BYTES);
        assert_eq!(f32::from_le_bytes(bytes[0..4].try_into().unwrap()), 1.0);
        assert_eq!(f32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2.0);
        assert_eq!(f32::from_le_bytes(bytes[8..12].try_into().unwrap()), 3.0);
        assert_eq!(f32::from_le_bytes(bytes[12..16].try_into().unwrap()), 1.0);
        assert_eq!(f32::from_le_bytes(bytes[16..20].try_into().unwrap()), 0.0);
        assert_eq!(&bytes[20..24], &[0x0D, 0x0C, 0x0B, 0x0A]);
        assert_eq!(bytes[24], 255, "sky 15 is the maximum");
        assert_eq!(bytes[25], 4 * 17, "block 4 out of 15");
        assert_eq!(f32::from_le_bytes(bytes[28..32].try_into().unwrap()), 1.0);
    }

    #[test]
    fn the_uv_is_exactly_what_the_mesher_rounded() {
        for step in [0u16, 1, 100, 16_384, 32_767, 32_768] {
            let vertex = TerrainVertex {
                uv: [step, step],
                ..Default::default()
            };
            // UV_STEPS is a power of two, so this division is exact in f32.
            assert_eq!(
                to_gpu(&vertex, Uv::Sprite, &placed(1)).unwrap().uv,
                [f32::from(step) / UV_STEPS; 2]
            );
        }
        // A tiled vertex's UV is in blocks, not in 0..1: 32 steps is one block.
        let tiled = TerrainVertex {
            uv: [32, 16],
            ..Default::default()
        };
        assert_eq!(to_gpu(&tiled, Uv::Tiled, &placed(1)).unwrap().uv, [1.0, 0.5]);
    }

    #[test]
    fn the_light_bytes_are_exact_for_every_level() {
        for level in 0..=15u8 {
            // 15 must be 255: anything less and the lightmap lookup lands on the wrong texel.
            assert_eq!(scale_light(level), level * 17);
        }
        assert_eq!(scale_light(15), 255);
        assert_eq!(scale_light(0), 0);
        // A field the mesher never writes (a level above 15) is clamped rather than wrapping
        // into the next byte's slot.
        assert_eq!(scale_light(200), 255);
    }

    #[test]
    fn the_atlas_comes_from_the_placement_not_from_a_formula() {
        // The bug a formula would have: a 2048^2 atlas holds 16 384 16x16 sprites, so
        // `sprite / 2048` would put sprite 5000 in atlas 2 (which does not exist) and sprite
        // 100 in atlas 0 by luck. The builder's answer is the only one that survives a pack
        // whose sprites are not all 16x16, and 10000 sprites in one atlas is legal.
        let mut placement = AtlasPlacement::with_capacity(10_000);
        for sprite in 0..10_000u16 {
            placement.place(sprite, 0).unwrap();
        }
        assert_eq!(placement.atlas_of(9_999), Some(0));
        // And a caller can put a sprite anywhere the builder decided, including out of order.
        let mut uneven = AtlasPlacement::default();
        uneven.place(7, 1).unwrap();
        uneven.place(2, 0).unwrap();
        assert_eq!(uneven.atlas_of(7), Some(1));
        assert_eq!(uneven.atlas_of(2), Some(0));
        assert_eq!(uneven.atlas_of(3), None);
        let vertex = TerrainVertex {
            layer: 7,
            ..Default::default()
        };
        assert_eq!(to_gpu(&vertex, Uv::Sprite, &uneven).unwrap().atlas, 1.0);
    }

    #[test]
    fn an_unplaced_sprite_is_an_error_not_a_wrap() {
        let placement = placed(4);
        // A sprite past the table: the mesher references something the builder does not have.
        let past = TerrainVertex {
            layer: 4,
            ..Default::default()
        };
        assert_eq!(
            to_gpu(&past, Uv::Sprite, &placement),
            Err(MeshError::UnplacedSprite { sprite: 4 })
        );
        // The flag bit is not part of the sprite index, so an opaque vertex with a placed
        // sprite is fine: `sprite()` masks it off.
        let opaque = TerrainVertex {
            layer: 3 | OPAQUE_LAYER,
            ..Default::default()
        };
        assert!(to_gpu(&opaque, Uv::Sprite, &placement).is_ok());
        // A hole in the middle of the table is unplaced too, and the message names the sprite.
        let mut holed = placed(4);
        holed.atlas[2] = UNPLACED;
        assert_eq!(
            to_gpu(
                &TerrainVertex {
                    layer: 2,
                    ..Default::default()
                },
                Uv::Sprite,
                &holed
            ),
            Err(MeshError::UnplacedSprite { sprite: 2 })
        );
        let text = MeshError::UnplacedSprite { sprite: 2 }.to_string();
        assert!(text.contains("sprite 2"), "{text}");
    }

    #[test]
    fn an_atlas_index_the_path_does_not_have_is_refused_at_placement() {
        let mut placement = AtlasPlacement::default();
        assert_eq!(placement.place(0, 2), Err(MeshError::NoSuchAtlas { atlas: 2 }));
        assert_eq!(placement.place(0, UNPLACED), Err(MeshError::NoSuchAtlas { atlas: UNPLACED }));
        assert_eq!(placement.place(0, ATLAS_COUNT as u8 - 1), Ok(()));
        assert!(placement.is_empty(), "a refused placement writes nothing");
        assert!(MeshError::NoSuchAtlas { atlas: 5 }.to_string().contains('2'));
    }

    #[test]
    fn a_placement_grows_for_sprites_written_in_order() {
        let mut placement = AtlasPlacement::default();
        assert!(placement.atlas.is_empty());
        placement.place(40, 1).unwrap();
        assert_eq!(placement.len(), 41);
        // Everything skipped is a hole, not atlas 0.
        assert_eq!(placement.atlas_of(39), None);
        assert_eq!(placement.atlas_of(40), Some(1));
        assert!(!placement.is_empty());
    }

    #[test]
    fn the_colour_word_is_copied_not_reinterpreted() {
        // The alpha byte is the emission code (mesher.rs's EMISSION_BITS), so it must arrive
        // in the GPU's alpha untouched: a shader that read it as alpha would make a glowing
        // face translucent.
        for word in [0u32, 0xFFFF_FFFF, 0x0F00_0000, 0x1234_5678] {
            let vertex = TerrainVertex {
                color: word,
                ..Default::default()
            };
            let gpu = to_gpu(&vertex, Uv::Sprite, &placed(1)).unwrap();
            assert_eq!(gpu.color, word.to_le_bytes());
        }
    }

    #[test]
    fn a_run_is_vertex_after_vertex_without_gaps() {
        let run: Vec<TerrainVertex> = (0..8)
            .map(|i| TerrainVertex {
                pos: [i as f32, 0.0, 0.0],
                uv: [0, 0],
                layer: i as u16,
                light: 0x0F00 | i as u16,
                color: 0xFF00_0000,
            })
            .collect();
        let bytes = to_bytes(&run, Uv::Sprite, &placed(8)).unwrap();
        assert_eq!(bytes.len(), run.len() * VERTEX_BYTES);
        for (i, vertex) in run.iter().enumerate() {
            let at = i * VERTEX_BYTES;
            assert_eq!(
                f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
                i as f32
            );
        }
    }

    #[test]
    fn an_unplaced_sprite_stops_the_run_at_the_vertex_that_has_it() {
        let placement = placed(2);
        let run = [
            TerrainVertex::default(),
            TerrainVertex {
                layer: 9,
                ..Default::default()
            },
        ];
        assert_eq!(
            to_bytes(&run, Uv::Sprite, &placement),
            Err(MeshError::UnplacedSprite { sprite: 9 })
        );
    }

    /// The level the mesher packs is `sky | block << 8`; reading it back the other way round
    /// would swap the two lightmap axes and look like a broken lightmap, not a bug.
    #[test]
    fn the_light_word_reads_sky_low_and_block_high() {
        let vertex = TerrainVertex {
            light: 0x0305,
            ..Default::default()
        };
        assert_eq!(vertex.sky(), 5);
        assert_eq!(vertex.block(), 3);
        let vertex = TerrainVertex {
            light: 0x0F0C,
            ..Default::default()
        };
        assert_eq!((vertex.sky(), vertex.block()), (12, 15));
    }

    #[test]
    fn a_sprite_uv_interpolates_inside_its_rectangle() {
        let rect = SpriteRect::from_texels(0, 0, 16, 16, ATLAS_TEXELS);
        assert_eq!(sprite_uv(rect, [0.0, 0.0]), [0.0, 0.0]);
        let size = 16.0 / ATLAS_TEXELS as f32;
        assert_eq!(sprite_uv(rect, [1.0, 1.0]), [size, size]);
        assert_eq!(sprite_uv(rect, [0.5, 0.25]), [size * 0.5, size * 0.25]);
    }
}
