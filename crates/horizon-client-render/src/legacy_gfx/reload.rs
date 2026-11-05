//! Synchronous same-context replacement. The exclusive borrow prevents active
//! GPU allocations/draws during staging; dropping the guard preserves the owner.
use super::*;
use crate::assets::RenderAssets;
use horizon_render_api::render_contract::ResourceGeneration;

fn generations(
    active: Option<ResourceGeneration>,
    expected: ResourceGeneration,
    next: ResourceGeneration,
) -> Result<(), String> {
    if active != Some(expected) {
        return Err("stale or untracked reload resource generation".into());
    }
    if next.0 <= expected.0 {
        return Err("native reload requires a newer full-bundle generation".into());
    }
    Ok(())
}
fn available(
    mut limits: Limits,
    free: horizon_legacy_render::Stats,
    animation: [usize; 3],
    backing: usize,
) -> Result<Limits, String> {
    limits.gpu = Budget {
        texture_bytes: free.texture_bytes,
        mesh_bytes: free.mesh_bytes,
    };
    limits.animation_sprites = limits
        .animation_sprites
        .checked_sub(animation[0])
        .ok_or("active animation target usage exceeds limit")?;
    limits.animation_frames = limits
        .animation_frames
        .checked_sub(animation[1])
        .ok_or("active animation frame usage exceeds limit")?;
    limits.animation_source_bytes = limits
        .animation_source_bytes
        .checked_sub(animation[2])
        .ok_or("active animation source usage exceeds limit")?;
    limits.hud_backing_bytes = limits
        .hud_backing_bytes
        .checked_sub(backing)
        .ok_or("active HUD backing usage exceeds limit")?;
    Ok(limits)
}

