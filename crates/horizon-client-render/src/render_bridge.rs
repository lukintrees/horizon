//! CPU bridge from the existing mesher to the backend-neutral SDR upload contract.
//! Does not choose/switch the game's renderer. No wgpu or GL objects cross this boundary.
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

/// SectionMesh retains full solid vertices alongside its optional packed cache;
/// use those directly rather than attempting GPU vertex-pulling on GL2.
/// Translucent input order must already be sorted by the caller for its camera.
/// Repeating/tiled LOD UVs are explicitly unsupported, not silently clamped.
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
    horizon_render_api::terrain::runs(
        atlas,
        vertices.iter().map(TerrainInput::from),
        translucent,
        max_vertices,
    )
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
}
