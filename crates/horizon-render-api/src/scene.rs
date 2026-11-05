//! Resident section resources shared by game adapters and render-backend tests.
//! No world generation, visibility decisions or translucent sorting live here.
use crate::{
    Draw, Fog, Frame, Image, MeshId, Pass, RenderBackend, TextureId, atlas::AtlasLayout,
    terrain::Run,
};
use std::collections::HashMap;

/// X/Z section coordinates, Y relative to the scene's configured minimum.
/// The default minimum is zero (so existing absolute-section probes agree).
pub type SectionKey = [i32; 3];
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Solid,
    /// Opaque/cutout terrain after a separately cleared background pass.
    SolidLoad,
    Translucent,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SceneError<E> {
    InvalidAtlas,
    Atlas(crate::atlas::AtlasError),
    InvalidLightmap,
    InvalidRuns,
    SectionLimit,
    MissingSection(SectionKey),
    InvalidOrigin,
    InvalidLayout,
    Backend(E),
}

/// Caller-provided visible order is authoritative. Translucent quads inside each
/// section must already have been camera-sorted before upload. Section ordering
/// alone is not an exact global translucent sort.
pub struct SceneFrame<'a> {
    pub view_projection: [f32; 16],
    pub camera: [f64; 3],
    pub clear: [f32; 4],
    pub fog: Option<Fog>,
    pub solid: &'a [SectionKey],
    pub translucent: &'a [SectionKey],
}

struct ResidentRun {
    mesh: MeshId,
    page: usize,
    pass: Pass,
}
struct Section {
    solid: Vec<ResidentRun>,
    translucent: Vec<ResidentRun>,
}

/// Owns its backend (or an exclusive backend reference) and atlas layout.
/// It can be stored in a game without self-referential lifetime tricks. Failed replacements leave the previous section usable.
/// Old resources are reclaimed before the next upload/render, with failures
/// retained for retry rather than silently forgetting their IDs.
pub struct WorldScene<B: RenderBackend> {
    backend: B,
    atlas: AtlasLayout,
    pages: Vec<TextureId>,
    lightmap: Option<TextureId>,
    sections: HashMap<SectionKey, Section>,
    retired: Vec<MeshId>,
    max_sections: usize,
    min_y: i32,
}
impl<B: RenderBackend> WorldScene<B> {
    pub fn new(
        backend: B,
        atlas: AtlasLayout,
        pages: &[&[u8]],
        lightmap: &[u8],
        max_sections: usize,
    ) -> Result<Self, SceneError<B::Error>> {
        let side = atlas.page_size();
        let bytes = (side as usize)
            .checked_mul(side as usize)
            .and_then(|n| n.checked_mul(4));
        if pages.len() != atlas.page_count() || pages.iter().any(|p| Some(p.len()) != bytes) {
            return Err(SceneError::InvalidAtlas);
        }
        Self::upload_pages(backend, atlas, lightmap, max_sections, |index| {
            Ok(pages[index])
        })
    }

    /// Prepare/upload exactly one page at a time. A failed preparation/upload
    /// drops the partial owner, reclaiming all previously created textures.
    pub fn new_paged(
        backend: B,
        atlas: AtlasLayout,
        lightmap: &[u8],
        max_sections: usize,
        page: impl FnMut(usize) -> Result<Vec<u8>, crate::atlas::AtlasError>,
    ) -> Result<Self, SceneError<B::Error>> {
        Self::upload_pages(backend, atlas, lightmap, max_sections, page)
    }

    fn upload_pages<P: AsRef<[u8]>>(
        backend: B,
        atlas: AtlasLayout,
        lightmap: &[u8],
        max_sections: usize,
        mut page: impl FnMut(usize) -> Result<P, crate::atlas::AtlasError>,
    ) -> Result<Self, SceneError<B::Error>> {
        let side = atlas.page_size();
        let bytes = (side as usize)
            .checked_mul(side as usize)
            .and_then(|n| n.checked_mul(4));
        if lightmap.len() != 16 * 16 * 4 {
            return Err(SceneError::InvalidLightmap);
        }
        // Construct the owner before allocating: Drop also cleans up a partially
        // completed constructor if a later GPU allocation fails.
        let mut scene = Self {
            backend,
            atlas,
            pages: Vec::new(),
            lightmap: None,
            sections: HashMap::new(),
            retired: Vec::new(),
            max_sections,
            min_y: 0,
        };
        for index in 0..scene.atlas.page_count() {
            let rgba = page(index).map_err(SceneError::Atlas)?;
            let rgba = rgba.as_ref();
            if Some(rgba.len()) != bytes {
                return Err(SceneError::InvalidAtlas);
            }
            let id = scene
                .backend
                .create_texture(Image {
                    width: side,
                    height: side,
                    rgba,
                })
                .map_err(SceneError::Backend)?;
            scene.pages.push(id);
        }
        scene.lightmap = Some(
            scene
                .backend
                .create_texture(Image {
                    width: 16,
                    height: 16,
                    rgba: lightmap,
                })
                .map_err(SceneError::Backend)?,
        );
        Ok(scene)
    }

