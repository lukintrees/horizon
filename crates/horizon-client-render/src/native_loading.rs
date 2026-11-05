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
        hud.font.reset_renderer_cache();
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires Mesa OSMesa and GL2 overrides"]
    fn boot_resources_are_bounded_and_context_survives_handoff() {
        exercise("osmesa", FrameOutcome::Offscreen);
    }
    #[test]
    #[ignore = "requires X11/GLX display and Mesa GL2 overrides"]
    fn visible_boot_window_survives_same_context_game_asset_handoff() {
        exercise("glx", FrameOutcome::Presented);
    }
    fn exercise(kind: &str, expected: FrameOutcome) {
        let context = Context::new(kind, [96, 64]).unwrap();
        let atlas = HudAtlas::boot();
        {
            let mut screen = LoadingScreen::new(&context, &atlas).unwrap();
            let baseline = screen.backend.stats();
            for _ in 0..20 {
                atlas.font.begin_frame();
                let mut batch = HudBatch::new(context.size());
                batch.fill(&atlas, 0., 0., batch.width, batch.height, u32::MAX);
                assert_eq!(screen.draw(&batch).unwrap(), expected);
                if !context.is_windowed() {
                    assert_eq!(screen.backend.read_rgba([48, 32, 1, 1]).unwrap(), [255; 4]);
                }
                context.poll_events().unwrap();
                assert_eq!(screen.backend.stats(), baseline);
            }
        }
        // No replacement window/context or lifetime leak: transfer that exact
        // context into the ordinary native asset factory after boot GPU drop.
        let assets = crate::legacy_gfx::test_assets();
        let mut owner = crate::renderer_owner::RendererOwner::legacy_menu_with_context(
            context,
            &assets,
            &atlas,
            horizon_render_api::render_contract::ResourceGeneration(1),
        )
        .unwrap();
        let mut batch = HudBatch::new([96, 64]);
        batch.fill(&atlas, 0., 0., batch.width, batch.height, u32::MAX);
        let mut facade = owner
            .facade(Default::default(), Default::default())
            .unwrap();
        assert_eq!(
            facade
                .render_menu(&crate::frame::MenuFrame::static_color(&batch, [0.; 4]))
                .unwrap(),
            expected
        );
    }
}

#[cfg(test)]
mod checked_stage_tests {
    use super::*;
    use crate::{
        hud::HudBatch,
        legacy_gfx::{Limits, ResourceLimits, StagePoint},
    };
    use horizon_render_api::{
        render_contract::ResourceGeneration, resource_contract::TerrainAtlasLimits,
    };

