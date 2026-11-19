//! What the native owner's resident sections hold besides their quads.
//!
//! The modern `gfx::Renderer` keeps a section's `SectionMesh::block_entities` beside its
//! terrain (`gfx/block_entity_positions.rs`) and the game asks that store which chests,
//! signs and pots to draw. The native owner kept only the quads, so `Game::entity_geometry`
//! found no membership to draw from on a native frame and every block entity vanished
//! (reported: "сундуки и decorated pot не рендерятся"). This is that same store, in the
//! same shape and with the same revisions, so the game's one producer body serves both.
use super::*;
use crate::{
    fast_hash::FxHashMap,
    gfx::{
        JoinedFrusta, SectionKey as GfxSectionKey, next_block_entity_revision,
        visible_block_entities, visible_block_entities_in_frustum,
    },
};
use std::time::Instant;

/// A section key in the shape `gfx`'s block-entity walk uses (`(x, section-y, z)`, with
/// the section's Y relative to the dimension's minimum, as `SectionMesh::block_entities`
/// already are).
fn gfx_key(key: SectionKey) -> GfxSectionKey {
    (key[0], key[1], key[2])
}
/// Resident block-entity membership and the two revision numbers `StaticSectionRevision`
/// pairs: the section mesh's (light alone moves it) and the membership's.
#[derive(Default)]
pub(super) struct Resident {
    blocks: FxHashMap<GfxSectionKey, Vec<([i32; 3], u32)>>,
    /// Changes whenever `blocks` does. Shared with the modern renderer's counter, so a
    /// cache handed a number from either owner never confuses the two.
    revision: u64,
    membership: FxHashMap<GfxSectionKey, u64>,
    sections: FxHashMap<GfxSectionKey, u64>,
    /// When each section was drawn first, for the chunk fade-in that holds its block
    /// entities back (`LevelExtractor.extractVisibleBlockEntities`).
    uploaded: FxHashMap<GfxSectionKey, Instant>,
    newest_upload: Option<Instant>,
}
impl Resident {
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }
    pub(super) fn section_revision(
        &self,
        section: SectionKey,
    ) -> Option<crate::block_entity_render::StaticSectionRevision> {
        let key = gfx_key(section);
        Some(crate::block_entity_render::StaticSectionRevision {
            mesh: *self.sections.get(&key)?,
            block_entities: *self.membership.get(&key)?,
        })
    }
    /// What goes with a new mesh for `key`, before its quads replace the old ones.
    /// A mesh is rebuilt whenever a section's contents or light change, so its own
    /// revision moves every time; the membership revision moves only when the list does.
    pub(super) fn set(&mut self, section: SectionKey, block_entities: &[([i32; 3], u32)]) {
        let key = gfx_key(section);
        let changed = if block_entities.is_empty() {
            self.blocks.remove(&key).is_some()
        } else {
            let old = self.blocks.insert(key, block_entities.to_vec());
            old.as_deref() != Some(block_entities)
        };
        if changed {
            self.revision = next_block_entity_revision();
            self.membership.insert(key, self.revision);
        }
        if !block_entities.is_empty() || self.membership.contains_key(&key) {
            // A tombstone for a section that had block entities: light alone can change
            // with an unchanged list, and that invalidates only this section's cache.
            self.sections.insert(key, next_block_entity_revision());
        }
        if !self.uploaded.contains_key(&key) {
            let now = Instant::now();
            self.uploaded.insert(key, now);
            self.newest_upload = Some(now);
        }
    }
    /// A section that left the world keeps no membership, no revision and no fade clock.
    pub(super) fn remove(&mut self, section: SectionKey) {
        let key = gfx_key(section);
        if self.blocks.remove(&key).is_some() {
            self.revision = next_block_entity_revision();
        }
        self.membership.remove(&key);
        self.sections.remove(&key);
        self.uploaded.remove(&key);
    }
    pub(super) fn remove_column(&mut self, x: i32, z: i32) {
        let gone: Vec<GfxSectionKey> = self
            .blocks
            .keys()
            .chain(self.membership.keys())
            .chain(self.sections.keys())
            .chain(self.uploaded.keys())
            .filter(|k| k.0 == x && k.2 == z)
            .copied()
            .collect();
        for key in gone {
            if self.blocks.remove(&key).is_some() {
                self.revision = next_block_entity_revision();
            }
            self.membership.remove(&key);
            self.sections.remove(&key);
            self.uploaded.remove(&key);
        }
    }
    pub(super) fn clear(&mut self) {
        if !self.blocks.is_empty() {
            self.revision = next_block_entity_revision();
        }
        self.blocks.clear();
        self.membership.clear();
        self.sections.clear();
        self.uploaded.clear();
        self.newest_upload = None;
    }
}

