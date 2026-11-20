//! Native factory from borrowed CPU resources. Does not initialize wgpu, decode
//! a pack, publish a reload or choose the Game's platform/renderer owner.
use super::*;
use crate::mipmap::{self, Strategy};
use horizon_render_api::{
    atlas::{AtlasError, sprite_patch_levels},
    render_contract::ResourceGeneration,
    resource_contract::{Resources, TerrainAtlas, TerrainAtlasLimits},
    scene::MippedPage,
};

#[derive(Clone, Copy, Debug)]
pub struct ResourceLimits {
    pub base_bytes: usize,
    pub terrain: TerrainAtlasLimits,
    /// Levels below each terrain page's base level (the `Mipmap Levels` option's
    /// default). Zero keeps one level, which is all a driver without mipmapped
    /// sampling of an atlas can use.
    ///
    /// Reported: "у цветов и травы мерцают текстуры" -- a flower's cutout texels
    /// are a texel or two across, so minified without mips the nearest one wins
    /// from one frame to the next and the sprite sparkles; the modern renderer
    /// minifies through the chain vanilla builds. The chain is cutout-aware
    /// (`mipmap::generate`), so thin grass and flower edges keep their coverage.
    pub mip_levels: u32,
}
impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            base_bytes: 128 << 20,
            terrain: TerrainAtlasLimits {
                // A section is drawn as one GL mesh per (atlas page, pass): the more pages the
                // 1301 vanilla block sprites need, the more objects and draw calls one section
                // costs. The gutter is `1 << mip_levels` texels (vanilla's `Stitcher` padding),
                // so at the shipped 4 levels a 64 px sprite occupies 96 px: a 1024 px page holds
                // 100 of them and the set needs 14 pages, which still leaves a section's solid
                // meshes well inside the backend's 65536-object limit at a radius-12 view.
                // 2048 is the largest texture some GL2-era cards (Radeon X1xxx) accept, so
                // 1024 leaves the smaller page size those drivers are sure of.
                page_size: 1024,
                max_pages: 32,
                max_layers: 32768,
                // One decoded page (4 MiB) plus the borrowed layer table.
                staging_bytes: 8 << 20,
                // Filled from `mip_levels` by `terrain_limits`: the gutter and the
                // chain depth have to be chosen together.
                mip_levels: 0,
            },
            // `Options.mipmapLevels` ships at 4.
            mip_levels: 4,
        }
    }
}

/// How many levels below the base one a page can carry: what was asked for, but
/// no more than the smallest sprite has (a level below one texel has nothing to
/// hold) and no more than leaves the page two texels wide, since a level is
/// addressed by its box.
fn usable_mip_levels(terrain: &TerrainAtlas<'_>, requested: u32) -> u32 {
    let sprite = terrain.size().max(1).ilog2();
    let page = terrain.layout().page_size().max(2).ilog2() - 1;
    requested.min(sprite).min(page)
}

