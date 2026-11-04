//! Conversion of Horizon's section quads to the prototype GL 2.1 vertex format.
//! This does not upload or draw them. Batches are consecutive rather than
//! sorted by page so the caller can preserve translucent draw order.
use super::atlas::{Atlas, AtlasError};
use crate::mesher::{OPAQUE_LAYER, SectionMesh, TILED_UV_STEPS, TerrainVertex, UV_STEPS};

pub use horizon_legacy_gl::{Batch, Vertex};

/// Convert the game's fog data for a section whose origin is relative to the
/// camera. Sky and cloud fog ends are for other pipelines and are not used.
pub fn fog(data: &crate::fog::FogData, section_offset: [f32; 3]) -> horizon_legacy_gl::Fog {
    horizon_legacy_gl::Fog {
        color: data.color,
        ranges: [
            data.environmental_start,
            data.environmental_end,
            data.render_distance_start,
            data.render_distance_end,
        ],
        section_offset,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum MeshError {
    IncompleteQuad,
    MixedSpriteLayers,
    Atlas(AtlasError),
    TooManyVertices,
    AllocationFailed,
}

#[derive(Debug, Default)]
pub struct Mesh {
    /// Six vertices per quad in the same winding as the main renderer.
    pub vertices: Vec<Vertex>,
    pub batches: Vec<Batch>,
}

/// Convert the two near-section render passes separately. Translucent quads
/// retain their input order; callers are responsible for sorting them per view.
pub fn build_section(atlas: &Atlas, section: &SectionMesh) -> Result<(Mesh, Mesh), MeshError> {
    Ok((
        build(atlas, &section.solid)?,
        build(atlas, &section.translucent)?,
    ))
}

/// Build a full-bright draw list in source order. Input must be near-section
/// vertices, not `TerrainVertex::tiled` (its UV has a different fixed-point
/// scale). For world lighting use `build_with_lightmap` instead.
pub fn build(atlas: &Atlas, quads: &[TerrainVertex]) -> Result<Mesh, MeshError> {
    build_inner(atlas, quads, None, 1.0)
}

/// Bake the game's 16x16 RGBA lightmap into the vertex colors. This avoids
/// requiring vertex texture fetch, which is optional in OpenGL 2.1. Rebuild the
/// mesh when the lightmap changes. Rows are sky, columns are block light.
pub fn build_with_lightmap(
    atlas: &Atlas,
    quads: &[TerrainVertex],
    lightmap: &[u8; 16 * 16 * 4],
) -> Result<Mesh, MeshError> {
    build_inner(atlas, quads, Some(lightmap), 1.0)
}

/// Convert merged distant-terrain quads created with `TerrainVertex::tiled`.
/// Their fixed-point UV encoding differs from near terrain; the shader repeats
/// the sprite across each block of the merged face.
pub fn build_tiled(atlas: &Atlas, quads: &[TerrainVertex]) -> Result<Mesh, MeshError> {
    build_inner(atlas, quads, None, UV_STEPS / TILED_UV_STEPS)
}

/// Convert distant terrain and bake the current lightmap into its vertices.
pub fn build_tiled_with_lightmap(
    atlas: &Atlas,
    quads: &[TerrainVertex],
    lightmap: &[u8; 16 * 16 * 4],
) -> Result<Mesh, MeshError> {
    build_inner(atlas, quads, Some(lightmap), UV_STEPS / TILED_UV_STEPS)
}

/// Sample linearly like the existing terrain shader's lightmap sampler.
fn lightmap_rgb(light: u16, pixels: &[u8; 16 * 16 * 4]) -> [f32; 3] {
    let sky = f32::from(light as u8).min(240.0) / 16.0;
    let block = f32::from((light >> 8) as u8).min(240.0) / 16.0;
    let x = block as usize;
    let y = sky as usize;
    let dx = block - x as f32;
    let dy = sky - y as f32;
    let sample = |x: usize, y: usize, c: usize| f32::from(pixels[(y * 16 + x) * 4 + c]) / 255.0;
    std::array::from_fn(|c| {
        let top = sample(x, y, c) * (1.0 - dx) + sample((x + 1).min(15), y, c) * dx;
        let bottom = sample(x, (y + 1).min(15), c) * (1.0 - dx)
            + sample((x + 1).min(15), (y + 1).min(15), c) * dx;
        top * (1.0 - dy) + bottom * dy
    })
}

fn build_inner(
    atlas: &Atlas,
    quads: &[TerrainVertex],
    lightmap: Option<&[u8; 16 * 16 * 4]>,
    uv_scale: f32,
) -> Result<Mesh, MeshError> {
    if !quads.len().is_multiple_of(4) {
        return Err(MeshError::IncompleteQuad);
    }
    let count = (quads.len() / 4)
        .checked_mul(6)
        .ok_or(MeshError::TooManyVertices)?;
    // glDrawArrays uses signed 32-bit first/count. Reject oversized input rather
    // than truncate indices when a future uploader casts them.
    if count > i32::MAX as usize {
        return Err(MeshError::TooManyVertices);
    }
    let mut mesh = Mesh::default();
    mesh.vertices
        .try_reserve_exact(count)
        .map_err(|_| MeshError::AllocationFailed)?;
    for quad in quads.as_chunks::<4>().0 {
        let layer = quad[0].layer & !OPAQUE_LAYER;
        if quad.iter().any(|v| v.layer & !OPAQUE_LAYER != layer) {
            return Err(MeshError::MixedSpriteLayers);
        }
        let location = atlas.location(u32::from(layer)).map_err(MeshError::Atlas)?;
        if mesh
            .batches
            .last()
            .is_none_or(|last| last.page != location.page)
        {
            mesh.batches
                .try_reserve(1)
                .map_err(|_| MeshError::AllocationFailed)?;
            mesh.batches.push(Batch {
                page: location.page,
                first: mesh.vertices.len(),
                count: 0,
            });
        }
        for index in [0, 1, 2, 0, 2, 3] {
            let vertex = quad[index];
            // The terrain alpha contains HDR metadata, not opacity.
            let [r, g, b, _metadata] = vertex.color.to_le_bytes();
            let light = lightmap.map_or([1.0; 3], |pixels| lightmap_rgb(vertex.light, pixels));
            mesh.vertices.push(Vertex {
                position: vertex.pos,
                sprite_uv: vertex.uv().map(|uv| uv * uv_scale),
                atlas_rect: location.rect,
                color: [
                    f32::from(r) / 255.0 * light[0],
                    f32::from(g) / 255.0 * light[1],
                    f32::from(b) / 255.0 * light[2],
                    1.0,
                ],
            });
        }
        mesh.batches
            .last_mut()
            .expect("a batch was just added")
            .count += 6;
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(layer: u16) -> Vec<TerrainVertex> {
        [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
            .into_iter()
            .map(|uv| TerrainVertex::new([uv[0], uv[1], 0.], uv, layer, 0x031e140a, 0))
            .collect()
    }

    #[test]
    fn preserves_winding_tint_and_translucent_order_across_pages() {
        let atlas = Atlas::new(1, 2, 4, &[255; 8]).unwrap();
        let vertices: Vec<_> = [quad(OPAQUE_LAYER), quad(1), quad(0)].concat();
        let mesh = build(&atlas, &vertices).unwrap();
        assert_eq!(
            mesh.batches,
            [
                Batch {
                    page: 0,
                    first: 0,
                    count: 6
                },
                Batch {
                    page: 1,
                    first: 6,
                    count: 6
                },
                Batch {
                    page: 0,
                    first: 12,
                    count: 6
                }
            ]
        );
        assert_eq!(
            mesh.vertices
                .iter()
                .take(6)
                .map(|v| v.position)
                .collect::<Vec<_>>(),
            [
                [0., 0., 0.],
                [1., 0., 0.],
                [1., 1., 0.],
                [0., 0., 0.],
                [1., 1., 0.],
                [0., 1., 0.]
            ]
        );
        assert_eq!(
            mesh.vertices[0].color,
            [10. / 255., 20. / 255., 30. / 255., 1.]
        );
        assert_eq!(mesh.vertices[2].sprite_uv, [1., 1.]);
    }

    #[test]
    fn lightmap_bilinear_interpolation_and_clamping() {
        let mut pixels = [0; 16 * 16 * 4];
        for y in 0..16 {
            for x in 0..16 {
                let i = (y * 16 + x) * 4;
                pixels[i..i + 4].copy_from_slice(&[(x * 16) as u8, (y * 16) as u8, 128, 255]);
            }
        }
        let sample = lightmap_rgb(0x0808, &pixels);
        assert_eq!(sample, [8.0 / 255.0, 8.0 / 255.0, 128.0 / 255.0]);
        assert_eq!(
            lightmap_rgb(0xffff, &pixels),
            [240.0 / 255.0, 240.0 / 255.0, 128.0 / 255.0]
        );
        let atlas = Atlas::new(1, 1, 4, &[255; 4]).unwrap();
        let mut vertices = quad(0);
        vertices.iter_mut().for_each(|v| v.light = 0x0808);
        let result = build_with_lightmap(&atlas, &vertices, &pixels).unwrap();
        assert_eq!(result.vertices[0].color[0], (10.0 / 255.0) * sample[0]);
    }

    #[test]
    fn section_keeps_solid_and_translucent_separate() {
        let atlas = Atlas::new(1, 1, 4, &[255; 4]).unwrap();
        let section = SectionMesh {
            solid: quad(0),
            translucent: [quad(0), quad(0)].concat(),
            ..SectionMesh::default()
        };
        let (solid, translucent) = build_section(&atlas, &section).unwrap();
        assert_eq!(solid.vertices.len(), 6);
        assert_eq!(translucent.vertices.len(), 12);
    }

    #[test]
    fn distant_uv_uses_tiled_encoding_and_can_bake_light() {
        let atlas = Atlas::new(1, 1, 4, &[255, 255, 255, 255]).unwrap();
        let quad: Vec<_> = [[0., 0.], [3.5, 0.], [3.5, 2.], [0., 2.]]
            .into_iter()
            .map(|uv| TerrainVertex::tiled([uv[0], uv[1], 0.], uv, 0, 0xffff_ffff, 0))
            .collect();
        let mesh = build_tiled(&atlas, &quad).unwrap();
        assert_eq!(mesh.vertices[1].sprite_uv, [3.5, 0.]);
        assert_eq!(mesh.vertices[2].sprite_uv, [3.5, 2.]);
        let light = [128; 16 * 16 * 4];
        let lit = build_tiled_with_lightmap(&atlas, &quad, &light).unwrap();
        assert_eq!(lit.vertices[1].sprite_uv, [3.5, 0.]);
        assert!((lit.vertices[1].color[0] - 128. / 255.).abs() < 1e-6);
    }

    #[test]
    fn converts_world_fog_ranges_and_camera_relative_offset() {
        let data = crate::fog::FogData {
            color: [0.1, 0.2, 0.3, 0.4],
            environmental_start: 1.0,
            environmental_end: 2.0,
            render_distance_start: 3.0,
            render_distance_end: 4.0,
            sky_end: 5.0,
            cloud_end: 6.0,
        };
        let actual = fog(&data, [7., 8., 9.]);
        assert_eq!(actual.color, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(actual.ranges, [1., 2., 3., 4.]);
        assert_eq!(actual.section_offset, [7., 8., 9.]);
    }

    #[test]
    fn combines_consecutive_pages_and_rejects_bad_input() {
        let atlas = Atlas::new(1, 1, 4, &[255; 4]).unwrap();
        let mesh = build(&atlas, &[quad(0), quad(0)].concat()).unwrap();
        assert_eq!(
            mesh.batches,
            [Batch {
                page: 0,
                first: 0,
                count: 12
            }]
        );
        assert_eq!(
            build(&atlas, &quad(0)[..3]).unwrap_err(),
            MeshError::IncompleteQuad
        );
        let mut bad = quad(0);
        bad[1].layer = 1;
        assert_eq!(
            build(&atlas, &bad).unwrap_err(),
            MeshError::MixedSpriteLayers
        );
        assert_eq!(
            build(&atlas, &quad(1)).unwrap_err(),
            MeshError::Atlas(AtlasError::LayerOutOfRange)
        );
    }
}
