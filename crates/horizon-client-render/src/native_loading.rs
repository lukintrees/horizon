//! Asset-free loading HUD on a borrowed native context. Dropping this screen
//! releases all boot GPU resources, not the window/context handed to Game.
use crate::{
    gui_bridge,
    hud::{HudAtlas, HudBatch},
};
pub use horizon_legacy_render::Context;
use horizon_legacy_render::{Budget, Renderer, Stats};
const BOOT_VERTICES: usize = 32768;
const BOOT_MESH_BYTES: usize = 2 << 20;
const _: () = assert!(
    BOOT_VERTICES
        * (std::mem::size_of::<horizon_render_api::gui::GuiVertex>() + std::mem::size_of::<u16>())
        <= BOOT_MESH_BYTES
);
use horizon_render_api::{
    Clear, Frame, IDENTITY, Image, RenderBackend, TextureId, gui, render_contract::FrameOutcome,
};

pub struct LoadingScreen<'a> {
    backend: Renderer<'a>,
    atlas: TextureId,
    white: TextureId,
    budget: Budget,
    atlas_size: [u32; 2],
    uv_scale: [f32; 2],
}
impl<'a> LoadingScreen<'a> {
    pub fn new(context: &'a Context, hud: &HudAtlas) -> Result<Self, String> {
        let budget = Budget {
            texture_bytes: 8 << 20,
            mesh_bytes: 2 << 20,
        };
        Self::from_backend(Renderer::new(context, budget)?, hud, budget)
    }
    fn from_backend(
        mut backend: Renderer<'a>,
        hud: &HudAtlas,
        budget: Budget,
    ) -> Result<Self, String> {
        let (atlas, uv_scale) = gui::upload_atlas(
            &mut backend,
            [hud.width, hud.height],
            &hud.pixels(),
            8 << 20,
        )
        .map_err(|e| format!("native boot atlas: {e:?}"))?;
        let white = backend.create_texture(Image {
            width: 1,
            height: 1,
            rgba: &[255; 4],
        })?;
        hud.font.reset_cache();
        Ok(Self {
            backend,
            atlas,
            white,
            budget,
            atlas_size: [hud.width, hud.height],
            uv_scale,
        })
    }
    pub fn context(&self) -> &Context {
        self.backend.context()
    }
    pub fn draw(&mut self, batch: &HudBatch) -> Result<FrameOutcome, String> {
        let size = self.backend.context().size();
        if size.contains(&0) {
            return Ok(FrameOutcome::Deferred);
        }
        // Reject an unsupported material before changing the framebuffer.
        let runs = gui_bridge::runs(batch, size, self.uv_scale, BOOT_VERTICES)
            .map_err(|e| format!("native boot HUD: {e:?}"))?;
        gui_bridge::upload_patches(
            &mut self.backend,
            self.atlas,
            self.atlas_size,
            batch,
            4 << 20,
        )
        .map_err(|e| format!("native boot glyphs: {e:?}"))?;
        self.backend.render(&Frame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            fog: None,
            clear: Clear::ColorDepth([0.025, 0.025, 0.025, 1.]),
            draws: &[],
        })?;
        gui::render(&mut self.backend, &runs, self.atlas, self.white)
            .map_err(|e| format!("native boot draw: {e:?}"))?;
        Ok(if self.backend.context().present()? {
            FrameOutcome::Presented
        } else {
            FrameOutcome::Offscreen
        })
    }
}

impl LoadingScreen<'static> {
    /// Own the boot context so a staged renderer can share it without borrowing
    /// self-referential storage or manufacturing another native window.
    pub fn owned(context: Context, hud: &HudAtlas, budget: Budget) -> Result<Self, String> {
        Self::from_backend(Renderer::owned(context, budget)?, hud, budget)
    }
    /// One consumed boot owner -> one committed Game renderer. All callbacks
    /// run on the owner thread and may draw ONLY this boot screen, never an
    /// incomplete candidate. Drop boot allocations before restoring final
    /// capacity/committing font state. Errors reclaim both candidate and boot.
    pub fn finish_assets(
        mut self,
        assets: &crate::assets::RenderAssets,
        hud: &HudAtlas,
        generation: horizon_render_api::render_contract::ResourceGeneration,
        limits: crate::legacy_gfx::Limits,
        preparation: crate::legacy_gfx::ResourceLimits,
        checkpoint: &mut impl FnMut(&mut Self, crate::legacy_gfx::StagePoint) -> Result<(), String>,
    ) -> Result<crate::legacy_gfx::TerrainHudRenderer, String> {
        if limits.gpu.texture_bytes != self.budget.texture_bytes
            || limits.gpu.mesh_bytes != self.budget.mesh_bytes
        {
            return Err("boot and Game must share the same tracked GPU budget".into());
        }
        checkpoint(&mut self, crate::legacy_gfx::StagePoint::Backend)?;
        // gui::render temporarily uploads at most one mesh per triangle, with
        // 52-byte GuiVertex values + u16 indices, then deletes every mesh. Atlas glyph
        // patches update the two existing textures; no texture headroom needed.
        let reserve = Stats {
            mesh_bytes: BOOT_MESH_BYTES,
            meshes: BOOT_VERTICES / 3 + 1,
            ..Default::default()
        };
        let backend = self.backend.staging_replacement_reserving(reserve)?;
        self.backend.set_budget(Budget {
            texture_bytes: self.backend.stats().texture_bytes,
            mesh_bytes: BOOT_MESH_BYTES,
        })?;
        let free = backend.remaining();
        let mut staged_limits = limits;
        staged_limits.gpu = Budget {
            texture_bytes: free.texture_bytes,
            mesh_bytes: free.mesh_bytes,
        };
        let staged = crate::legacy_gfx::TerrainHudRenderer::stage_assets_backend(
            backend,
            assets,
            hud,
            generation,
            staged_limits,
            preparation,
            &mut |point, _context| checkpoint(&mut self, point),
        )?;
        drop(self);
        staged.commit_after_boot(limits)
    }
}
