//! CPU-side resident world for the experimental GL backend. It accepts the same
//! section meshes as the game renderer, retaining originals to rebake lighting.
//! It is not yet connected to the game's renderer selection or window lifecycle.
use std::collections::BTreeMap;

use super::{atlas::Atlas, terrain};
use crate::mesher::{SectionMesh, TerrainVertex};
use horizon_legacy_gl::{Draw, Fog, Pass};

/// Like `gfx::SectionKey`: x/z are chunk coordinates, y is an index relative
/// to the dimension's minimum Y, not an absolute section coordinate.
pub type SectionKey = (i32, i32, i32);

struct Section {
    solid_source: Vec<TerrainVertex>,
    translucent_source: Vec<TerrainVertex>,
    solid: terrain::Mesh,
    translucent: terrain::Mesh,
}

/// Converted sections and their sprite atlas. Each upload replaces one section;
/// empty sections remain resident so `contains` has the same semantics as the
/// game's uploader. This intentionally makes no assumptions about visibility.
pub struct World {
    atlas: Atlas,
    lightmap: [u8; 16 * 16 * 4],
    sections: BTreeMap<SectionKey, Section>,
}

impl World {
    pub fn new(atlas: Atlas, lightmap: [u8; 16 * 16 * 4]) -> Self {
        Self {
            atlas,
            lightmap,
            sections: BTreeMap::new(),
        }
    }

    pub fn atlas(&self) -> &Atlas {
        &self.atlas
    }
    /// Updating sprite pixels does not change locations or invalidate meshes.
    /// The caller must upload the changed page to its GL renderer afterwards.
    pub fn atlas_mut(&mut self) -> &mut Atlas {
        &mut self.atlas
    }
    pub fn contains(&self, key: SectionKey) -> bool {
        self.sections.contains_key(&key)
    }
    pub fn len(&self) -> usize {
        self.sections.len()
    }
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
    pub fn remove(&mut self, key: SectionKey) {
        self.sections.remove(&key);
    }
    pub fn clear(&mut self) {
        self.sections.clear();
    }

    /// A failed conversion leaves the previous section untouched.
    pub fn upload(
        &mut self,
        key: SectionKey,
        mesh: &SectionMesh,
    ) -> Result<(), terrain::MeshError> {
        let solid = terrain::build_with_lightmap(&self.atlas, &mesh.solid, &self.lightmap)?;
        let translucent =
            terrain::build_with_lightmap(&self.atlas, &mesh.translucent, &self.lightmap)?;
        self.sections.insert(
            key,
            Section {
                solid_source: mesh.solid.clone(),
                translucent_source: mesh.translucent.clone(),
                solid,
                translucent,
            },
        );
        Ok(())
    }

    /// Rebake when the game's lightmap changes. Do not install a partial update
    /// if allocation or conversion fails. This costs CPU and memory and will
    /// eventually need a more incremental path for large worlds on 32-bit PCs.
    pub fn set_lightmap(
        &mut self,
        lightmap: [u8; 16 * 16 * 4],
    ) -> Result<bool, terrain::MeshError> {
        if self.lightmap == lightmap {
            return Ok(false);
        }
        let mut replacement = Vec::new();
        replacement
            .try_reserve_exact(self.sections.len())
            .map_err(|_| terrain::MeshError::AllocationFailed)?;
        for (&key, section) in &self.sections {
            replacement.push((
                key,
                terrain::build_with_lightmap(&self.atlas, &section.solid_source, &lightmap)?,
                terrain::build_with_lightmap(&self.atlas, &section.translucent_source, &lightmap)?,
            ));
        }
        for (key, solid, translucent) in replacement {
            let section = self
                .sections
                .get_mut(&key)
                .expect("keys came from this map");
            section.solid = solid;
            section.translucent = translucent;
        }
        self.lightmap = lightmap;
        Ok(true)
    }

