//! Asset-free loading HUD on a borrowed native context. Dropping this screen
//! releases all boot GPU resources, not the window/context handed to Game.
use crate::{
    gui_bridge,
    hud::{HudAtlas, HudBatch},
};
pub use horizon_legacy_render::Context;
use horizon_legacy_render::{Budget, Renderer};
use horizon_render_api::{
    Clear, Frame, IDENTITY, Image, RenderBackend, TextureId, gui, render_contract::FrameOutcome,
};

pub struct LoadingScreen<'a> {
    backend: Renderer<'a>,
    atlas: TextureId,
    white: TextureId,
    atlas_size: [u32; 2],
    uv_scale: [f32; 2],
}
impl<'a> LoadingScreen<'a> {
    pub fn new(context: &'a Context, hud: &HudAtlas) -> Result<Self, String> {
        let mut backend = Renderer::new(
            context,
            Budget {
                texture_bytes: 8 << 20,
                mesh_bytes: 2 << 20,
            },
        )?;
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
            atlas_size: [hud.width, hud.height],
            uv_scale,
        })
    }
    pub fn draw(&mut self, batch: &HudBatch) -> Result<FrameOutcome, String> {
        let size = self.backend.context().size();
        if size.contains(&0) {
            return Ok(FrameOutcome::Deferred);
        }
        // Reject an unsupported material before changing the framebuffer.
        let runs = gui_bridge::runs(batch, size, self.uv_scale, 32768)
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