#[must_use = "commit the reload, or drop it to keep the active owner/font/screenshot state"]
pub struct StagedAssetReload<'owner, 'hud> {
    active: &'owner mut TerrainHudRenderer,
    replacement: TerrainHudRenderer,
    hud: &'hud HudAtlas,
    font_epoch: u64,
}
impl StagedAssetReload<'_, '_> {
    /// After all fallible validation/promotion, publish without allocation or
    /// fallible upload. The caller must rebuild text batches and world meshes
    /// for the new generation; no old UV-bearing section survives publication.
    pub fn commit(mut self) -> Result<ResourceGeneration, String> {
        if self.hud.font.epoch() != self.font_epoch {
            return Err("font changed during native reload staging; rebuild the snapshot".into());
        }
        let limits = self.active.limits;
        self.replacement.hud.set_limit(limits.hud_backing_bytes)?;
        self.replacement
            .world
            .backend_mut()
            .set_budget(limits.gpu)?;
        let free = self.replacement.world.backend_mut().remaining();
        let capacity = limits
            .hud_resident_pages
            .min(free.textures)
            .min(free.texture_bytes / self.replacement.hud.layout().page_bytes());
        self.replacement.cache =
            Cache::new(self.replacement.hud.layout(), capacity).map_err(diagnostic)?;
        self.replacement.hud_capacity = capacity;
        let generation = self
            .replacement
            .resource_generation
            .ok_or("untracked staged reload")?;
        // No further GPU allocation until the old owner is destroyed. Preserve
        // both screenshot slots (including an already completed result) exactly.
        self.replacement.limits = limits;
        std::mem::swap(
            &mut self.replacement.screenshots,
            &mut self.active.screenshots,
        );
        self.hud.font.reset_renderer_cache();
        let old = std::mem::replace(self.active, self.replacement);
        drop(old);
        Ok(generation)
    }
}
impl TerrainHudRenderer {
    /// Stage resources/animations at the requested authoritative tick on the
    /// existing GL context and window. All old payloads remain resident until
    /// commit; insufficient joint GPU/source/backing capacity is a real error,
    /// not eviction of the active scene. No modern device/PreparedTextures.
    pub fn stage_reload_assets<'owner, 'hud>(
        &'owner mut self,
        assets: &RenderAssets,
        hud: &'hud HudAtlas,
        expected: ResourceGeneration,
        next: ResourceGeneration,
        tick: u64,
        preparation: ResourceLimits,
    ) -> Result<StagedAssetReload<'owner, 'hud>, String> {
        generations(self.resource_generation, expected, next)?;
        let limits = available(
            self.limits,
            self.world.backend_mut().remaining(),
            self.animations.usage(),
            self.hud.compressed_bytes(),
        )?;
        let font_epoch = hud.font.epoch();
        let snapshot = AssetSnapshot::new(assets, hud, next, preparation.base_bytes)?;
        let backend = self.world.backend_mut().staging_replacement()?;
        let mut replacement = Self::from_resources_backend(
            backend,
            &snapshot.resources(),
            &self.lightmap,
            limits,
            preparation,
        )?;
        replacement.install_animations(assets, hud)?;
        replacement.advance_animations(next, tick)?;
        if let Some(layout) = self.game_layout {
            replacement.clear_sections(layout)?;
        }
        Ok(StagedAssetReload {
            active: self,
            replacement,
            hud,
            font_epoch,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn epochs_reject_foreign_untracked_reused_older_and_exhausted_generations() {
        let expected = ResourceGeneration(7);
        for (active, next) in [
            (None, 8),
            (Some(ResourceGeneration(6)), 8),
            (Some(expected), 7),
            (Some(expected), 6),
        ] {
            assert!(generations(active, expected, ResourceGeneration(next)).is_err());
        }
        assert!(generations(Some(expected), expected, ResourceGeneration(8)).is_ok());
        assert!(
            generations(
                Some(ResourceGeneration(u64::MAX)),
                ResourceGeneration(u64::MAX),
                ResourceGeneration(0)
            )
            .is_err()
        );
    }
    #[test]
    fn staging_charges_both_owners_source_metadata_backing_and_gpu_capacity() {
        let full = Limits::default();
        let free = horizon_legacy_render::Stats {
            texture_bytes: 1234,
            mesh_bytes: 5678,
            textures: 7,
            meshes: 8,
        };
        let limits = available(full, free, [10, 20, 30], 40).unwrap();
        assert_eq!(limits.gpu.texture_bytes, 1234);
        assert_eq!(limits.gpu.mesh_bytes, 5678);
        assert_eq!(limits.animation_sprites, full.animation_sprites - 10);
        assert_eq!(limits.animation_frames, full.animation_frames - 20);
        assert_eq!(
            limits.animation_source_bytes,
            full.animation_source_bytes - 30
        );
        assert_eq!(limits.hud_backing_bytes, full.hud_backing_bytes - 40);
        for (usage, backing) in [
            ([full.animation_sprites + 1, 0, 0], 0),
            ([0, full.animation_frames + 1, 0], 0),
            ([0, 0, full.animation_source_bytes + 1], 0),
            ([0; 3], full.hud_backing_bytes + 1),
        ] {
            assert!(available(full, free, usage, backing).is_err());
        }
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::{
        hud::Rect,
        mesher::{SectionMesh, TerrainVertex},
        sprites::test_frames,
    };
    use horizon_render_api::{
        IDENTITY, render_contract::ScreenshotRequest, resource_contract::TerrainAtlasLimits,
    };

    fn hud(size: [u32; 2], color: [u8; 4]) -> HudAtlas {
        let mut atlas = HudAtlas::boot();
        [atlas.width, atlas.height] = size;
        atlas.pixels = vec![255; (size[0] * size[1] * 4) as usize];
        let rect = Rect {
            u0: 0.,
            v0: 0.,
            u1: 1.,
            v1: 1.,
            w: size[0],
            h: size[1],
        };
        atlas.sprites.insert("horizon/white".into(), rect);
        atlas.sprite_animations.push((
            rect,
            test_frames(
                0,
                vec![vec![255; 4], color.to_vec()],
                r#"{"frametime":4,"interpolate":true}"#,
            ),
        ));
        atlas
    }
    fn assets(color: [u8; 4]) -> RenderAssets {
        let mut assets = super::super::test_assets();
        assets.animations.push(test_frames(
            0,
            vec![color.to_vec(), vec![0, 0, 200, 255]],
            r#"{"frametime":4,"interpolate":true}"#,
        ));
        assets
    }
    fn mesh() -> SectionMesh {
        let mut mesh = SectionMesh::default();
        mesh.solid.extend(
            [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
                .map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000, u32::MAX, 0xf0f0)),
        );
        mesh
    }
    fn batch(atlas: &HudAtlas) -> HudBatch {
        let mut batch = HudBatch::new([64; 2]);
        batch.rect_uv(0., 0., 16., 16., atlas.sprites["horizon/white"], u32::MAX);
        batch
    }
    #[test]
    #[ignore = "requires OSMesa and explicit GL2 software-driver environment"]
    fn same_context_reload_rolls_back_or_commits_epochs_meshes_animation_and_screenshot_slots() {
        let original = assets([200, 0, 0, 255]);
        let old_hud = hud([4, 4], [200, 0, 0, 255]);
        let next_assets = assets([0, 200, 0, 255]);
        let next_hud = hud([8, 4], [0, 0, 200, 255]);
        let preparation = ResourceLimits {
            terrain: TerrainAtlasLimits {
                page_size: 8,
                max_pages: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        let limits = Limits {
            gpu: Budget {
                texture_bytes: 6000,
                ..Budget::default()
            },
            hud_page_size: 4,
            animation_sprites: 8,
            animation_frames: 16,
            animation_source_bytes: 40,
            hud_backing_bytes: 4096,
            ..Limits::default()
        };
        let expected = ResourceGeneration(77);
        let next = ResourceGeneration(78);
        let mut renderer = TerrainHudRenderer::stage_assets(
            Context::new("osmesa", [64; 2]).unwrap(),
            &original,
            &old_hud,
            expected,
            &[255; 1024],
            limits,
            preparation,
        )
        .unwrap()
        .commit()
        .unwrap();
        renderer
            .clear_sections(crate::chunk::ColumnLayout::new(-64, 384).unwrap())
            .unwrap();
        renderer.upload_section([0; 3], &mesh()).unwrap();
        renderer.advance_animations(expected, 2).unwrap();
        let scene = SceneFrame {
            view_projection: IDENTITY,
            camera: [0., -64., 0.],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[[0; 3]],
            translucent: &[],
        };
        let old_batch = batch(&old_hud);
        renderer.render(&scene, &old_batch).unwrap();
        let image = renderer.read_rgba([0, 0, 64, 64]).unwrap();
        assert_eq!(
            renderer.read_rgba([32, 32, 1, 1]).unwrap(),
            [100, 0, 100, 255]
        );
        let context = renderer.world.backend_mut().context() as *const Context;
        let stats = renderer.world.backend_mut().stats();
        let remaining = renderer.world.backend_mut().remaining();
        assert!(
            renderer
                .world
                .backend_mut()
                .set_budget(Budget {
                    texture_bytes: stats.texture_bytes - 1,
                    ..limits.gpu
                })
                .is_err()
        );
        assert_eq!(renderer.world.backend_mut().remaining(), remaining);
        let old_epoch = old_hud.font.epoch();
        let epoch = next_hud.font.epoch();
        renderer
            .request_screenshot(ScreenshotRequest {
                path: "not-written-by-reload.png".into(),
                size: None,
                scaled: false,
            })
            .unwrap();
        renderer.screenshots.result = Some(Err("retained completion".into()));
        for (before, after) in [
            (ResourceGeneration(76), next),
            (expected, expected),
            (expected, ResourceGeneration(76)),
        ] {
            assert!(
                renderer
                    .stage_reload_assets(&next_assets, &next_hud, before, after, 2, preparation)
                    .is_err()
            );
        }
        // Each candidate fits the full limit alone, but not old+new together.
        assert!(
            renderer
                .stage_reload_assets(
                    &next_assets,
                    &next_hud,
                    expected,
                    next,
                    2,
                    ResourceLimits {
                        terrain: TerrainAtlasLimits {
                            page_size: 32,
                            ..preparation.terrain
                        },
                        ..preparation
                    }
                )
                .is_err()
        );
        let mut large_source = assets([0, 200, 0, 255]);
        large_source.animations[0] =
            test_frames(0, vec![vec![0; 16], vec![1; 16]], r#"{"frametime":4}"#);
        assert!(
            renderer
                .stage_reload_assets(&large_source, &next_hud, expected, next, 2, preparation)
                .is_err()
        );
        assert_eq!(renderer.world.backend_mut().stats(), stats);
        assert_eq!(next_hud.font.epoch(), epoch);
        // A fully prepared stage can be discarded without publication.
        {
            let mut stage = renderer
                .stage_reload_assets(&next_assets, &next_hud, expected, next, 2, preparation)
                .unwrap();
            assert_eq!(
                stage.replacement.world.backend_mut().context() as *const Context,
                context
            );
            let new = stage.replacement.world.backend_mut().stats();
            let free = stage.replacement.world.backend_mut().remaining();
            assert_eq!(
                stats.texture_bytes + new.texture_bytes + free.texture_bytes,
                limits.gpu.texture_bytes
            );
            assert_eq!(
                stats.mesh_bytes + new.mesh_bytes + free.mesh_bytes,
                limits.gpu.mesh_bytes
            );
            assert_eq!(stats.textures + new.textures + free.textures, 4096);
            assert_eq!(stats.meshes + new.meshes + free.meshes, 65536);
            assert!(stats.texture_bytes + new.texture_bytes <= limits.gpu.texture_bytes);
            assert!(stats.mesh_bytes + new.mesh_bytes <= limits.gpu.mesh_bytes);
            assert!(stats.textures + new.textures <= 4096);
            assert!(stats.meshes + new.meshes <= 65536);
            assert_eq!(next_hud.font.epoch(), epoch);
        }
        assert_eq!(renderer.world.backend_mut().stats(), stats);
        assert_eq!(renderer.resource_generation(), Some(expected));
        assert_eq!(renderer.resident_sections(), 1);
        renderer.render(&scene, &old_batch).unwrap();
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), image);
        assert_eq!(old_hud.font.epoch(), old_epoch);
        // A changed producer font invalidates commit, not the active owner.
        let stage = renderer
            .stage_reload_assets(&next_assets, &next_hud, expected, next, 2, preparation)
            .unwrap();
        next_hud.font.reset_cache();
        let changed = next_hud.font.epoch();
        assert!(stage.commit().is_err());
        assert_eq!(next_hud.font.epoch(), changed);
        assert_eq!(renderer.world.backend_mut().stats(), stats);
        let stage = renderer
            .stage_reload_assets(&next_assets, &next_hud, expected, next, 2, preparation)
            .unwrap();
        assert_eq!(stage.commit().unwrap(), next);
        assert_ne!(next_hud.font.epoch(), changed);
        assert_eq!(old_hud.font.epoch(), old_epoch);
        assert_eq!(renderer.resource_generation(), Some(next));
        assert_eq!(
            renderer.world.backend_mut().context() as *const Context,
            context
        );
        assert_eq!(renderer.size(), [64; 2]);
        assert_eq!(renderer.resident_sections(), 0);
        assert_eq!(renderer.hud_capacity, limits.hud_resident_pages);
        assert_eq!(
            renderer.screenshots.pending.as_ref().unwrap().path,
            std::path::Path::new("not-written-by-reload.png")
        );
        assert_eq!(
            renderer.screenshots.result,
            Some(Err("retained completion".into()))
        );
        assert!(renderer.advance_animations(expected, 2).is_err());
        assert!(renderer.render(&scene, &old_batch).is_err());
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), image);
        renderer.upload_section([0; 3], &mesh()).unwrap();
        let new_batch = batch(&next_hud);
        renderer.render(&scene, &new_batch).unwrap();
        assert_eq!(
            renderer.read_rgba([32, 32, 1, 1]).unwrap(),
            [0, 100, 100, 255]
        );
        assert_eq!(
            renderer.read_rgba([8, 56, 1, 1]).unwrap(),
            [128, 128, 228, 255]
        );
        let stable = renderer.world.backend_mut().stats();
        let mut generation = next;
        for number in 79..83 {
            let next = ResourceGeneration(number);
            crate::renderer_facade::RendererFacade::legacy(
                &mut renderer,
                frame::VisibleSections::default(),
                frame::Options::default(),
            )
            .reload_assets(&next_assets, &next_hud, generation, next, 2, preparation)
            .unwrap();
            renderer.upload_section([0; 3], &mesh()).unwrap();
            renderer.render(&scene, &new_batch).unwrap();
            assert_eq!(renderer.world.backend_mut().stats(), stable);
            assert_eq!(
                renderer.world.backend_mut().context() as *const Context,
                context
            );
            assert_eq!(
                renderer.read_rgba([32, 32, 1, 1]).unwrap(),
                [0, 100, 100, 255]
            );
            assert_eq!(renderer.limits.gpu.texture_bytes, limits.gpu.texture_bytes);
            assert_eq!(
                renderer.world.backend_mut().stats().texture_bytes
                    + renderer.world.backend_mut().remaining().texture_bytes,
                limits.gpu.texture_bytes
            );
            generation = next;
        }
    }
}
