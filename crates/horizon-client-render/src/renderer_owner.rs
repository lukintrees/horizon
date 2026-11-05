//! Owning storage for the Game renderer migration. No Deref: modern-only
//! compatibility accesses are deliberately named, not a native fallback.
use crate::{gfx, renderer_facade::RendererFacade};
use horizon_render_api::render_contract::{Backend, FrameOptions, VisibleSections};

#[derive(Default)]
pub struct RendererOwner {
    inner: Owner,
}
#[derive(Default)]
enum Owner {
    #[default]
    Empty,
    Modern(Box<gfx::Renderer>),
    #[cfg(feature = "legacy-gl")]
    Legacy(Box<crate::legacy_gfx::TerrainHudRenderer>),
}
impl RendererOwner {
    pub fn from_modern(renderer: gfx::Renderer) -> Self {
        Self {
            inner: Owner::Modern(Box::new(renderer)),
        }
    }
    #[cfg(feature = "legacy-gl")]
    pub fn from_legacy(renderer: crate::legacy_gfx::TerrainHudRenderer) -> Self {
        Self {
            inner: Owner::Legacy(Box::new(renderer)),
        }
    }
    /// Actual Game's first native bootstrap: CPU assets only, static-menu SDR.
    /// The white lightmap is deliberate for this menu-only route, not a claim
    /// of live world lighting. World startup must supply/tick its own lightmap.
    #[cfg(feature = "legacy-gl")]
    pub fn legacy_menu_bootstrap(
        kind: &str,
        size: [u32; 2],
        assets: &crate::assets::RenderAssets,
        hud: &crate::hud::HudAtlas,
        generation: horizon_render_api::render_contract::ResourceGeneration,
    ) -> crate::Result<Self> {
        let context = horizon_legacy_render::Context::new(kind, size)?;
        Self::legacy_menu_with_context(context, assets, hud, generation)
    }
    /// Transfer the existing loading window/context. Boot GPU resources must
    /// already have been dropped; never create a second native window/device.
    #[cfg(feature = "legacy-gl")]
    pub fn legacy_menu_with_context(
        context: horizon_legacy_render::Context,
        assets: &crate::assets::RenderAssets,
        hud: &crate::hud::HudAtlas,
        generation: horizon_render_api::render_contract::ResourceGeneration,
    ) -> crate::Result<Self> {
        log::info!("native GL2 Game menu factory: {}", context.description());
        let renderer = crate::legacy_gfx::TerrainHudRenderer::stage_assets(
            context,
            assets,
            hud,
            generation,
            &[255; 1024],
            crate::legacy_gfx::Limits::default(),
            crate::legacy_gfx::ResourceLimits::default(),
        )?
        .commit()?;
        Ok(Self::from_legacy(renderer))
    }
    pub fn backend(&self) -> Option<Backend> {
        match self.inner {
            Owner::Empty => None,
            Owner::Modern(_) => Some(Backend::Modern),
            #[cfg(feature = "legacy-gl")]
            Owner::Legacy(_) => Some(Backend::LegacyGl),
        }
    }
    /// The frame size the world producer needs before it can cull or lay out a
    /// frame. `None` while no device exists yet, as before the native route.
    pub fn size(&mut self) -> Option<[u32; 2]> {
        match &mut self.inner {
            Owner::Empty => None,
            Owner::Modern(renderer) => Some([renderer.config.width, renderer.config.height]),
            #[cfg(feature = "legacy-gl")]
            Owner::Legacy(renderer) => Some(renderer.size()),
        }
    }
    /// A native owner may not enter winit/wgpu initialization or recovery.
    /// Empty is permitted: the default modern startup has not created a device yet.
    pub fn allow_modern_startup(&self) -> Result<(), &'static str> {
        if self.backend() == Some(Backend::LegacyGl) {
            return Err("native GL2 owner cannot enter modern window/device startup");
        }
        Ok(())
    }
    /// Transitional leaf paths (shader packs, split composition, HDR, GPU
    /// queries) remain explicitly modern. Native startup must never interpret
    /// None from these as permission to create a modern replacement device.
    pub fn modern_ref(&self) -> Option<&gfx::Renderer> {
        match &self.inner {
            Owner::Modern(r) => Some(r),
            _ => None,
        }
    }
    pub fn modern_mut(&mut self) -> Option<&mut gfx::Renderer> {
        match &mut self.inner {
            Owner::Modern(r) => Some(r),
            _ => None,
        }
    }
    pub fn has_modern(&self) -> bool {
        self.modern_ref().is_some()
    }
    /// Only transfers a modern owner. A native owner is preserved on refusal.
    pub fn take_modern(&mut self) -> Option<gfx::Renderer> {
        if !self.has_modern() {
            return None;
        }
        match std::mem::take(&mut self.inner) {
            Owner::Modern(r) => Some(*r),
            _ => unreachable!(),
        }
    }
    /// Explicit modern publication, not an automatic backend conversion.
    pub fn set_modern(&mut self, renderer: gfx::Renderer) -> crate::Result<()> {
        self.allow_modern_startup()?;
        self.inner = Owner::Modern(Box::new(renderer));
        Ok(())
    }
    /// Borrow the existing submission facade without exposing graphics handles.
    /// Visibility/options are caller-owned CPU data; no invented native world.
    pub fn facade<'a>(
        &'a mut self,
        visible: VisibleSections<'a>,
        options: FrameOptions,
    ) -> Option<RendererFacade<'a>> {
        match &mut self.inner {
            Owner::Empty => None,
            Owner::Modern(r) => {
                let _ = (visible, options);
                Some(RendererFacade::modern(r))
            }
            #[cfg(feature = "legacy-gl")]
            Owner::Legacy(r) => Some(RendererFacade::legacy(r, visible, options)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_owner_does_not_create_a_backend_or_fabricate_a_modern_renderer() {
        let mut owner = RendererOwner::default();
        assert_eq!(owner.backend(), None);
        assert!(!owner.has_modern());
        assert!(owner.allow_modern_startup().is_ok());
        assert!(owner.modern_ref().is_none());
        assert!(owner.modern_mut().is_none());
        assert!(owner.take_modern().is_none());
        assert!(
            owner
                .facade(VisibleSections::default(), FrameOptions::default())
                .is_none()
        );
    }
    #[cfg(feature = "legacy-gl")]
    #[test]
    #[ignore = "requires OSMesa and explicit GL2 software-driver environment"]
    fn native_owner_survives_modern_take_and_dispatches_the_existing_facade() {
        use crate::{
            frame::MenuFrame,
            hud::{HudAtlas, HudBatch},
            legacy_gfx::{Limits, ResourceLimits, TerrainHudRenderer, test_assets},
        };
        use horizon_render_api::{
            render_contract::{FrameOutcome, ResourceGeneration},
            resource_contract::TerrainAtlasLimits,
        };
        let assets = test_assets();
        let atlas = HudAtlas::boot();
        let renderer = TerrainHudRenderer::stage_assets(
            horizon_legacy_render::Context::new("osmesa", [64; 2]).unwrap(),
            &assets,
            &atlas,
            ResourceGeneration(77),
            &[255; 1024],
            Limits::default(),
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
        let mut owner = RendererOwner::from_legacy(renderer);
        assert_eq!(owner.backend(), Some(Backend::LegacyGl));
        assert!(owner.allow_modern_startup().is_err());
        assert!(owner.modern_ref().is_none());
        assert!(owner.modern_mut().is_none());
        assert!(owner.take_modern().is_none());
        assert_eq!(owner.backend(), Some(Backend::LegacyGl));
        let mut facade = owner
            .facade(VisibleSections::default(), FrameOptions::default())
            .unwrap();
        assert_eq!(facade.resource_generation(), Some(ResourceGeneration(77)));
        facade.resize([32; 2]).unwrap();
        assert_eq!(facade.size(), [32; 2]);
        let mut hud = HudBatch::new([32; 2]);
        hud.fill(&atlas, 0., 0., 32., 32., u32::MAX);
        assert_eq!(
            facade
                .render_menu(&MenuFrame::static_color(&hud, [0., 0., 0., 1.]))
                .unwrap(),
            FrameOutcome::Offscreen
        );
        // This verifies ownership/submission only, not Game/native startup.
        assert_eq!(owner.backend(), Some(Backend::LegacyGl));
    }
}

#[cfg(all(test, feature = "legacy-gl"))]
mod bootstrap_tests {
    use super::*;
    use horizon_render_api::render_contract::{FrameOutcome, ResourceGeneration};
    #[test]
    fn bad_bootstrap_surface_does_not_reset_the_cpu_font_or_load_a_driver() {
        let assets = crate::legacy_gfx::test_assets();
        let hud = crate::hud::HudAtlas::boot();
        let epoch = hud.font.epoch();
        assert!(
            RendererOwner::legacy_menu_bootstrap(
                "osmesa",
                [0, 64],
                &assets,
                &hud,
                ResourceGeneration(1)
            )
            .is_err()
        );
        assert_eq!(hud.font.epoch(), epoch);
    }
    #[test]
    #[ignore = "requires OSMesa and explicit GL2 software-driver environment"]
    fn game_bootstrap_factory_uses_cpu_assets_and_submits_real_hud_batches() {
        let assets = crate::legacy_gfx::test_assets();
        let hud = crate::hud::HudAtlas::boot();
        let mut owner = RendererOwner::legacy_menu_bootstrap(
            "osmesa",
            [64; 2],
            &assets,
            &hud,
            ResourceGeneration(1),
        )
        .unwrap();
        assert_eq!(owner.backend(), Some(Backend::LegacyGl));
        let mut batch = crate::hud::HudBatch::new([64; 2]);
        batch.fill(&hud, 0., 0., 64., 64., u32::MAX);
        let menu = crate::frame::MenuFrame::static_color(&batch, [0., 0., 0., 1.]);
        let mut facade = owner
            .facade(VisibleSections::default(), FrameOptions::default())
            .unwrap();
        assert_eq!(facade.resource_generation(), Some(ResourceGeneration(1)));
        assert_eq!(facade.render_menu(&menu).unwrap(), FrameOutcome::Offscreen);
    }
}