/// One page's pixels (level zero) and the levels below them, every sprite's chain
/// made by `mipmap::generate` so a cutout keeps its coverage as it minifies.
fn mipped_page(
    terrain: &TerrainAtlas<'_>,
    page: usize,
    levels: u32,
) -> Result<MippedPage, AtlasError> {
    let layout = terrain.layout();
    let side = layout.page_size();
    let rgba = terrain.page(page)?;
    if levels == 0 || side >> levels == 0 {
        return Ok(MippedPage {
            rgba,
            mips: Vec::new(),
        });
    }
    let mut mips: Vec<Vec<u8>> = (0..levels)
        .map(|level| {
            let edge = (side >> (level + 1)) as usize;
            vec![0; edge * edge * 4]
        })
        .collect();
    // A page holds hundreds of sprites and every one of them builds its own
    // chain, each level bisecting the alpha scale for its coverage: on one thread
    // the pages alone would hold up the loading screen for seconds. The sprites go
    // across the machine's threads and are blitted in layout order.
    let sprites: Vec<(usize, u32)> = layout
        .sprites()
        .iter()
        .enumerate()
        .filter(|(_, place)| place.page == page)
        .map(|(index, place)| (index, place.width.min(place.height)))
        .collect();
    let chains = crate::par::par_map(&sprites, |&(index, size)| {
        if size >> levels == 0 {
            // A sprite smaller than the page cannot fill every level: its own
            // chain stops at one texel, and the higher levels stay the neighbours'.
            return Ok(Vec::new());
        }
        let pixels = terrain.layer(index).ok_or(AtlasError::InvalidSpriteIndex)?;
        let base = mipmap::Image {
            width: size,
            height: size,
            pixels: pixels.as_chunks::<4>().0.to_vec(),
        };
        let settings = mipmap::Settings {
            strategy: Strategy::Auto,
            has_transparent: base.pixels.iter().any(|p| p[3] == 0),
            dither_cutouts: mipmap::dither_cutouts(),
            ..Default::default()
        };
        Ok::<_, AtlasError>(
            mipmap::generate(base, levels, settings)
                .iter()
                .skip(1)
                .map(|image| image.pixels.iter().flatten().copied().collect())
                .collect::<Vec<Vec<u8>>>(),
        )
    });
    for (&(index, size), chain) in sprites.iter().zip(chains) {
        let chain = chain?;
        if chain.is_empty() {
            continue;
        }
        let refs: Vec<&[u8]> = chain.iter().map(Vec::as_slice).collect();
        let budget = size as usize * size as usize * 4;
        for (level, patch) in sprite_patch_levels(layout, index, &refs, budget)?
            .iter()
            .enumerate()
        {
            let edge = (side >> (level as u32 + 1)) as usize;
            let target = &mut mips[level];
            for row in 0..patch.size[1] as usize {
                let from = row * patch.size[0] as usize * 4;
                let to = ((patch.at[1] as usize + row) * edge + patch.at[0] as usize) * 4;
                let len = patch.size[0] as usize * 4;
                target[to..to + len].copy_from_slice(&patch.rgba[from..from + len]);
            }
        }
    }
    Ok(MippedPage { rgba, mips })
}

fn prepare<'a>(
    resources: &Resources<'a>,
    lightmap: &[u8],
    limits: Limits,
    preparation: ResourceLimits,
) -> Result<TerrainAtlas<'a>, String> {
    resources
        .validate(preparation.base_bytes)
        .map_err(diagnostic)?;
    if lightmap.len() != 1024 {
        return Err("legacy resource lightmap must be 16x16 RGBA8".into());
    }
    if limits.hud_resident_pages == 0 {
        return Err("legacy HUD needs at least one resident page".into());
    }
    for image in [&resources.item_glint, &resources.armor_glint] {
        if !image.width.is_power_of_two() || !image.height.is_power_of_two() {
            return Err("legacy repeating glint images must be power-of-two".into());
        }
    }
    let hud = Layout::new(
        [resources.hud.width, resources.hud.height],
        limits.hud_page_size,
        limits.hud_page_count,
    )
    .map_err(diagnostic)?;
    let terrain = TerrainAtlas::new(
        &resources.terrain,
        terrain_limits(
            preparation.terrain,
            preparation.mip_levels,
            resources.terrain.size,
            preparation.terrain.page_size,
        ),
    )
    .map_err(diagnostic)?;
    let bytes = terrain
        .texture_bytes_with_mips(usable_mip_levels(&terrain, preparation.mip_levels))
        .map_err(diagnostic)?
        .checked_add(1024 + 4)
        .and_then(|n| n.checked_add(resources.item_glint.rgba.len()))
        .and_then(|n| n.checked_add(resources.armor_glint.rgba.len()))
        .and_then(|n| n.checked_add(hud.page_bytes()))
        .filter(|&n| n <= limits.gpu.texture_bytes);
    // Terrain, lightmap, white, two glints and room for at least one HUD page.
    if bytes.is_none() || terrain.layout().page_count() > 4096 - 5 {
        return Err("legacy base textures leave no HUD residency within GPU budget".into());
    }
    Ok(terrain)
}

