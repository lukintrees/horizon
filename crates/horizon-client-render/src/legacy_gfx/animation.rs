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
    /// Terrain layers: the alpha scale each level keeps the whole animation
    /// sheet's cutout coverage with, measured once from the stacked frames the
    /// way `SpriteContents.increaseMipLevel` does. Empty for a HUD sprite, an
    /// opaque layer, or a sprite too small for the page's chain.
    scales: Vec<f32>,
    /// Whether [`Self::scales`] was measured yet: a terrain layer pays for the
    /// measurement the first time one of its frames is uploaded, not per tick.
    measured: bool,
    /// Terrain layers: every frame resampled to [`Self::size`] and prepared the
    /// way the page's chain prepares its base (`mipmap::prepare_base`), so a
    /// tick only interpolates two of them instead of resampling and solidifying
    /// the frame it shows. Empty for a HUD sprite.
    frames: Vec<Vec<u8>>,
    /// The chain settings the scales were measured with: the sheet's, not one
    /// frame's, so a frame with no transparent texel still gets a cutout chain
    /// when the animation it belongs to has one.
    settings: crate::mipmap::Settings,
}
#[derive(Default)]
pub(super) struct Animations {
    sprites: Vec<Sprite>,
    failed: bool,
    source_bytes: usize,
    source_frames: usize,
}
struct Work {
    /// One layer's changed frame, its chain settings, and the alpha scale of
    /// each level below it.
    terrain: Vec<TerrainWork>,
    hud: Vec<Patch>,
    shown: Vec<(usize, FrameBlend)>,
    terrain_bytes: usize,
}
struct TerrainWork {
    layer: usize,
    rgba: Vec<u8>,
    settings: crate::mipmap::Settings,
    scales: Vec<f32>,
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
                scales: Vec::new(),
                measured: false,
                frames: Vec::new(),
                settings: crate::mipmap::Settings::default(),
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
                scales: Vec::new(),
                measured: true,
                frames: Vec::new(),
                settings: crate::mipmap::Settings::default(),
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
        result.source_bytes = source;
        result.source_frames = frames;
        Ok(result)
    }
    pub(super) fn usage(&self) -> [usize; 3] {
        [self.sprites.len(), self.source_frames, self.source_bytes]
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
            return Err(format!(
                "animation staging/decoded-page budget exceeded: {} retained + {} scratch/pages over {limit}",
                retained,
                scratch.max(decoded)
            ));
        }
        let mut work = Work {
            terrain: Vec::new(),
            hud: Vec::new(),
            shown: states,
            terrain_bytes,
        };
        for &(i, blend) in &work.shown {
            let sprite = &self.sprites[i];
            match sprite.target {
                Target::Terrain(layer) => work.terrain.push(TerrainWork {
                    layer,
                    rgba: if sprite.frames.is_empty() {
                        sample(&sprite.clip, blend, sprite.size)?
                    } else {
                        interpolate(&sprite.frames, blend, sprite.size)?
                    },
                    settings: sprite.settings,
                    scales: sprite.scales.clone(),
                }),
                Target::Hud([x, y]) => work.hud.push(Patch {
                    x,
                    y,
                    width: sprite.size[0],
                    height: sprite.size[1],
                    rgba: sample(&sprite.clip, blend, sprite.size)?,
                }),
            }
        }
        Ok(work)
    }
    /// Measure each terrain layer once, over its whole animation sheet:
    ///  * its frames resampled to the sprite's size and prepared the way the
    ///    page's chain prepares its base (solidified for a cutout), so a tick
    ///    interpolates two stored frames instead of resampling and solidifying;
    ///  * its per-level alpha scales, which vanilla measures over the sheet's
    ///    stacked frames (`SpriteContents.increaseMipLevel`).
    ///
    /// Both were per-tick work before: five coverage bisections per level plus a
    /// breadth-first solidify of every changed frame, which together were a
    /// quarter of the legacy backend's frame.
    fn measure(&mut self, levels: u32) -> Result<(), String> {
        for sprite in &mut self.sprites {
            if sprite.measured || !matches!(sprite.target, Target::Terrain(_)) {
                sprite.measured = true;
                continue;
            }
            sprite.measured = true;
            let frames = sprite.clip.frame_count();
            let [width, height] = sprite.size;
            if frames == 0 || levels == 0 || width != height || width < 2 {
                continue;
            }
            let mut prepared = Vec::with_capacity(frames);
            for frame in 0..frames {
                let blend = FrameBlend {
                    frame,
                    next: frame,
                    millis: 0,
                };
                prepared.push(sample(&sprite.clip, blend, sprite.size)?);
            }
            sprite.settings = crate::mipmap::Settings {
                has_transparent: prepared
                    .iter()
                    .any(|frame| frame.chunks_exact(4).any(|p| p[3] == 0)),
                dither_cutouts: crate::mipmap::dither_cutouts(),
                ..Default::default()
            };
            // Prepare each frame as the page's own chain prepared its base, and
            // keep the result: a tick interpolates two of these instead of
            // resampling and solidifying the frame it shows.
            for frame in &mut prepared {
                let mut image = crate::mipmap::Image {
                    width,
                    height,
                    pixels: frame.as_chunks::<4>().0.to_vec(),
                };
                crate::mipmap::prepare_base(&mut image, sprite.settings);
                *frame = image.pixels.iter().flatten().copied().collect();
            }
            // The sheet as vanilla stacks it, already prepared: what the levels'
            // alpha scales are measured on.
            let sheet = prepared.concat();
            let image = crate::mipmap::Image {
                width,
                height: height * frames as u32,
                pixels: sheet.as_chunks::<4>().0.to_vec(),
            };
            sprite.scales = crate::mipmap::sheet_scales(&image, frames, levels, sprite.settings);
            sprite.frames = prepared;
        }
        Ok(())
    }
}
/// One animated terrain layer's frame pixels reduced to the levels its page holds,
/// made the way the page's own chain was: cutout-aware, so a flower keeps its
/// coverage at distance whichever frame an animation shows. The alpha scales are
/// the sheet's, measured once at install ([`Animations::measure`]); a frame's own
/// chain re-bisected the coverage five times per level every tick, which was a
/// fifth of the legacy backend's frame.
fn layer_mips(
    pixels: &[u8],
    levels: u32,
    scales: &[f32],
    settings: crate::mipmap::Settings,
) -> Vec<Vec<u8>> {
    let edge = (pixels.len() / 4).isqrt() as u32;
    if levels == 0 || edge >> levels == 0 || pixels.len() % 4 != 0 {
        return Vec::new();
    }
    let base = crate::mipmap::Image {
        width: edge,
        height: edge,
        pixels: pixels.as_chunks::<4>().0.to_vec(),
    };
    // Without a sheet measurement (a sprite too small or not square) the chain
    // is made the slow way, from the frame alone, rather than leaving a cutout
    // unscaled.
    if scales.len() as u32 == levels {
        crate::mipmap::animated_levels(&base, levels, settings, scales)
            .iter()
            .map(|image| image.pixels.iter().flatten().copied().collect())
            .collect()
    } else {
        crate::mipmap::generate(base, levels, settings)
            .iter()
            .skip(1)
            .map(|image| image.pixels.iter().flatten().copied().collect())
            .collect()
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

/// [`sample`] over frames [`Animations::measure`] already resampled and
/// prepared: only the interpolation is left per tick.
fn interpolate(frames: &[Vec<u8>], blend: FrameBlend, size: [u32; 2]) -> Result<Vec<u8>, String> {
    let (Some(current), Some(next)) = (frames.get(blend.frame), frames.get(blend.next)) else {
        return Err("animation frame missing from the prepared sheet".into());
    };
    if current.len() != bytes(size)? || next.len() != current.len() {
        return Err("prepared animation frame has the wrong size".into());
    }
    if blend.millis == 0 || current == next {
        return Ok(current.clone());
    }
    Ok(current
        .iter()
        .zip(next)
        .map(|(&a, &b)| {
            ((u32::from(a) * (1000 - blend.millis) + u32::from(b) * blend.millis + 500) / 1000)
                as u8
        })
        .collect())
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
        // The layers' sheet scales are measured before `prepare` picks work, so
        // the first upload of a world already has them.
        self.animations.measure(self.world.mip_levels())?;
        let work = self.animations.prepare(
            tick,
            self.hud.layout(),
            self.limits.animation_staging_bytes,
        )?;
        let count = work.shown.len();
        self.animations.failed = true;
        let levels = self.world.mip_levels();
        for terrain in &work.terrain {
            let mips = layer_mips(&terrain.rgba, levels, &terrain.scales, terrain.settings);
            if mips.len() as u32 == levels {
                let refs: Vec<&[u8]> = mips.iter().map(Vec::as_slice).collect();
                self.world
                    .update_sprite_mipped(
                        terrain.layer,
                        &terrain.rgba,
                        &refs,
                        self.limits.animation_staging_bytes,
                    )
                    .map_err(diagnostic)?;
            } else {
                // A frame whose sprite is too small for the page's chain still has to
                // be drawn: its base level alone is what that page samples up close.
                self.world
                    .update_sprite(
                        terrain.layer,
                        &terrain.rgba,
                        self.limits.animation_staging_bytes,
                    )
                    .map_err(diagnostic)?;
            }
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
        let mut state = Animations::new(
            std::slice::from_ref(&clip),
            &[],
            &layout,
            hud,
            Limits::default(),
        )
        .unwrap();
        assert_eq!(
            state.sprites[0].clip.pixels(0).as_ptr(),
            clip.pixels(0).as_ptr()
        );
        let work = state.prepare(2, hud, 80).unwrap();
        assert_eq!(work.terrain[0].rgba, [100, 50, 0, 255].repeat(4));
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
    /// An animated cutout's frame must come with the same chain depth as the page
    /// it is patched into, or the update is refused and a distant animated flower
    /// keeps showing its first frame's mips.
    #[test]
    fn an_animated_frames_chain_is_as_deep_as_its_page_and_keeps_the_cutout() {
        let mut pixels = vec![0u8; 8 * 8 * 4];
        for y in 0..8 {
            for x in 0..4 {
                let at = (y * 8 + x) * 4;
                pixels[at..at + 4].copy_from_slice(&[90, 140, 40, 255]);
            }
        }
        let settings = crate::mipmap::Settings {
            has_transparent: true,
            ..Default::default()
        };
        let sheet = crate::mipmap::Image {
            width: 8,
            height: 8,
            pixels: pixels.as_chunks::<4>().0.to_vec(),
        };
        let scales = crate::mipmap::sheet_scales(&sheet, 1, 2, settings);
        assert_eq!(scales.len(), 2);
        let mips = layer_mips(&pixels, 2, &scales, settings);
        assert_eq!(mips.len(), 2);
        assert_eq!(mips[0].len(), 4 * 4 * 4);
        assert_eq!(mips[1].len(), 2 * 2 * 4);
        for level in &mips {
            assert!(
                level.chunks_exact(4).any(|p| p[3] != 0),
                "a level lost every opaque texel"
            );
        }
        // No levels asked for, or a sprite too small to halve: no chain to patch.
        assert!(layer_mips(&pixels, 0, &scales, settings).is_empty());
        assert!(layer_mips(&[255u8; 4], 1, &scales, settings).is_empty());
    }
    /// The animation's sheet decides the scale, not the frame shown: a frame
    /// that is entirely opaque when the sheet has transparent texels still takes
    /// the sheet's cutout chain, so a flower's far level does not fill in.
    #[test]
    fn a_frames_levels_take_the_sheets_alpha_scale_not_its_own() {
        let frame = |solid: bool| {
            let mut pixels = vec![0u8; 8 * 8 * 4];
            for y in 0..8 {
                for x in 0..(if solid { 8 } else { 4 }) {
                    let at = (y * 8 + x) * 4;
                    pixels[at..at + 4].copy_from_slice(&[90, 140, 40, 255]);
                }
            }
            pixels
        };
        let mut sheet = frame(false);
        sheet.extend(frame(true));
        let settings = crate::mipmap::Settings {
            has_transparent: true,
            ..Default::default()
        };
        let image = crate::mipmap::Image {
            width: 8,
            height: 16,
            pixels: sheet.as_chunks::<4>().0.to_vec(),
        };
        let scales = crate::mipmap::sheet_scales(&image, 2, 2, settings);
        let mips = layer_mips(&frame(true), 2, &scales, settings);
        // The solid frame keeps the sheet's coverage: its 2x2 level is not the
        // opaque square a frame-local scale would have produced.
        assert_eq!(mips.len(), 2);
        assert!(
            mips[1].chunks_exact(4).any(|p| p[3] < 255),
            "a frame-local scale left the far level fully opaque"
        );
    }
    #[test]
    fn nearest_sampling_preserves_rgba_interpolation_and_clip_clock() {
        let clip = crate::sprites::test_frames(
            0,
            vec![(0..16).collect(), (100..116).collect()],
            r#"{"frametime":4,"interpolate":true}"#,
        );
        let first = clip.resized_pixels(0, 4, 2);
        for (tick, offset) in [(0, 0), (2, 50), (4, 100), (8, 0), (u64::MAX, 25)] {
            assert_eq!(
                sample(&clip, clip.blend(tick), [4, 2]).unwrap(),
                first.iter().map(|v| v + offset).collect::<Vec<_>>()
            );
        }
        let step = crate::sprites::test_frames(
            0,
            vec![vec![0; 4], vec![100; 4]],
            r#"{"frames":[1,0],"frametime":2}"#,
        );
        for (tick, value) in [(0, 100), (1, 100), (2, 0), (4, 100)] {
            assert_eq!(sample(&step, step.blend(tick), [1, 1]).unwrap(), [value; 4]);
        }
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
        let epoch = atlas.font.epoch();
        assert!(
            TerrainHudRenderer::stage_assets(
                Context::new("osmesa", [64; 2]).unwrap(),
                &assets,
                &atlas,
                generation,
                &[255; 1024],
                Limits {
                    animation_source_bytes: 0,
                    ..Limits::default()
                },
                ResourceLimits::default()
            )
            .is_err()
        );
        assert_eq!(atlas.font.epoch(), epoch);
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
        let mut hud = HudBatch::scratch([64; 2]);
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
            {
                let mut facade = RendererFacade::legacy(
                    &mut renderer,
                    frame::VisibleSections::default(),
                    frame::Options::default(),
                );
                assert_eq!(facade.advance_animations(generation, tick).unwrap(), 2);
                assert_eq!(facade.advance_animations(generation, tick).unwrap(), 0);
            }
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
        let limit = renderer.limits.animation_staging_bytes;
        renderer.limits.animation_staging_bytes = 1;
        assert!(renderer.advance_animations(generation, 4).is_err());
        renderer.render(&scene, &hud).unwrap();
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), before);
        renderer.limits.animation_staging_bytes = limit;
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
        assert!(renderer.render_menu(&hud, [0.; 4]).is_err());
        assert!(
            renderer
                .render_game_frame(
                    &frame::tests::source(&hud),
                    frame::VisibleSections::default(),
                    frame::Options::default()
                )
                .is_err()
        );
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