/// The resident membership as the game sees it, through `RendererOwner::block_entities`:
/// borrowed so one producer body can walk either owner's store without copying it.
#[derive(Clone, Copy)]
pub struct NativeBlockEntities<'a> {
    pub(super) resident: &'a Resident,
    pub(super) min_y: i32,
}
impl<'a> NativeBlockEntities<'a> {
    /// The same walk `gfx::Renderer::visible_block_entities_in_frustum` does: 64 blocks
    /// (256 for an End gateway), then the frustum with a one-block margin, sections wholly
    /// inside skipping the per-block test.
    pub fn visible_in_frustum(
        &self,
        fade_seconds: f32,
        camera: glam::DVec3,
        frustum: Option<JoinedFrusta>,
        keep_section: impl Fn(u32) -> bool + 'a,
        keep_long_range: impl Fn(u32) -> bool + 'a,
    ) -> impl Iterator<Item = (&'a ([i32; 3], u32), bool)> + 'a {
        visible_block_entities_in_frustum(
            &self.resident.blocks,
            &self.resident.uploaded,
            self.resident.newest_upload,
            fade_seconds,
            Instant::now(),
            self.min_y,
            camera,
            frustum,
            keep_section,
            keep_long_range,
        )
    }
    /// `gfx::Renderer::visible_block_entities`: the membership filtered only by the fade-in.
    pub fn visible(&self, fade_seconds: f32) -> impl Iterator<Item = &'a ([i32; 3], u32)> + '_ {
        visible_block_entities(
            &self.resident.blocks,
            &self.resident.uploaded,
            self.resident.newest_upload,
            fade_seconds,
            Instant::now(),
        )
    }
    pub fn revision(&self) -> u64 {
        self.resident.revision()
    }
    pub fn section_revision(
        &self,
        section: [i32; 3],
    ) -> Option<crate::block_entity_render::StaticSectionRevision> {
        self.resident.section_revision(section)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn count(resident: &Resident) -> usize {
        NativeBlockEntities {
            resident,
            min_y: -64,
        }
        .visible_in_frustum(0.0, glam::DVec3::ZERO, None, |_| false, |_| false)
        .count()
    }
    #[test]
    fn a_new_mesh_moves_the_section_revision_while_an_unchanged_list_keeps_membership() {
        let mut resident = Resident::default();
        let key = [0, 4, 0];
        let chest = ([3, 70, 3], 11);
        resident.set(key, std::slice::from_ref(&chest));
        let first = resident.section_revision(key).unwrap();
        assert_eq!(resident.revision(), first.block_entities);
        assert_eq!(count(&resident), 1);
        // A light-only remesh: same blocks, new mesh revision, same membership.
        resident.set(key, std::slice::from_ref(&chest));
        let second = resident.section_revision(key).unwrap();
        assert_ne!(second.mesh, first.mesh);
        assert_eq!(second.block_entities, first.block_entities);
        assert_eq!(resident.revision(), first.block_entities);
        // The chest breaks: membership moves and the rendered list empties.
        resident.set(key, &[]);
        let third = resident.section_revision(key).unwrap();
        assert_ne!(third.block_entities, second.block_entities);
        assert_eq!(resident.revision(), third.block_entities);
        assert_eq!(count(&resident), 0);
        // Unloading forgets the section entirely: no revision, nothing to draw.
        resident.set(key, std::slice::from_ref(&chest));
        resident.remove(key);
        assert!(resident.section_revision(key).is_none());
        assert_eq!(count(&resident), 0);
    }
}
