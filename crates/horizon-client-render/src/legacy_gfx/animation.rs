//! CPU sampling of the existing SpriteAnimation clips. Retain shared frame
//! storage under a conservative source-byte cap; allocate only changed sprites.
use super::*;
use crate::{
    font::Patch,
    sprites::{FrameBlend, SpriteAnimation},
};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
enum Target {
    Terrain(usize),
    Hud([u32; 2]),
}
struct Sprite {
    clip: SpriteAnimation,
    target: Target,
    size: [u32; 2],
    shown: Option<FrameBlend>,
    padded: usize,
}
#[derive(Default)]
pub(super) struct Animations {
    sprites: Vec<Sprite>,
    failed: bool,
}
struct Work {
    terrain: Vec<(usize, Vec<u8>)>,
    hud: Vec<Patch>,
    shown: Vec<(usize, FrameBlend)>,
    terrain_bytes: usize,
}
fn bytes(size: [u32; 2]) -> Result<usize, String> {
    horizon_render_api::resource_contract::image_bytes(size[0], size[1]).map_err(diagnostic)
}
impl Animations {
    fn new(
        terrain: &[SpriteAnimation],
        hud: &[(crate::hud::Rect, SpriteAnimation)],
        atlas: &AtlasLayout,
        layout: Layout,
        limits: Limits,
    ) -> Result<Self, String> {
        if terrain
            .len()
            .checked_add(hud.len())
            .is_none_or(|n| n > limits.animation_sprites)
        {
            return Err("animation sprite count budget exceeded".into());
        }
        let mut result = Self::default();
        let mut layers = BTreeSet::new();
        for clip in terrain {
            if !layers.insert(clip.layer) {
                return Err("duplicate terrain animation layer".into());
            }
            let place = atlas
                .sprites()
                .get(clip.layer as usize)
                .ok_or("animation layer missing from atlas")?;
            let size = [place.width, place.height];
            let border = atlas
                .padding()
                .checked_mul(2)
                .ok_or("animation padding overflow")?;
            let padded = bytes([
                size[0]
                    .checked_add(border)
                    .ok_or("animation size overflow")?,
                size[1]
                    .checked_add(border)
                    .ok_or("animation size overflow")?,
            ])?;
            result.sprites.push(Sprite {
                clip: clip.clone(),
                target: Target::Terrain(clip.layer as usize),
                size,
                shown: None,
                padded,
            });
        }
        let mut rectangles: Vec<[u32; 4]> = Vec::new();
        for (rect, clip) in hud {
            if [rect.u0, rect.v0, rect.u1, rect.v1]
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                || rect.u0 >= rect.u1
                || rect.v0 >= rect.v1
            {
                return Err("invalid animated HUD UV rectangle".into());
            }
            let dims = layout.size();
            let size = {
                let (w, h) = rect.texels((dims[0], dims[1]));
                [w, h]
            };
            let at = [
                (rect.u0 * dims[0] as f32).round() as u32,
                (rect.v0 * dims[1] as f32).round() as u32,
            ];
            layout.affected(at, size).map_err(diagnostic)?;
            bytes(size)?;
            for &[x, y, w, h] in &rectangles {
                if at[0] < x + w && x < at[0] + size[0] && at[1] < y + h && y < at[1] + size[1] {
                    return Err("overlapping animated HUD rectangles".into());
                }
            }
            rectangles.push([at[0], at[1], size[0], size[1]]);
            result.sprites.push(Sprite {
                clip: clip.clone(),
                target: Target::Hud(at),
                size,
                shown: None,
                padded: 0,
            });
        }
        let mut source = 0usize;
        let mut frames = 0usize;
        for sprite in &result.sprites {
            frames = frames
                .checked_add(sprite.clip.frame_count())
                .filter(|n| *n <= limits.animation_frames)
                .ok_or("animation frame count budget exceeded")?;
            let mut expected = None;
            if sprite.clip.frame_count() == 0 {
                return Err("animation has no frames".into());
            }
            for frame in 0..sprite.clip.frame_count() {
                let len = sprite.clip.pixels(frame).len();
                let edge = (len / 4).isqrt();
                if len == 0
                    || edge.checked_mul(edge).and_then(|n| n.checked_mul(4)) != Some(len)
                    || expected.is_some_and(|n| n != len)
                {
                    return Err("animation frames must be equal RGBA squares".into());
                }
                expected = Some(len);
                source = source
                    .checked_add(len)
                    .filter(|n| *n <= limits.animation_source_bytes)
                    .ok_or("animation shared source-byte budget exceeded")?;
            }
        }
        Ok(result)
    }
    pub(super) fn ready(&self) -> Result<(), String> {
        if self.failed {
            return Err("animation upload incomplete; retry updates before drawing".into());
        }
        Ok(())
    }
    fn prepare(&self, tick: u64, layout: Layout, limit: usize) -> Result<Work, String> {
        let mut states = Vec::new();
        let mut retained = 0usize;
        let mut terrain_bytes = 0usize;
        let mut scratch = 0usize;
        let mut pages = BTreeSet::new();
        for (i, sprite) in self.sprites.iter().enumerate() {
            let blend = sprite.clip.blend(tick);
            if !self.failed && sprite.shown == Some(blend) {
                continue;
            }
            let size = bytes(sprite.size)?;
            retained = retained
                .checked_add(size)
                .ok_or("animation staging overflow")?;
            scratch = scratch.max(sprite.padded);
            match sprite.target {
                Target::Terrain(_) => {
                    terrain_bytes = terrain_bytes
                        .checked_add(size)
                        .ok_or("animation staging overflow")?
                }
                Target::Hud(at) => {
                    pages.extend(layout.affected(at, sprite.size).map_err(diagnostic)?)
                }
            }
            states.push((i, blend));
        }
        let decoded = pages
            .len()
            .checked_mul(layout.page_bytes())
            .ok_or("animation HUD page overflow")?;
        if retained
            .checked_add(scratch.max(decoded))
            .is_none_or(|n| n > limit)
        {
            return Err("animation staging/decoded-page budget exceeded".into());
        }
        let mut work = Work {
            terrain: Vec::new(),
            hud: Vec::new(),
            shown: states,
            terrain_bytes,
        };
        for &(i, blend) in &work.shown {
            let sprite = &self.sprites[i];
            let rgba = sample(&sprite.clip, blend, sprite.size)?;
            match sprite.target {
                Target::Terrain(layer) => work.terrain.push((layer, rgba)),
                Target::Hud([x, y]) => work.hud.push(Patch {
                    x,
                    y,
                    width: sprite.size[0],
                    height: sprite.size[1],
                    rgba,
                }),
            }
        }
        Ok(work)
    }
}
fn sample(
    clip: &SpriteAnimation,
    blend: FrameBlend,
    [width, height]: [u32; 2],
) -> Result<Vec<u8>, String> {
    let current = clip.pixels(blend.frame);
    let next = clip.pixels(blend.next);
    let edge = (current.len() / 4).isqrt();
    let mut pixels = Vec::new();
    let len = bytes([width, height])?;
    pixels
        .try_reserve_exact(len)
        .map_err(|_| "animation allocation failed")?;
    for y in 0..height as usize {
        for x in 0..width as usize {
            let from = (y * edge / height as usize * edge + x * edge / width as usize) * 4;
            for c in 0..4 {
                let (a, b) = (u32::from(current[from + c]), u32::from(next[from + c]));
                pixels.push(((a * (1000 - blend.millis) + b * blend.millis + 500) / 1000) as u8);
            }
        }
    }
    Ok(pixels)
}
impl TerrainHudRenderer {
    pub(super) fn install_animations(
        &mut self,
        assets: &crate::assets::RenderAssets,
        hud: &HudAtlas,
    ) -> Result<(), String> {
        let state = Animations::new(
            &assets.animations,
            &hud.sprite_animations,
            self.world.atlas(),
            self.hud.layout(),
            self.limits,
        )?;
        self.animations = state;
        Ok(())
    }
    /// Explicit owner clock, using the same clip sampling as modern. Stale bundle
    /// commands are rejected before preparation/mutation. After a partial upload
    /// failure, draw is blocked until a successful full retry restores coherence.
    pub fn advance_animations(
        &mut self,
        generation: horizon_render_api::render_contract::ResourceGeneration,
        tick: u64,
    ) -> Result<usize, String> {
        if self.resource_generation != Some(generation) {
            return Err("stale or untracked animation resource generation".into());
        }
        let work = self.animations.prepare(
            tick,
            self.hud.layout(),
            self.limits.animation_staging_bytes,
        )?;
        let count = work.shown.len();
        self.animations.failed = true;
        for (layer, pixels) in &work.terrain {
            self.world
                .update_sprite(*layer, pixels, self.limits.animation_staging_bytes)
                .map_err(diagnostic)?;
        }
        if !work.hud.is_empty() {
            let remaining = self.limits.animation_staging_bytes - work.terrain_bytes;
            let changed = self.hud.patch(&work.hud, remaining)?;
            self.cache.invalidate(&changed);
        }
        for (i, state) in work.shown {
            self.animations.sprites[i].shown = Some(state);
        }
        self.animations.failed = false;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clips_interpolate_resize_bound_work_and_force_full_retry() {
        let layout = horizon_render_api::atlas::pack_atlas(
            &[horizon_render_api::atlas::SpriteSize {
                width: 2,
                height: 2,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let hud = Layout::new([4, 4], 4, 4).unwrap();
        let clip = crate::sprites::test_frames(
            0,
            vec![vec![0, 0, 0, 255], vec![200, 100, 0, 255]],
            r#"{"frametime":4,"interpolate":true}"#,
        );
        let mut state =
            Animations::new(&[clip.clone()], &[], &layout, hud, Limits::default()).unwrap();
        let work = state.prepare(2, hud, 80).unwrap();
        assert_eq!(work.terrain[0].1, [100, 50, 0, 255].repeat(4));
        assert!(state.prepare(2, hud, 79).is_err()); // 16 sampled + 64 padded
        state.sprites[0].shown = Some(clip.blend(2));
        assert!(state.prepare(2, hud, 80).unwrap().shown.is_empty());
        state.failed = true;
        assert!(state.ready().is_err());
        assert_eq!(state.prepare(2, hud, 80).unwrap().shown.len(), 1);
        assert!(
            Animations::new(
                &[clip.clone(), clip.clone()],
                &[],
                &layout,
                hud,
                Limits::default()
            )
            .is_err()
        );
        assert!(
            Animations::new(
                &[clip],
                &[],
                &layout,
                hud,
                Limits {
                    animation_source_bytes: 7,
                    ..Limits::default()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn hud_dirty_pages_share_budget_with_terrain_and_inputs_are_checked() {
        use horizon_render_api::atlas::{SpriteSize, pack_atlas};
        let atlas = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let layout = Layout::new([4, 4], 4, 4).unwrap();
        let rect = crate::hud::Rect {
            u0: 0.,
            v0: 0.,
            u1: 1.,
            v1: 1.,
            w: 4,
            h: 4,
        };
        let clip = crate::sprites::test_frames(
            0,
            vec![vec![0, 50, 100, 150], vec![100, 150, 200, 250]],
            r#"{"frametime":4,"interpolate":true}"#,
        );
        let state = Animations::new(
            std::slice::from_ref(&clip),
            &[(rect, clip.clone())],
            &atlas,
            layout,
            Limits::default(),
        )
        .unwrap();
        // 4 terrain + 64 HUD sampled pixels + four decoded 64-byte pages.
        let work = state.prepare(2, layout, 324).unwrap();
        assert_eq!(work.terrain_bytes, 4);
        assert_eq!(work.hud[0].rgba, [50, 100, 150, 200].repeat(16));
        assert!(state.prepare(2, layout, 323).is_err());
        assert!(state.ready().is_ok()); // preflight failure did not poison the owner
        for limits in [
            Limits {
                animation_sprites: 0,
                ..Limits::default()
            },
            Limits {
                animation_frames: 1,
                ..Limits::default()
            },
        ] {
            assert!(
                Animations::new(std::slice::from_ref(&clip), &[], &atlas, layout, limits).is_err()
            );
        }
        let unknown =
            crate::sprites::test_frames(1, vec![vec![0; 4], vec![1; 4]], r#"{"frametime":4}"#);
        assert!(Animations::new(&[unknown], &[], &atlas, layout, Limits::default()).is_err());
        for frames in [
            vec![vec![], vec![]],
            vec![vec![0; 3], vec![0; 3]],
            vec![vec![0; 4], vec![0; 16]],
        ] {
            let malformed = crate::sprites::test_frames(0, frames, r#"{"frametime":4}"#);
            assert!(Animations::new(&[malformed], &[], &atlas, layout, Limits::default()).is_err());
        }
        for invalid in [
            crate::hud::Rect {
                u0: f32::NAN,
                ..rect
            },
            crate::hud::Rect { u1: 2., ..rect },
            crate::hud::Rect { u1: 0., ..rect },
        ] {
            assert!(
                Animations::new(
                    &[],
                    &[(invalid, clip.clone())],
                    &atlas,
                    layout,
                    Limits::default()
                )
                .is_err()
            );
        }
        assert!(
            Animations::new(
                &[],
                &[(rect, clip.clone()), (rect, clip)],
                &atlas,
                layout,
                Limits::default()
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::{
        legacy_gfx::ResourceLimits,
        mesher::{SectionMesh, TerrainVertex},
        renderer_facade::RendererFacade,
    };
    use horizon_render_api::{
        IDENTITY, render_contract::ResourceGeneration, resource_contract::TerrainAtlasLimits,
    };

    #[test]
    #[ignore = "requires OSMesa and explicit GL2 software-driver environment"]
    fn native_animations_preserve_meshes_reject_stale_ticks_and_recover_partial_updates() {
        let mut assets = super::super::test_assets();
        assets.animations.push(crate::sprites::test_frames(
            0,
            vec![vec![200, 0, 0, 255], vec![0, 0, 200, 255]],
            r#"{"frametime":4,"interpolate":true}"#,
        ));
        let mut atlas = HudAtlas::boot();
        atlas.width = 4;
        atlas.height = 4;
        atlas.pixels = vec![255; 64];
        let rect = crate::hud::Rect {
            u0: 0.,
            v0: 0.,
            u1: 0.5,
            v1: 1.,
            w: 2,
            h: 4,
        };
        atlas.sprites.insert("horizon/white".into(), rect);
        atlas.sprite_animations.push((
            rect,
            crate::sprites::test_frames(
                0,
                vec![vec![255; 16], [0, 0, 200, 255].repeat(4)],
                r#"{"frametime":4,"interpolate":true}"#,
            ),
        ));
        let generation = ResourceGeneration(77);
        let mut renderer = TerrainHudRenderer::stage_assets(
            Context::new("osmesa", [64; 2]).unwrap(),
            &assets,
            &atlas,
            generation,
            &[255; 1024],
            Limits {
                hud_page_size: 4,
                ..Limits::default()
            },
            ResourceLimits {
                terrain: TerrainAtlasLimits {
                    page_size: 8,
                    max_pages: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap()
        .commit()
        .unwrap();
        let mut mesh = SectionMesh::default();
        for (x0, x1, layer) in [(-1., 0., 0x8000), (0., 1., 0x8001)] {
            mesh.solid.extend(
                [[x0, -1., 0.], [x1, -1., 0.], [x1, 1., 0.], [x0, 1., 0.]]
                    .map(|p| TerrainVertex::new(p, [0.5; 2], layer, u32::MAX, 0xf0f0)),
            );
        }
        renderer.upload_section([0; 3], &mesh).unwrap();
        let mut hud = HudBatch::new([64; 2]);
        hud.rect_uv(0., 0., 16., 16., rect, u32::MAX);
        let scene = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[[0; 3]],
            translucent: &[],
        };
        // Warm the HUD cache before the first update, then exercise invalidation.
        renderer.render(&scene, &hud).unwrap();
        for (tick, terrain, gui) in [
            (0, [200, 0, 0, 255], [255; 4]),
            (2, [100, 0, 100, 255], [128, 128, 228, 255]),
            (4, [0, 0, 200, 255], [0, 0, 200, 255]),
            (8, [200, 0, 0, 255], [255; 4]),
        ] {
            let mut facade = RendererFacade::legacy(
                &mut renderer,
                frame::VisibleSections::default(),
                frame::Options::default(),
            );
            assert_eq!(facade.advance_animations(generation, tick).unwrap(), 2);
            assert_eq!(facade.advance_animations(generation, tick).unwrap(), 0);
            drop(facade);
            renderer.render(&scene, &hud).unwrap();
            assert_eq!(renderer.read_rgba([16, 32, 1, 1]).unwrap(), terrain);
            assert_eq!(
                renderer.read_rgba([48, 32, 1, 1]).unwrap(),
                [0, 255, 0, 255]
            );
            assert_eq!(renderer.read_rgba([8, 56, 1, 1]).unwrap(), gui);
            assert_eq!(renderer.resident_sections(), 1);
        }
        let before = renderer.read_rgba([0, 0, 64, 64]).unwrap();
        assert!(
            renderer
                .advance_animations(ResourceGeneration(23), 4)
                .is_err()
        );
        renderer.render(&scene, &hud).unwrap();
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), before);
        // A real HUD backing-budget failure occurs AFTER terrain uploaded. Its
        // initial pixels are identical, so no artificial state mismatch is needed.
        let layout = renderer.hud.layout();
        let tight = (1..1024)
            .find_map(|limit| Backing::new(layout, &atlas.pixels, limit).ok())
            .unwrap();
        let healthy = std::mem::replace(&mut renderer.hud, tight);
        let error = renderer.advance_animations(generation, 4).unwrap_err();
        assert!(error.contains("compressed HUD patch budget"), "{error}");
        assert!(renderer.render(&scene, &hud).is_err());
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), before);
        assert!(
            renderer
                .advance_animations(ResourceGeneration(23), 0)
                .is_err()
        );
        assert!(renderer.render(&scene, &hud).is_err());
        renderer.hud = healthy;
        // The old shown state already equals tick 0: retry MUST force all sprites.
        assert_eq!(renderer.advance_animations(generation, 0).unwrap(), 2);
        renderer.render(&scene, &hud).unwrap();
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), before);
        assert_eq!(renderer.resident_sections(), 1);
    }
}