    /// Replace one stable layer's level-zero pixels and gutters in-place.
    /// Membership/VBOs/layout are unchanged. A backend failure is not a rollback
    /// guarantee: the owner must stop drawing until resources are recovered.
    pub fn update_sprite(
        &mut self,
        index: usize,
        rgba: &[u8],
        max_bytes: usize,
    ) -> Result<(), SceneError<B::Error>> {
        let patch = crate::atlas::sprite_patch(&self.atlas, index, rgba, max_bytes)
            .map_err(SceneError::Atlas)?;
        self.backend
            .update_texture(
                self.pages[patch.page],
                patch.at,
                Image {
                    width: patch.size[0],
                    height: patch.size[1],
                    rgba: &patch.rgba,
                },
            )
            .map_err(SceneError::Backend)
    }

    pub fn atlas(&self) -> &AtlasLayout {
        &self.atlas
    }
    /// Compose GUI/hand views on the same backend after the world view. Resource
    /// IDs remain backend-validated, including those not owned by this scene.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    pub fn has_section(&self, key: SectionKey) -> bool {
        self.sections.contains_key(&key)
    }

    /// A dimension change commits a new Y origin only after validation and
    /// previous garbage cleanup. Texture/atlas/lightmap resources survive.
    /// Removed meshes are retained for retry until the next upload/draw/cleanup.
    pub fn reset_sections(&mut self, min_y: i32) -> Result<(), SceneError<B::Error>> {
        if min_y % 16 != 0 {
            return Err(SceneError::InvalidLayout);
        }
        self.collect_garbage()?;
        for (_, section) in std::mem::take(&mut self.sections) {
            self.retire(section);
        }
        self.min_y = min_y;
        Ok(())
    }

    /// Unload only this X/Z column, regardless of its vertical span. No second
    /// resident index or unbounded temporary key list is allocated.
    pub fn unload_column(&mut self, column: [i32; 2]) -> Result<usize, SceneError<B::Error>> {
        self.collect_garbage()?;
        let before = self.sections.len();
        let retired = &mut self.retired;
        self.sections.retain(|key, section| {
            if [key[0], key[2]] != column {
                return true;
            }
            retired.extend(
                section
                    .solid
                    .drain(..)
                    .chain(section.translucent.drain(..))
                    .map(|run| run.mesh),
            );
            false
        });
        Ok(before - self.sections.len())
    }

    fn section_origin(&self, key: SectionKey) -> [f64; 3] {
        [
            f64::from(key[0]) * 16.,
            f64::from(key[1]) * 16. + f64::from(self.min_y),
            f64::from(key[2]) * 16.,
        ]
    }

    pub fn collect_garbage(&mut self) -> Result<(), SceneError<B::Error>> {
        while let Some(&id) = self.retired.last() {
            self.backend.remove_mesh(id).map_err(SceneError::Backend)?;
            self.retired.pop();
        }
        Ok(())
    }

    pub fn replace_section(
        &mut self,
        key: SectionKey,
        solid: &[Run],
        translucent: &[Run],
    ) -> Result<(), SceneError<B::Error>> {
        self.collect_garbage()?;
        if !self.sections.contains_key(&key) && self.sections.len() >= self.max_sections {
            return Err(SceneError::SectionLimit);
        }
        if solid
            .iter()
            .any(|r| !matches!(r.pass, Pass::Opaque | Pass::Cutout))
            || translucent.iter().any(|r| r.pass != Pass::Translucent)
            || solid
                .iter()
                .chain(translucent)
                .any(|r| r.page >= self.pages.len())
        {
            return Err(SceneError::InvalidRuns);
        }
        let mut staged: Vec<ResidentRun> = Vec::new();
        for run in solid.iter().chain(translucent) {
            let mesh = match self.backend.create_mesh(&run.vertices, &run.indices) {
                Ok(id) => id,
                Err(error) => {
                    self.retired.extend(staged.into_iter().map(|r| r.mesh));
                    return Err(SceneError::Backend(error));
                }
            };
            staged.push(ResidentRun {
                mesh,
                page: run.page,
                pass: run.pass,
            });
        }
        let translucent = staged.split_off(solid.len());
        let section = Section {
            solid: staged,
            translucent,
        };
        if let Some(old) = self.sections.insert(key, section) {
            self.retire(old);
        }
        Ok(())
    }

    fn retire(&mut self, section: Section) {
        self.retired.extend(
            section
                .solid
                .into_iter()
                .chain(section.translucent)
                .map(|r| r.mesh),
        );
    }

    pub fn unload_section(&mut self, key: SectionKey) -> Result<bool, SceneError<B::Error>> {
        self.collect_garbage()?;
        if let Some(section) = self.sections.remove(&key) {
            self.retire(section);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn update_lightmap(&mut self, rgba: &[u8]) -> Result<(), SceneError<B::Error>> {
        if rgba.len() != 16 * 16 * 4 {
            return Err(SceneError::InvalidLightmap);
        }
        self.backend
            .update_texture(
                self.lightmap.unwrap(),
                [0, 0],
                Image {
                    width: 16,
                    height: 16,
                    rgba,
                },
            )
            .map_err(SceneError::Backend)
    }

    pub fn lightmap(&self) -> TextureId {
        self.lightmap.unwrap()
    }
    /// Preflight all visible keys and camera-relative translations without
    /// clearing or drawing; background passes must not erase a rejected frame.
    pub fn validate_visible(&self, frame: &SceneFrame<'_>) -> Result<(), SceneError<B::Error>> {
        for key in frame.solid.iter().chain(frame.translucent) {
            if !self.sections.contains_key(key) {
                return Err(SceneError::MissingSection(*key));
            }
            if crate::relative_origin(self.section_origin(*key), frame.camera).is_none() {
                return Err(SceneError::InvalidOrigin);
            }
        }
        Ok(())
    }
    pub fn render_phase(
        &mut self,
        frame: &SceneFrame<'_>,
        phase: Phase,
    ) -> Result<(), SceneError<B::Error>> {
        self.render_selected(frame, Some(phase))
    }
    pub fn render(&mut self, frame: &SceneFrame<'_>) -> Result<(), SceneError<B::Error>> {
        self.render_selected(frame, None)
    }
    fn render_selected(
        &mut self,
        frame: &SceneFrame<'_>,
        phase: Option<Phase>,
    ) -> Result<(), SceneError<B::Error>> {
        self.validate_visible(frame)?;
        self.collect_garbage()?;
        let mut draws = Vec::new();
        for (keys, translucent) in [(frame.solid, false), (frame.translucent, true)] {
            for key in keys {
                let section = self
                    .sections
                    .get(key)
                    .ok_or(SceneError::MissingSection(*key))?;
                if phase.is_some_and(|p| (p == Phase::Translucent) != translucent) {
                    continue;
                }
                let runs = if translucent {
                    &section.translucent
                } else {
                    &section.solid
                };
                for run in runs {
                    draws.push(Draw {
                        mesh: run.mesh,
                        texture: self.pages[run.page],
                        lightmap: self.lightmap.unwrap(),
                        origin: self.section_origin(*key),
                        pass: run.pass,
                        scissor: None,
                    });
                }
            }
        }
        self.backend
            .render(&Frame {
                view_projection: frame.view_projection,
                camera: frame.camera,
                clear: if matches!(phase, Some(Phase::Translucent | Phase::SolidLoad)) {
                    crate::Clear::Load
                } else {
                    crate::Clear::ColorDepth(frame.clear)
                },
                fog: frame.fog,
                draws: &draws,
            })
            .map_err(SceneError::Backend)
    }

    pub fn read_rgba(&mut self, rect: [u32; 4]) -> Result<Vec<u8>, SceneError<B::Error>> {
        self.backend.read_rgba(rect).map_err(SceneError::Backend)
    }
}
impl<B: RenderBackend> Drop for WorldScene<B> {
    fn drop(&mut self) {
        for (_, section) in self.sections.drain() {
            for run in section.solid.into_iter().chain(section.translucent) {
                let _ = self.backend.remove_mesh(run.mesh);
            }
        }
        for id in self.retired.drain(..) {
            let _ = self.backend.remove_mesh(id);
        }
        for id in self.pages.drain(..).chain(self.lightmap.take()) {
            let _ = self.backend.remove_texture(id);
        }
        // Removal can fail on a lost context. The owning backend must still
        // reclaim its remaining resources on drop (the GL backend does).
    }
}

#[cfg(test)]
mod tests;