    fn preparation() -> ResourceLimits {
        ResourceLimits {
            terrain: TerrainAtlasLimits {
                page_size: 4,
                max_pages: 2,
                ..Default::default()
            },
            ..Default::default()
        }
    }
    fn fill(screen: &mut LoadingScreen<'_>, hud: &HudAtlas) -> FrameOutcome {
        let mut batch = HudBatch::new(screen.context().size());
        batch.fill(hud, 0., 0., batch.width, batch.height, u32::MAX);
        screen.draw(&batch).unwrap()
    }
    fn exercise(kind: &str, outcome: FrameOutcome) {
        let boot = HudAtlas::boot();
        let gui = HudAtlas::boot();
        let epoch = gui.font.epoch();
        let limits = Limits::default();
        let context = Context::new(kind, [96, 64]).unwrap();
        let screen = LoadingScreen::owned(context, &boot, limits.gpu).unwrap();
        let identity = screen.context() as *const Context;
        let baseline = screen.backend.stats();
        let mut seen = Vec::new();
        let mut renderer = screen
            .finish_assets(
                &crate::legacy_gfx::test_assets(),
                &gui,
                ResourceGeneration(9),
                limits,
                preparation(),
                &mut |screen, point| {
                    assert_eq!(screen.context() as *const Context, identity);
                    assert_eq!(
                        gui.font.epoch(),
                        epoch,
                        "published font before complete stage"
                    );
                    screen.context().poll_events()?;
                    if point
                        == (StagePoint::Terrain {
                            completed: 1,
                            total: 2,
                        })
                    {
                        screen.context().resize([80, 48])?;
                    }
                    assert_eq!(fill(screen, &boot), outcome);
                    assert_eq!(
                        screen.backend.stats(),
                        baseline,
                        "boot allocations accumulated"
                    );
                    seen.push(point);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(
            seen,
            [
                StagePoint::Backend,
                StagePoint::Snapshot,
                StagePoint::Layout,
                StagePoint::Terrain {
                    completed: 0,
                    total: 2
                },
                StagePoint::Terrain {
                    completed: 1,
                    total: 2
                },
                StagePoint::Terrain {
                    completed: 2,
                    total: 2
                },
                StagePoint::Hud,
                StagePoint::Glint,
                StagePoint::Animations,
                StagePoint::Ready
            ]
        );
        assert_ne!(gui.font.epoch(), epoch);
        assert_eq!(renderer.size(), [80, 48]);
        assert_eq!(renderer.resource_generation(), Some(ResourceGeneration(9)));
        let mut batch = HudBatch::new([80, 48]);
        batch.fill(&gui, 0., 0., batch.width, batch.height, u32::MAX);
        assert_eq!(
            crate::renderer_facade::RendererFacade::legacy(
                &mut renderer,
                Default::default(),
                Default::default()
            )
            .render_menu(&crate::frame::MenuFrame::static_color(&batch, [0.; 4]))
            .unwrap(),
            outcome
        );
        if outcome == FrameOutcome::Offscreen {
            assert_eq!(renderer.read_rgba([40, 24, 1, 1]).unwrap(), [255; 4]);
        }
    }
    #[test]
    #[ignore = "requires Mesa OSMesa and GL2 overrides"]
    fn boot_resources_checked_stage_redraws_resizes_and_commits_once() {
        exercise("osmesa", FrameOutcome::Offscreen);
    }
    #[test]
    #[ignore = "requires visible GLX/Xvfb and GL2 overrides"]
    fn visible_boot_checked_stage_redraws_and_preserves_the_actual_window() {
        exercise("glx", FrameOutcome::Presented);
    }
    #[test]
    #[ignore = "requires Mesa OSMesa and GL2 overrides"]
    fn boot_resources_cancel_each_boundary_without_font_publication() {
        for stop in 0..10 {
            let boot = HudAtlas::boot();
            let gui = HudAtlas::boot();
            let epoch = gui.font.epoch();
            let limits = Limits::default();
            let context = Context::new("osmesa", [64; 2]).unwrap();
            let screen = LoadingScreen::owned(context, &boot, limits.gpu).unwrap();
            // This zero-payload peer keeps only the same context alive, so its
            // usability can be verified after BOTH cancelled owners drop.
            let mut observer = screen.backend.staging_replacement().unwrap();
            let mut calls = 0;
            let result = screen.finish_assets(
                &crate::legacy_gfx::test_assets(),
                &gui,
                ResourceGeneration(10),
                limits,
                preparation(),
                &mut |screen, _| {
                    let n = calls;
                    calls += 1;
                    if n == stop {
                        return Err("stop-stage-test".into());
                    }
                    fill(screen, &boot);
                    Ok(())
                },
            );
            assert!(result.err().unwrap().contains("stop-stage-test"));
            assert_eq!(calls, stop + 1);
            assert_eq!(gui.font.epoch(), epoch);
            observer
                .render(&Frame {
                    view_projection: IDENTITY,
                    camera: [0.; 3],
                    fog: None,
                    clear: Clear::ColorDepth([1., 0., 0., 1.]),
                    draws: &[],
                })
                .unwrap();
            assert_eq!(
                observer.read_rgba([32, 32, 1, 1]).unwrap(),
                [255, 0, 0, 255]
            );
        }
    }
    #[test]
    #[ignore = "requires Mesa OSMesa and GL2 overrides"]
    fn boot_resources_joint_budget_and_changed_font_reject_before_commit() {
        for changed_font in [false, true] {
            let boot = HudAtlas::boot();
            let gui = HudAtlas::boot();
            let epoch = gui.font.epoch();
            let limits = if changed_font {
                Limits::default()
            } else {
                Limits {
                    gpu: Budget {
                        texture_bytes: 5 << 20,
                        mesh_bytes: 2 << 20,
                    },
                    ..Limits::default()
                }
            };
            let screen =
                LoadingScreen::owned(Context::new("osmesa", [64; 2]).unwrap(), &boot, limits.gpu)
                    .unwrap();
            let result = screen.finish_assets(
                &crate::legacy_gfx::test_assets(),
                &gui,
                ResourceGeneration(11),
                limits,
                ResourceLimits::default(),
                &mut |_, point| {
                    if changed_font && point == StagePoint::Ready {
                        gui.font.reset_cache();
                    }
                    Ok(())
                },
            );
            let error = result.err().unwrap();
            if changed_font {
                assert!(error.contains("font changed"), "{error}");
            } else {
                assert!(error.contains("GPU budget"), "{error}");
                assert_eq!(gui.font.epoch(), epoch);
            }
        }
    }
}
