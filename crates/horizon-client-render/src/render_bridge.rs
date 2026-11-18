//! CPU bridge from the existing mesher to the backend-neutral SDR upload contract.
//! Does not choose or switch the game's renderer. No wgpu or GL object crosses
//! this boundary, so the modern path and the legacy one share it unchanged.
use crate::mesher::{SectionMesh, TerrainVertex};
use horizon_render_api::{
    atlas::AtlasLayout,
    terrain::{Run, TerrainError, TerrainInput},
};

impl From<&TerrainVertex> for TerrainInput {
    fn from(v: &TerrainVertex) -> Self {
        Self {
            pos: v.pos,
            uv: v.uv(),
            layer: v.layer,
            color: v.color,
            light: v.light,
        }
    }
}

/// `SectionMesh` retains full solid vertices alongside its optional packed cache;
/// use those directly rather than attempting GPU vertex-pulling on GL2.
/// Translucent input order must already be sorted by the caller for its camera.
/// Repeating/tiled LOD UVs are explicitly unsupported, not silently clamped.
///
/// Solid passes (opaque and cutout) write depth and never blend, so the order the
/// mesher emitted their quads in cannot change the picture: they are grouped by atlas
/// page and pass first. The mesher emits quads in block order, so an ungrouped section
/// alternates pages constantly and becomes one GPU mesh per page change -- ~50 per
/// vanilla section rather than the handful of pages it actually touches, which a wide
/// view then multiplies past the backend's mesh object limit.
pub fn section_runs(
    mesh: &SectionMesh,
    atlas: &AtlasLayout,
    translucent: bool,
    max_vertices: usize,
) -> Result<Vec<Run>, TerrainError> {
    let vertices = if translucent {
        &mesh.translucent
    } else {
        &mesh.solid
    };
    if translucent {
        return horizon_render_api::terrain::runs(
            atlas,
            vertices.iter().map(TerrainInput::from),
            true,
            max_vertices,
        );
    }
    // A trailing partial quad is the mesher's invariant to keep, not ours to drop:
    // hand it to the plain path so it reports IncompleteQuad as it always has.
    if vertices.len() % 4 != 0 {
        return horizon_render_api::terrain::runs(
            atlas,
            vertices.iter().map(TerrainInput::from),
            false,
            max_vertices,
        );
    }
    // Stable order within a page is kept by sorting quad indices, not quads.
    let mut quads = Vec::with_capacity(vertices.len() / 4);
    for (index, quad) in vertices.chunks_exact(4).enumerate() {
        let layer = quad[0].layer & 0x7fff;
        let place = atlas
            .sprites()
            .get(layer as usize)
            .ok_or(TerrainError::MissingSprite(layer))?;
        // Cutout is the low bit: neighbours in the same key share a page and a pass.
        quads.push((place.page << 1 | usize::from(quad[0].layer & 0x8000 == 0), index));
    }
    quads.sort_by_key(|&(key, _)| key);
    let ordered = quads
        .into_iter()
        .flat_map(|(_, index)| vertices[index * 4..index * 4 + 4].iter())
        .map(TerrainInput::from);
    horizon_render_api::terrain::runs(atlas, ordered, false, max_vertices)
}

#[derive(Debug)]
pub enum UploadError<E> {
    Terrain(TerrainError),
    Scene(horizon_render_api::scene::SceneError<E>),
}

/// Convert both passes before changing resident GPU resources. Conversion or
/// upload failure leaves the previous section intact; the vertex ceiling covers
/// solid and translucent geometry together, not a separate allowance per pass.
/// The caller still owns visibility, job scheduling and translucent quad sorting.
pub fn upload_section<B: horizon_render_api::RenderBackend>(
    scene: &mut horizon_render_api::scene::WorldScene<B>,
    key: horizon_render_api::scene::SectionKey,
    mesh: &SectionMesh,
    max_vertices: usize,
) -> Result<(), UploadError<B::Error>> {
    if mesh
        .solid
        .len()
        .checked_add(mesh.translucent.len())
        .is_none_or(|n| n > max_vertices)
    {
        return Err(UploadError::Terrain(TerrainError::VertexBudget));
    }
    let solid =
        section_runs(mesh, scene.atlas(), false, max_vertices).map_err(UploadError::Terrain)?;
    let translucent = section_runs(mesh, scene.atlas(), true, max_vertices - mesh.solid.len())
        .map_err(UploadError::Terrain)?;
    scene
        .replace_section(key, &solid, &translucent)
        .map_err(UploadError::Scene)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_render_api::{
        Pass,
        atlas::{SpriteSize, pack_atlas},
    };
    #[test]
    fn real_mesher_uv_fixed_point_and_material_alpha_reach_the_sdr_contract() {
        let mut mesh = SectionMesh::default();
        mesh.solid
            .extend([TerrainVertex::new([1., 2., 3.], [0.5, 1.], 0x8000, 0x8d403020, 0xf080); 4]);
        let atlas = pack_atlas(
            &[SpriteSize {
                width: 2,
                height: 2,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let runs = section_runs(&mesh, &atlas, false, 4).unwrap();
        assert_eq!(runs[0].pass, Pass::Opaque);
        assert_eq!(runs[0].vertices[0].uv, [0.5, 0.75]);
        assert_eq!(
            runs[0].vertices[0].color,
            [32. / 255., 48. / 255., 64. / 255., 1.]
        );
        assert_eq!(runs[0].indices, [0, 1, 2, 0, 2, 3]);
    }
    #[test]
    fn solid_quads_are_grouped_by_atlas_page_and_pass_into_one_run_each() {
        // Four 2x2 sprites fill a 4 px page, so layer 4 is the first on page 1. The
        // mesher's own order alternates pages and passes, so grouping is what keeps
        // this at three GPU meshes instead of one per swap.
        let atlas = pack_atlas(&[SpriteSize { width: 2, height: 2 }; 5], 4, 0, 2).unwrap();
        let quad = |layer: u16| {
            [TerrainVertex::new(
                [0., 0., 0.],
                [0., 0.],
                layer,
                u32::MAX,
                0,
            ); 4]
        };
        let mut mesh = SectionMesh::default();
        // opaque page 0, cutout page 0, opaque page 1, interleaved.
        for layer in [0x8000u16, 1, 0x8000, 1, 0x8004, 3] {
            mesh.solid.extend(quad(layer));
        }
        let runs = section_runs(&mesh, &atlas, false, 64).unwrap();
        // Page order first, then the pass the mesher gave within that page.
        assert_eq!(
            runs.iter().map(|r| (r.page, r.pass)).collect::<Vec<_>>(),
            vec![(0, Pass::Opaque), (0, Pass::Cutout), (1, Pass::Opaque)]
        );
        // Every quad survives the reordering: 6 quads, 24 vertices, 36 indices.
        assert_eq!(runs.iter().map(|r| r.vertices.len()).sum::<usize>(), 24);
        assert_eq!(runs.iter().map(|r| r.indices.len()).sum::<usize>(), 36);
        // A layer the atlas does not have is still the error it was.
        let mut bad = SectionMesh::default();
        bad.solid.extend(quad(7));
        assert!(matches!(
            section_runs(&bad, &atlas, false, 64),
            Err(TerrainError::MissingSprite(7))
        ));
    }
}
