//! Borrow the existing Game resource producers, not a second pack loader.
//! This is a base-frame SDR snapshot; animated updates/reload publication remain
//! separate. Staging never resets the font cache used by the active renderer.
use super::{Context, Image, Limits, ResourceLimits, TerrainHudRenderer};
use crate::{assets::RenderAssets, hud::HudAtlas};
use horizon_render_api::{
    render_contract::ResourceGeneration,
    resource_contract::{Resources, TextureLayers, image_bytes},
};
use std::borrow::Cow;

pub struct AssetSnapshot<'a> {
    assets: &'a RenderAssets,
    hud: &'a HudAtlas,
    pixels: Cow<'a, [u8]>,
    generation: ResourceGeneration,
}
impl<'a> AssetSnapshot<'a> {
    /// `generation` must be issued by the resource owner for the full bundle;
    /// do not derive it solely from RenderAssets::generation on HUD-only reload.
    /// Bound all base pixel bytes before waiting for/decompressing packed HUD
    /// pixels. Terrain and glint pixels remain borrowed; a compressed HUD uses
    /// one bounded temporary decode, discarded when upload finishes.
    pub fn new(
        assets: &'a RenderAssets,
        hud: &'a HudAtlas,
        generation: ResourceGeneration,
        max_bytes: usize,
    ) -> Result<Self, String> {
        if hud.text_shader.is_some() || hud.lines_shader.is_some() {
            return Err(
                "legacy asset snapshot does not support custom core text/line shaders".into(),
            );
        }
        let terrain = TextureLayers {
            size: assets.sprite_size,
            layers: assets.texture_layers,
            rgba: &assets.pixels,
        };
        let terrain_bytes = terrain.validate().map_err(super::diagnostic)?;
        let hud_bytes = image_bytes(hud.width, hud.height).map_err(super::diagnostic)?;
        let mut total = terrain_bytes
            .checked_add(hud_bytes)
            .ok_or("asset pixel size overflow")?;
        for (pixels, width, height) in [&hud.glint, &hud.glint_armor] {
            let bytes = image_bytes(*width, *height).map_err(super::diagnostic)?;
            if pixels.len() != bytes {
                return Err("invalid asset glint pixel length".into());
            }
            total = total
                .checked_add(bytes)
                .ok_or("asset pixel size overflow")?;
        }
        if total > max_bytes {
            return Err("asset base pixel budget exceeded".into());
        }
        let pixels = hud.checked_pixels(hud_bytes)?;
        Ok(Self {
            assets,
            hud,
            pixels,
            generation,
        })
    }
    /// Source provenance is distinct from the caller-issued full bundle epoch:
    /// a HUD-only reload can change layout without reloading RenderAssets.
    pub fn asset_generation(&self) -> u64 {
        self.assets.generation
    }
    pub fn hud_revision(&self) -> u64 {
        self.hud.revision
    }
    pub fn resources(&self) -> Resources<'_> {
        Resources {
            generation: self.generation,
            terrain: TextureLayers {
                size: self.assets.sprite_size,
                layers: self.assets.texture_layers,
                rgba: &self.assets.pixels,
            },
            hud: Image {
                width: self.hud.width,
                height: self.hud.height,
                rgba: &self.pixels,
            },
            item_glint: Image {
                width: self.hud.glint.1,
                height: self.hud.glint.2,
                rgba: &self.hud.glint.0,
            },
            armor_glint: Image {
                width: self.hud.glint_armor.1,
                height: self.hud.glint_armor.2,
                rgba: &self.hud.glint_armor.0,
            },
        }
    }
    /// These producers carry animations which this base snapshot does not tick.
    /// F01/A02 must select/update them explicitly before claiming live pack support.
    pub fn animation_counts(&self) -> [usize; 2] {
        [
            self.assets.animations.len(),
            self.hud.sprite_animations.len(),
        ]
    }
}