    /// Prepare a depth-correct pass list for already-visible sections. `view_projection`
    /// is column-major OpenGL clip space for camera-relative world positions. A
    /// section is sorted as a unit; individual translucent quads are not yet
    /// sorted within it. Visibility, sky, entities, distant tiles and the HUD
    /// are deliberately outside this partial bridge.
    pub fn plan<'a>(
        &'a self,
        visible: &[SectionKey],
        camera: [f64; 3],
        min_y: i32,
        view_projection: [f32; 16],
        fog: Option<Fog>,
    ) -> Result<Vec<SectionDraw<'a>>, PlanError> {
        if camera.iter().any(|x| !x.is_finite()) || view_projection.iter().any(|x| !x.is_finite()) {
            return Err(PlanError::InvalidTransform);
        }
        let mut keys = visible.to_vec();
        keys.sort_unstable();
        keys.dedup();
        let mut items = Vec::new();
        for key in keys {
            if let Some(section) = self.sections.get(&key) {
                // Subtract in f64 before conversion: far from the origin a
                // float world position alone loses block-sized detail.
                let origin = [
                    f64::from(key.0) * 16.0,
                    f64::from(min_y) + f64::from(key.1) * 16.0,
                    f64::from(key.2) * 16.0,
                ];
                let delta = std::array::from_fn::<_, 3, _>(|i| origin[i] - camera[i]);
                let offset = delta.map(|v| v as f32);
                if offset.iter().any(|x| !x.is_finite()) {
                    return Err(PlanError::InvalidTransform);
                }
                let distance = delta.iter().map(|v| (v + 8.0) * (v + 8.0)).sum::<f64>();
                let mut transform = view_projection;
                for row in 0..4 {
                    transform[12 + row] = view_projection[row] * offset[0]
                        + view_projection[4 + row] * offset[1]
                        + view_projection[8 + row] * offset[2]
                        + view_projection[12 + row];
                }
                if transform.iter().any(|x| !x.is_finite()) {
                    return Err(PlanError::InvalidTransform);
                }
                items.push((key, section, offset, distance, transform));
            }
        }
        items.sort_by(|a, b| a.3.total_cmp(&b.3).then_with(|| a.0.cmp(&b.0)));
        let mut result = Vec::new();
        for &(key, section, offset, _, transform) in &items {
            if !section.solid.vertices.is_empty() {
                result.push(SectionDraw::new(
                    key,
                    &section.solid,
                    transform,
                    offset,
                    fog,
                    false,
                ));
            }
        }
        for &(key, section, offset, _, transform) in items.iter().rev() {
            if !section.translucent.vertices.is_empty() {
                result.push(SectionDraw::new(
                    key,
                    &section.translucent,
                    transform,
                    offset,
                    fog,
                    true,
                ));
            }
        }
        Ok(result)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PlanError {
    InvalidTransform,
}