/// The atlas bounds for a preparation policy: the caller's page and layer
/// limits, with the gutter sized to the mip depth the backend will build.
/// The two travel together because a one-texel gutter is only wide enough for
/// level zero -- see [`TerrainAtlasLimits::mip_levels`].
fn terrain_limits(
    mut terrain: TerrainAtlasLimits,
    requested: u32,
    size: u32,
    page: u32,
) -> TerrainAtlasLimits {
    let sprite = size.max(1).ilog2();
    let page = page.max(2).ilog2() - 1;
    terrain.mip_levels = requested.min(sprite).min(page);
    terrain
}

impl TerrainHudRenderer {
    /// Creates one native owner, streaming a single terrain page into GL at a
    /// time. Resources remain borrowed only during construction; no full CPU
    /// terrain copy is retained. Generation becomes observable only on success.
    pub fn from_resources(
        context: Context,
        resources: &Resources<'_>,
        lightmap: &[u8],
        limits: Limits,
        preparation: ResourceLimits,
    ) -> Result<Self, String> {
        let terrain = prepare(resources, lightmap, limits, preparation)?;
        let backend = Renderer::owned(context, limits.gpu)?;
        Self::from_terrain(backend, terrain, resources, lightmap, limits, preparation)
    }

    pub(super) fn from_resources_backend(
        backend: Renderer<'static>,
        resources: &Resources<'_>,
        lightmap: &[u8],
        limits: Limits,
        preparation: ResourceLimits,
    ) -> Result<Self, String> {
        let terrain = prepare(resources, lightmap, limits, preparation)?;
        Self::from_terrain(backend, terrain, resources, lightmap, limits, preparation)
    }

    pub(crate) fn from_resources_backend_checked(
        backend: Renderer<'static>,
        resources: &Resources<'_>,
        lightmap: &[u8],
        limits: Limits,
        preparation: ResourceLimits,
        checkpoint: &mut impl FnMut(StagePoint, &Context) -> Result<(), String>,
    ) -> Result<Self, String> {
        checkpoint(StagePoint::Layout, backend.context())?;
        let terrain = prepare(resources, lightmap, limits, preparation)?;
        Self::from_terrain_checked(
            backend,
            terrain,
            resources,
            lightmap,
            limits,
            preparation,
            checkpoint,
        )
    }

    fn from_terrain(
        backend: Renderer<'static>,
        terrain: TerrainAtlas<'_>,
        resources: &Resources<'_>,
        lightmap: &[u8],
        limits: Limits,
        preparation: ResourceLimits,
    ) -> Result<Self, String> {
        Self::from_terrain_checked(
            backend,
            terrain,
            resources,
            lightmap,
            limits,
            preparation,
            &mut |_, _| Ok(()),
        )
    }

