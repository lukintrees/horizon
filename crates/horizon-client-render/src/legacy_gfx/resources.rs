//! Native factory from borrowed CPU resources. Does not initialize wgpu, decode
//! a pack, publish a reload or choose the Game's platform/renderer owner.
use super::*;
use horizon_render_api::{
    render_contract::ResourceGeneration,
    resource_contract::{Resources, TerrainAtlas, TerrainAtlasLimits},
};

#[derive(Clone, Copy, Debug)]
pub struct ResourceLimits {
    pub base_bytes: usize,
    pub terrain: TerrainAtlasLimits,
}
impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            base_bytes: 128 << 20,
            terrain: TerrainAtlasLimits::default(),
        }
    }
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
    let terrain = TerrainAtlas::new(&resources.terrain, preparation.terrain).map_err(diagnostic)?;
    let bytes = terrain
        .texture_bytes()
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
        let world = WorldScene::new_paged(
            backend,
            terrain.layout().clone(),
            lightmap,
            limits.sections,
            |page| terrain.page(page),
        )
        .map_err(diagnostic)?;
        drop(terrain);
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
            },
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