/// One VBO upload for a section's solid or translucent geometry. Execute with
/// `Renderer::draw_ordered_with_fog` (or `draw_ordered` without fog), in order.
pub struct SectionDraw<'a> {
    pub key: SectionKey,
    pub vertices: &'a [horizon_legacy_gl::Vertex],
    pub draws: Vec<Draw>,
    pub transform: [f32; 16],
    pub fog: Option<Fog>,
}
impl<'a> SectionDraw<'a> {
    fn new(
        key: SectionKey,
        mesh: &'a terrain::Mesh,
        transform: [f32; 16],
        offset: [f32; 3],
        fog: Option<Fog>,
        translucent: bool,
    ) -> Self {
        let draws = mesh
            .batches
            .iter()
            .zip(&mesh.opaque)
            .map(|(&batch, &opaque)| Draw {
                batch,
                pass: if translucent {
                    Pass::Translucent { alpha_cutoff: 0.01 }
                } else {
                    Pass::Solid {
                        alpha_cutoff: if opaque { 0.0 } else { 0.5 },
                    }
                },
            })
            .collect();
        Self {
            key,
            vertices: &mesh.vertices,
            draws,
            transform,
            fog: fog.map(|mut f| {
                f.section_offset = offset;
                f
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::OPAQUE_LAYER;
    const IDENTITY: [f32; 16] = [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];
    fn quad(layer: u16, z: f32) -> Vec<TerrainVertex> {
        [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
            .into_iter()
            .map(|p| TerrainVertex::new([p[0], p[1], z], p, layer, 0xffffffff, 0))
            .collect()
    }
    fn section() -> SectionMesh {
        let mut s = SectionMesh::default();
        s.solid.extend(quad(OPAQUE_LAYER, 0.0));
        s.solid.extend(quad(0, 1.0));
        s.translucent.extend(quad(0, 2.0));
        s
    }
    fn world() -> World {
        World::new(Atlas::new(1, 1, 4, &[255; 4]).unwrap(), [255; 1024])
    }
    #[test]
    fn sections_are_sorted_and_preserve_cutout_flags() {
        let mut w = world();
        let near = (0, 0, 0);
        let far = (10, 0, 0);
        w.upload(near, &section()).unwrap();
        w.upload(far, &section()).unwrap();
        let plan = w
            .plan(
                &[far, near, far, (100, 0, 0)],
                [0., 0., 0.],
                -64,
                IDENTITY,
                None,
            )
            .unwrap();
        assert_eq!(
            plan.iter().map(|p| p.key).collect::<Vec<_>>(),
            [near, far, far, near]
        );
        assert_eq!(plan[0].draws[0].pass, Pass::Solid { alpha_cutoff: 0.0 });
        assert_eq!(plan[0].draws[1].pass, Pass::Solid { alpha_cutoff: 0.5 });
        assert_eq!(
            plan[2].draws[0].pass,
            Pass::Translucent { alpha_cutoff: 0.01 }
        );
        assert_eq!(plan[0].transform[13], -64.0);
        assert_eq!(plan[1].transform[12], 160.0);
    }
    #[test]
    fn lightmap_and_upload_are_transactional() {
        let mut w = world();
        w.upload((0, 0, 0), &section()).unwrap();
        let old = w.plan(&[(0, 0, 0)], [0.; 3], 0, IDENTITY, None).unwrap()[0].vertices[0].color;
        assert_eq!(old, [1.; 4]);
        let mut invalid = section();
        invalid.solid.pop();
        assert_eq!(
            w.upload((0, 0, 0), &invalid),
            Err(terrain::MeshError::IncompleteQuad)
        );
        assert!(w.contains((0, 0, 0)));
        assert!(w.set_lightmap([0; 1024]).unwrap());
        assert!(!w.set_lightmap([0; 1024]).unwrap());
        let plan = w.plan(&[(0, 0, 0)], [0.; 3], 0, IDENTITY, None).unwrap();
        assert_eq!(plan[0].vertices[0].color, [0., 0., 0., 1.]);
        w.remove((0, 0, 0));
        assert!(w.is_empty());
    }
    #[test]
    fn camera_relative_offsets_keep_precision_and_fog_origin() {
        let mut w = world();
        let key = (1_875_000, 1, -1_875_000);
        w.upload(key, &section()).unwrap();
        let fog = Fog {
            color: [1.; 4],
            ranges: [0., 1., 2., 3.],
            section_offset: [0.; 3],
        };
        let plan = w
            .plan(
                &[key],
                [30_000_000.25, -47.5, -30_000_000.75],
                -64,
                IDENTITY,
                Some(fog),
            )
            .unwrap();
        assert_eq!(&plan[0].transform[12..15], &[-0.25, -0.5, 0.75]);
        assert_eq!(plan[0].fog.unwrap().section_offset, [-0.25, -0.5, 0.75]);
        assert!(matches!(
            w.plan(&[key], [f64::NAN, 0., 0.], -64, IDENTITY, None),
            Err(PlanError::InvalidTransform)
        ));
    }
}
