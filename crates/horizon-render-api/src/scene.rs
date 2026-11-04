//! Resident section resources shared by game adapters and render-backend tests.
//! No world generation, visibility decisions or translucent sorting live here.
use crate::{
    Draw, Fog, Frame, Image, MeshId, Pass, RenderBackend, TextureId, atlas::AtlasLayout,
    terrain::Run,
};
use std::collections::HashMap;

pub type SectionKey = [i32; 3];

#[derive(Debug, PartialEq, Eq)]
pub enum SceneError<E> {
    InvalidAtlas,
    InvalidLightmap,
    InvalidRuns,
    SectionLimit,
    MissingSection(SectionKey),
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

/// Owns only resources it creates; borrowing the backend prevents it being
/// dropped first. Failed replacements leave the previous section usable.
/// Old resources are reclaimed before the next upload/render, with failures
/// retained for retry rather than silently forgetting their IDs.
pub struct WorldScene<'a, B: RenderBackend> {
    backend: &'a mut B,
    atlas: &'a AtlasLayout,
    pages: Vec<TextureId>,
    lightmap: Option<TextureId>,
    sections: HashMap<SectionKey, Section>,
    retired: Vec<MeshId>,
    max_sections: usize,
}
impl<'a, B: RenderBackend> WorldScene<'a, B> {
    pub fn new(
        backend: &'a mut B,
        atlas: &'a AtlasLayout,
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
        };
        for rgba in pages {
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

    pub fn atlas(&self) -> &AtlasLayout {
        self.atlas
    }
    pub fn section_count(&self) -> usize {
        self.sections.len()
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

    pub fn render(&mut self, frame: &SceneFrame<'_>) -> Result<(), SceneError<B::Error>> {
        self.collect_garbage()?;
        let mut draws = Vec::new();
        for (keys, translucent) in [(frame.solid, false), (frame.translucent, true)] {
            for key in keys {
                let section = self
                    .sections
                    .get(key)
                    .ok_or(SceneError::MissingSection(*key))?;
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
                        origin: key.map(|v| f64::from(v) * 16.),
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
                clear: crate::Clear::ColorDepth(frame.clear),
                fog: frame.fog,
                draws: &draws,
            })
            .map_err(SceneError::Backend)
    }

    pub fn read_rgba(&mut self, rect: [u32; 4]) -> Result<Vec<u8>, SceneError<B::Error>> {
        self.backend.read_rgba(rect).map_err(SceneError::Backend)
    }
}
impl<B: RenderBackend> Drop for WorldScene<'_, B> {
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