    fn from_terrain_checked(
        backend: Renderer<'static>,
        terrain: TerrainAtlas<'_>,
        resources: &Resources<'_>,
        lightmap: &[u8],
        limits: Limits,
        preparation: ResourceLimits,
        checkpoint: &mut impl FnMut(StagePoint, &Context) -> Result<(), String>,
    ) -> Result<Self, String> {
        let levels = usable_mip_levels(&terrain, preparation.mip_levels);
        let world = WorldScene::new_paged_mipped_checked(
            backend,
            terrain.layout().clone(),
            lightmap,
            limits.sections,
            |page| mipped_page(&terrain, page, levels),
            |backend, completed, total| {
                checkpoint(StagePoint::Terrain { completed, total }, backend.context())
            },
        )
        .map_err(diagnostic)?;
        drop(terrain);
        let mut world = world;
        checkpoint(StagePoint::Hud, world.backend_mut().context())?;
        let mut renderer = Self::from_world(
            world,
            lightmap,
            Image {
                width: resources.hud.width,
                height: resources.hud.height,
                rgba: resources.hud.rgba,
            },
            limits,
        )?;
        checkpoint(StagePoint::Glint, renderer.world.backend_mut().context())?;
        renderer.set_glint_textures(
            Image {
                width: resources.item_glint.width,
                height: resources.item_glint.height,
                rgba: resources.item_glint.rgba,
            },
            Image {
                width: resources.armor_glint.width,
                height: resources.armor_glint.height,
                rgba: resources.armor_glint.rgba,
            },
        )?;
        renderer.resource_generation = Some(resources.generation);
        Ok(renderer)
    }
    pub fn resource_generation(&self) -> Option<ResourceGeneration> {
        self.resource_generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A 8x8 cutout sprite -- the left half opaque, the right half clear, as a
    /// flower's petals are against its transparent corner -- in a 16 px page.
    fn flower() -> ([u8; 8 * 8 * 4], TerrainAtlasLimits) {
        let mut pixels = [0u8; 8 * 8 * 4];
        for y in 0..8 {
            for x in 0..4 {
                let at = (y * 8 + x) * 4;
                pixels[at..at + 4].copy_from_slice(&[90, 140, 40, 255]);
            }
        }
        (
            pixels,
            TerrainAtlasLimits {
                page_size: 16,
                max_pages: 1,
                max_layers: 1,
                staging_bytes: 16 * 16 * 4 + std::mem::size_of::<&[u8]>(),
                mip_levels: 0,
            },
        )
    }
    #[test]
    fn a_mipped_page_carries_every_level_and_keeps_a_cutouts_coverage_at_distance() {
        let (pixels, limits) = flower();
        let layers = horizon_render_api::resource_contract::TextureLayers {
            size: 8,
            layers: 1,
            rgba: &pixels,
        };
        let terrain = TerrainAtlas::new(&layers, limits).unwrap();
        let page = mipped_page(&terrain, 0, 2).unwrap();
        assert_eq!(page.rgba.len(), 16 * 16 * 4);
        assert_eq!(page.mips.len(), 2);
        assert_eq!(page.mips[0].len(), 8 * 8 * 4);
        assert_eq!(page.mips[1].len(), 4 * 4 * 4);
        // The cutout survives minification: each level still has opaque texels, so a
        // distant flower keeps its shape instead of eroding to a gap.
        for level in &page.mips {
            assert!(
                level.chunks_exact(4).any(|p| p[3] != 0),
                "a level lost every opaque texel"
            );
        }
        // Zero levels is the single-level page the backend uploads as before.
        let flat = mipped_page(&terrain, 0, 0).unwrap();
        assert!(flat.mips.is_empty());
        assert_eq!(flat.rgba.len(), 16 * 16 * 4);
    }
    #[test]
    fn preparation_checks_base_staging_gpu_and_hud_room_without_creating_a_context() {
        let image = || Image {
            width: 1,
            height: 1,
            rgba: &[255; 4],
        };
        let resources = Resources {
            generation: ResourceGeneration(7),
            terrain: horizon_render_api::resource_contract::TextureLayers {
                size: 1,
                layers: 1,
                rgba: &[255; 4],
            },
            hud: image(),
            item_glint: image(),
            armor_glint: image(),
        };
        let preparation = ResourceLimits {
            base_bytes: 16,
            terrain: TerrainAtlasLimits {
                page_size: 4,
                max_pages: 1,
                max_layers: 1,
                staging_bytes: 64 + std::mem::size_of::<&[u8]>(),
                mip_levels: 0,
            },
            mip_levels: 0,
        };
        let required = 64 + 1024 + 4 + 8 + 512 * 512 * 4;
        let limits = Limits {
            gpu: Budget {
                texture_bytes: required,
                ..Budget::default()
            },
            ..Limits::default()
        };
        assert!(prepare(&resources, &[255; 1024], limits, preparation).is_ok());
        assert!(
            prepare(
                &resources,
                &[255; 1024],
                Limits {
                    gpu: Budget {
                        texture_bytes: required - 1,
                        ..Budget::default()
                    },
                    ..limits
                },
                preparation
            )
            .is_err()
        );
        assert!(
            prepare(
                &resources,
                &[255; 1024],
                limits,
                ResourceLimits {
                    base_bytes: 15,
                    ..preparation
                }
            )
            .is_err()
        );
        assert!(prepare(&resources, &[], limits, preparation).is_err());
        assert!(
            prepare(
                &resources,
                &[255; 1024],
                Limits {
                    hud_resident_pages: 0,
                    ..limits
                },
                preparation
            )
            .is_err()
        );
    }
}
