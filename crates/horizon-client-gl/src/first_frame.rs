//! The first frame: a clear, then one quad of terrain, built from mesher vertices.
//!
//! This module exists to be the smallest thing that proves the whole path end to end:
//! `mesh.rs` turns `TerrainVertex`s into this side's vertex bytes, the bytes have a length and
//! a stride, and `draw.rs` describes the pass that will consume them. There is no GL call here
//! and no window: what it returns is a [`DrawList`] and an upload, which a platform layer binds
//! a context to and executes.
//!
//! Why one quad and not a section: everything that can be wrong with the first frame is wrong
//! at a smaller size too — the vertex layout, the UV scale, the atlas choice, the light bytes,
//! the origin uniform, the winding and the clear — and a quad makes each of them a line in a
//! test instead of a picture someone has to look at. The second frame's job is a section; the
//! first frame's job is to be *known* correct.
//!
//! The quad is the top face of a block at the world origin, in a section whose origin is the
//! origin, so a camera looking down at y = 1 sees a 16x16 repeat of one sprite. Its winding is
//! counter-clockwise seen from above, which is what `Cull::Back` expects: getting that backwards
//! is the classic "the world is invisible from outside" bug and it belongs in a test.

use crate::draw::{
    Blend, Clear, Cull, DepthState, Draw, DrawList, Geometry, State, Target, Uniforms, UniformId,
    Value, Viewport,
};
use crate::gl::buffer::VertexFormat;
use crate::mesh::{self, AtlasPlacement, TerrainVertex, Uv, OPAQUE_LAYER};
use crate::multi_draw::MultiDrawElements;

/// The pipeline id the first frame's terrain draw uses. The renderer's table maps it to a
/// program; here it only has to be stable.
pub const TERRAIN_PIPELINE: u32 = 1;

/// The vertex buffer the quad is uploaded into.
pub const QUAD_VERTICES: u32 = 1;

/// The index buffer the quad's six indices live in.
pub const QUAD_INDICES: u32 = 2;

/// The program's uniforms, by the names `terrain.vert`/`terrain.frag` declare. A caller
/// resolves each to its [`UniformId`] once, after the link, and reuses it every frame.
#[derive(Clone, Copy, Debug)]
pub struct TerrainUniforms {
    pub view_proj: UniformId,
    pub chunk_origin: UniformId,
    pub pass: UniformId,
    pub fog_color: UniformId,
    pub fog: UniformId,
}

/// What the platform layer uploads: one vertex buffer and one index buffer, with the sizes and
/// the stride the format declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upload {
    pub vertices: Vec<u8>,
    pub indices: Vec<u16>,
    pub stride: u32,
}

/// The four corners of the block's top face, in the mesher's own encoding.
///
/// `uv` is `round(uv * UV_STEPS)` as `TerrainVertex::new` writes it; the corners go clockwise in
/// texture order and counter-clockwise in XZ, which is the winding `Cull::Back` keeps. The light
/// is full sky and no block light, the colour white (no tint, no face shade, no occlusion), and
/// the layer is sprite 0 with no opaque flag.
pub fn quad_vertices() -> [TerrainVertex; 4] {
    let sky = 15;
    let light = sky | 0 << 8;
    let white = 0xFFFF_FFFF;
    let corner = |x: f32, z: f32, u: f32, v: f32| TerrainVertex {
        pos: [x, 1.0, z],
        uv: [
            (u * mesh::UV_STEPS) as u16,
            (v * mesh::UV_STEPS) as u16,
        ],
        layer: 0,
        light,
        color: white,
    };
    [
        corner(0.0, 0.0, 0.0, 0.0),
        corner(0.0, 1.0, 0.0, 1.0),
        corner(1.0, 1.0, 1.0, 1.0),
        corner(1.0, 0.0, 1.0, 0.0),
    ]
}

/// The six indices of the quad, as two triangles: `0-1-2` and `0-2-3`. `u16` because 2.1 has no
/// other index type in practice and the executor passes `GL_UNSIGNED_SHORT`.
pub const QUAD_INDEX_LIST: [u16; 6] = [0, 1, 2, 0, 2, 3];