#[must_use = "commit the staged renderer, or drop it to keep the active font cache unchanged"]
pub struct StagedAssetRenderer<'a> {
    renderer: TerrainHudRenderer,
    hud: &'a HudAtlas,
    font_epoch: u64,
}
impl StagedAssetRenderer<'_> {
    /// The caller's publication boundary: no fallible GPU work after this font
    /// reset. Old text batches must be rebuilt. A changed font epoch rejects the
    /// staged owner instead of committing under stale producer state.
    pub fn commit(self) -> Result<TerrainHudRenderer, String> {
        if self.hud.font.epoch() != self.font_epoch {
            return Err("font changed during native asset staging; rebuild the snapshot".into());
        }
        self.hud.font.reset_renderer_cache();
        Ok(self.renderer)
    }
}
impl TerrainHudRenderer {
    /// Prepare from RenderAssets/HudAtlas without resetting glyph UVs/patches.
    /// A failed or discarded stage leaves the active font cache untouched.
    pub fn stage_assets<'a>(
        context: Context,
        assets: &RenderAssets,
        hud: &'a HudAtlas,
        generation: ResourceGeneration,
        lightmap: &[u8],
        limits: Limits,
        preparation: ResourceLimits,
    ) -> Result<StagedAssetRenderer<'a>, String> {
        let font_epoch = hud.font.epoch();
        let snapshot = AssetSnapshot::new(assets, hud, generation, preparation.base_bytes)?;
        let renderer = Self::from_resources(
            context,
            &snapshot.resources(),
            lightmap,
            limits,
            preparation,
        )?;
        Ok(StagedAssetRenderer {
            renderer,
            hud,
            font_epoch,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn assets() -> RenderAssets {
        RenderAssets {
            sprite_size: 1,
            sprite_minimum_size: 1,
            pixels: vec![255, 0, 0, 255, 0, 255, 0, 255],
            texture_layers: 2,
            model_catalog: Default::default(),
            model_library: Err("synthetic snapshot fixture".into()),
            model_placements: vec![],
            model_gallery: vec![],
            animations: vec![],
            sprite_layers: Default::default(),
            emissive: vec![],
            translucent_layers: vec![],
            sprite_mips: vec![],
            pbr_normals: vec![],
            pbr_specular: vec![],
            pbr_size: 0,
            connected: Default::default(),
            fusion: Default::default(),
            generation: 23,
        }
    }
    #[test]
    fn snapshots_preserve_producer_buffers_generation_and_compressed_hud_without_font_reset() {
        let assets = assets();
        let mut hud = HudAtlas::boot();
        let epoch = hud.font.epoch();
        let bytes =
            assets.pixels.len() + hud.pixels.len() + hud.glint.0.len() + hud.glint_armor.0.len();
        {
            let snapshot =
                AssetSnapshot::new(&assets, &hud, ResourceGeneration(77), bytes).unwrap();
            let bundle = snapshot.resources();
            assert_eq!(bundle.validate(bytes), Ok(bytes));
            assert_eq!(bundle.generation, ResourceGeneration(77));
            assert_eq!(snapshot.asset_generation(), 23);
            assert_eq!(snapshot.hud_revision(), hud.revision);
            assert_eq!(bundle.terrain.rgba.as_ptr(), assets.pixels.as_ptr());
            assert_eq!(bundle.hud.rgba.as_ptr(), hud.pixels.as_ptr());
            assert_eq!(bundle.item_glint.rgba.as_ptr(), hud.glint.0.as_ptr());
            assert_eq!(snapshot.animation_counts(), [0, 0]);
            assert!(AssetSnapshot::new(&assets, &hud, ResourceGeneration(77), bytes - 1).is_err());
        }
        hud.pack_pixels();
        let snapshot = AssetSnapshot::new(&assets, &hud, ResourceGeneration(77), bytes).unwrap();
        assert_eq!(snapshot.resources().validate(bytes), Ok(bytes));
        assert_eq!(hud.font.epoch(), epoch);
        drop(snapshot);
        let replacement_hud = HudAtlas::boot();
        let replacement =
            AssetSnapshot::new(&assets, &replacement_hud, ResourceGeneration(78), bytes).unwrap();
        assert_eq!(replacement.asset_generation(), 23);
        assert_eq!(replacement.resources().generation, ResourceGeneration(78));
        assert_ne!(replacement.hud_revision(), hud.revision);
        drop(replacement);
        let mut bad_assets = assets;
        bad_assets.pixels.pop();
        assert!(AssetSnapshot::new(&bad_assets, &hud, ResourceGeneration(78), usize::MAX).is_err());
    }
}