/// Builds the upload for the first frame: the quad's vertices in the format's layout, its
/// indices, and the stride.
///
/// `placement` is the atlas builder's table; sprite 0 is the only sprite the quad references,
/// so a table without it is an error rather than a black quad with no explanation.
pub fn quad_upload(placement: &AtlasPlacement) -> Result<Upload, mesh::MeshError> {
    let vertices = mesh::to_bytes(&quad_vertices(), Uv::Sprite, placement)?;
    let stride = VertexFormat::legacy().stride;
    Ok(Upload {
        vertices,
        indices: QUAD_INDEX_LIST.to_vec(),
        stride,
    })
}

/// The draw list of the first frame: one pass, the screen, cleared to the sky, and one terrain
/// draw whose geometry is the quad.
///
/// The pass declares no [`crate::draw::Requirement`]: a clear and a textured triangle are there
/// on any 2.1 context, and a first frame that demanded an extension would not be the first frame
/// on the machine this path exists for.
pub fn first_frame(uniforms: TerrainUniforms, viewport: Viewport) -> DrawList {
    let mut list = DrawList::new();
    let pass = list.begin_pass(
        Target::Screen,
        viewport,
        Clear {
            color: Some([0.62, 0.78, 1.0, 1.0]),
            depth: Some(1.0),
            stencil: None,
        },
        "first frame",
    );

    let geometry = Geometry {
        buffer: QUAD_VERTICES,
        vertex_offset: 0,
        stride: VertexFormat::legacy().stride,
        indices: QUAD_INDICES,
        // A second quad appended to the same index buffer starts at 6 * 2, because the offset
        // is in bytes: this one starts at the beginning.
        index_offset: 0,
        index_count: QUAD_INDEX_LIST.len() as u32,
        base_vertex: 0,
    };

    let mut draw = Draw::new(TERRAIN_PIPELINE, geometry, "terrain quad");
    draw.state = State {
        blend: Blend::Off,
        depth: DepthState::ReadWrite,
        cull: Cull::Back,
    };
    draw.origin = [0.0; 3];
    let mut frame_uniforms = Uniforms::new();
    frame_uniforms.set(uniforms.chunk_origin, Value::Vec3([0.0; 3]));
    // An opaque pass: `u_pass.x` is the cutout flag, and this quad is not one.
    frame_uniforms.set(uniforms.pass, Value::Vec4([0.0, 0.0, 0.0, 0.0]));
    frame_uniforms.set(uniforms.fog_color, Value::Vec4([0.62, 0.78, 1.0, 0.0]));
    frame_uniforms.set(uniforms.fog, Value::Vec4([0.0, 0.0, 0.0, 0.0]));
    draw.uniforms = frame_uniforms;

    pass.draws.push(draw);
    list
}

/// The first frame's one draw as a multi-draw run, so the batching path is exercised on the one
/// pass that is known to be correct before it is exercised on 800 sections.
pub fn quad_run() -> MultiDrawElements {
    let mut run = MultiDrawElements::with_capacity(1);
    run.push(QUAD_INDEX_LIST.len() as u32, 0);
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement() -> AtlasPlacement {
        let mut placement = AtlasPlacement::with_capacity(1);
        placement.place(0, 0).unwrap();
        placement
    }

    #[test]
    fn the_upload_is_four_vertices_of_the_formats_stride_and_six_indices() {
        let upload = quad_upload(&placement()).unwrap();
        let stride = VertexFormat::legacy().stride as usize;
        assert_eq!(stride, mesh::VERTEX_BYTES);
        assert_eq!(upload.vertices.len(), 4 * stride);
        assert_eq!(upload.indices.len(), 6);
        assert_eq!(upload.stride, VertexFormat::legacy().stride);
        // A u16 index buffer of six entries is twelve bytes, which is what the executor's
        // `index_problem` is asked about.
        assert_eq!(upload.indices.len() * 2, 12);
    }

    #[test]
    fn the_quad_is_a_block_top_face_with_its_uvs_and_light_intact() {
        let upload = quad_upload(&placement()).unwrap();
        let stride = mesh::VERTEX_BYTES;
        let read = |index: usize, at: usize, width: usize| {
            let start = index * stride + at;
            upload.vertices[start..start + width].to_vec()
        };
        // Corner 2 is (1, 1, 1) with UV (1, 1): the far corner in both.
        let x = f32::from_le_bytes(read(2, 0, 4).try_into().unwrap());
        let y = f32::from_le_bytes(read(2, 4, 4).try_into().unwrap());
        let z = f32::from_le_bytes(read(2, 8, 4).try_into().unwrap());
        assert_eq!((x, y, z), (1.0, 1.0, 1.0));
        let u = f32::from_le_bytes(read(2, 12, 4).try_into().unwrap());
        let v = f32::from_le_bytes(read(2, 16, 4).try_into().unwrap());
        assert_eq!((u, v), (1.0, 1.0));
        // Full sky light, no block light, white, atlas 0.
        assert_eq!(read(2, 24, 2), vec![255, 0]);
        assert_eq!(read(2, 20, 4), vec![255, 255, 255, 255]);
        assert_eq!(f32::from_le_bytes(read(2, 28, 4).try_into().unwrap()), 0.0);
    }

    /// The winding test: two triangles, and the cross product of the first one points up. A
    /// counter-clockwise quad in XZ seen from +Y has a normal with a positive Y, which is what
    /// `Cull::Back` needs; the other winding makes the ground invisible from above.
    #[test]
    fn the_quad_winds_counter_clockwise_seen_from_above() {
        let corners = quad_vertices();
        let triangle = [0usize, 1, 2];
        let a = corners[triangle[0]].pos;
        let b = corners[triangle[1]].pos;
        let c = corners[triangle[2]].pos;
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        // The Y component of ab x ac.
        let normal_y = ab[2] * ac[0] - ab[0] * ac[2];
        assert!(normal_y < 0.0, "normal_y {normal_y}: the face points down");
        // The second triangle has the same winding, or half the quad disappears.
        let triangle = [0usize, 2, 3];
        let a = corners[triangle[0]].pos;
        let b = corners[triangle[1]].pos;
        let c = corners[triangle[2]].pos;
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let normal_y = ab[2] * ac[0] - ab[0] * ac[2];
        assert!(normal_y < 0.0, "the second triangle winds the other way");
    }

    #[test]
    fn the_frame_is_one_pass_one_draw_with_a_depth_write_and_back_face_culling() {
        let uniforms = TerrainUniforms {
            view_proj: UniformId(0),
            chunk_origin: UniformId(1),
            pass: UniformId(2),
            fog_color: UniformId(3),
            fog: UniformId(4),
        };
        let list = first_frame(uniforms, Viewport::full(800, 600));
        assert_eq!(list.passes().len(), 1);
        assert_eq!(list.draw_count(), 1);
        assert_eq!(list.triangle_count(), 2);
        let pass = &list.passes()[0];
        assert_eq!(pass.label, "first frame");
        assert_eq!(pass.target, Target::Screen);
        // Clears colour and depth: a pass that leaves depth alone draws the sky onto last
        // frame's depth and the quad vanishes.
        assert!(pass.clear.color.is_some() && pass.clear.depth.is_some());
        assert_eq!(pass.viewport, Viewport::full(800, 600));
        // No requirement: a clear and a textured triangle are on every 2.1 context, and a
        // first frame that needed an extension would not run on the target at all.
        assert!(pass.needs.is_empty(), "{:?}", pass.needs);
        let draw = &pass.draws[0];
        assert_eq!(draw.state.depth, DepthState::ReadWrite);
        assert_eq!(draw.state.cull, Cull::Back);
        assert_eq!(draw.state.blend, Blend::Off);
        assert_eq!(draw.geometry.index_count, 6);
        assert_eq!(draw.geometry.stride, VertexFormat::legacy().stride);
        assert_eq!(draw.geometry.buffer, QUAD_VERTICES);
        assert_eq!(draw.geometry.indices, QUAD_INDICES);
    }

    #[test]
    fn the_quad_needs_its_sprite_or_it_says_which_one_is_missing() {
        // An atlas table without sprite 0: the quad cannot pick a half of the texture, and the
        // error names the sprite rather than drawing black.
        let empty = AtlasPlacement::default();
        assert_eq!(
            quad_upload(&empty),
            Err(mesh::MeshError::UnplacedSprite { sprite: 0 })
        );
    }

    #[test]
    fn the_opaque_flag_is_not_set_on_a_bare_quad() {
        for vertex in &quad_vertices() {
            assert_eq!(vertex.layer & OPAQUE_LAYER, 0);
            assert!(!vertex.opaque());
            assert_eq!(vertex.sprite(), 0);
        }
    }

    #[test]
    fn the_run_for_the_quad_is_one_draw_of_six_indices() {
        let run = quad_run();
        assert_eq!(run.len(), 1);
        assert_eq!(run.counts, [6]);
        assert_eq!(run.offsets, [0]);
        assert_eq!(run.indices(), 6);
        assert_eq!(run.indices() / 3, 2);
    }

    /// The one uniform the WGSL's fade relies on: the origin is passed as a `vec3` of whole
    /// blocks, not folded into the vertex positions, because 2.1 has no instancing and a world
    /// coordinate of 30 million does not survive a `f32`.
    #[test]
    fn the_origin_travels_as_a_uniform() {
        let uniforms = TerrainUniforms {
            view_proj: UniformId(0),
            chunk_origin: UniformId(1),
            pass: UniformId(2),
            fog_color: UniformId(3),
            fog: UniformId(4),
        };
        let list = first_frame(uniforms, Viewport::full(64, 64));
        let draw = &list.passes()[0].draws[0];
        assert_eq!(draw.origin, [0.0; 3]);
        assert_eq!(
            draw.uniforms.get(UniformId(1)),
            Some(Value::Vec3([0.0; 3]))
        );
        // And the cutout flag is 0 for this pass: the quad must not be discarded.
        assert_eq!(
            draw.uniforms.get(UniformId(2)),
            Some(Value::Vec4([0.0, 0.0, 0.0, 0.0]))
        );
    }

    #[test]
    fn the_vertex_buffer_geometry_matches_the_upload() {
        let upload = quad_upload(&placement()).unwrap();
        let list = first_frame(
            TerrainUniforms {
                view_proj: UniformId(0),
                chunk_origin: UniformId(1),
                pass: UniformId(2),
                fog_color: UniformId(3),
                fog: UniformId(4),
            },
            Viewport::full(1, 1),
        );
        let geometry = list.passes()[0].draws[0].geometry;
        // Four vertices at the stride, and the draw's first vertex is the buffer's start.
        assert_eq!(upload.vertices.len() as u32 / geometry.stride, 4);
        assert_eq!(geometry.vertex_offset, 0);
        // Every index is inside those four vertices: what `index_problem` cannot see, because
        // it checks the buffer against the draw, not the indices against the vertex count.
        for index in &upload.indices {
            assert!((*index as u32) < 4);
        }
    }

    /// The frame the executor refuses, before its first draw, is the one that names a feature.
    /// This pass names none, so it must be drawable both on the target's table and on a
    /// stricter five-and-a-half-times-newer one: it asks for nothing in the first place.
    #[test]
    fn the_pass_is_drawable_on_the_target_and_on_anything_newer() {
        let strict = crate::caps::Caps::detect(
            crate::caps::Version::parse("2.1.8545 Release"),
            "1.20",
            "ATI Technologies Inc.",
            "Radeon X1600/X1650 Series",
            std::iter::empty(),
            crate::caps::Limits::X1650,
        )
        .strict_over();
        let list = first_frame(
            TerrainUniforms {
                view_proj: UniformId(0),
                chunk_origin: UniformId(1),
                pass: UniformId(2),
                fog_color: UniformId(3),
                fog: UniformId(4),
            },
            Viewport::full(800, 600),
        );
        assert!(list.unmet(&strict).is_empty(), "{:?}", list.unmet(&strict));
    }
}
