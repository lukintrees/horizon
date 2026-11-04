//! C01 common submission boundary. Intentionally borrowed during R02 migration:
//! Game still owns its modern renderer; no Deref or public GPU fields here.
//! Submissions use the CPU Frame contract; gfx::Frame remains a compatibility export.
use crate::{Result, frame::Frame, gfx, mesher::SectionMesh};
use horizon_render_api::{
    render_contract::{Backend, Capabilities, Feature, FrameOutcome, ScreenshotRequest},
    scene::SectionKey,
};

pub struct RendererFacade<'a> {
    inner: Adapter<'a>,
}
enum Adapter<'a> {
    Modern(&'a mut gfx::Renderer),
    #[cfg(feature = "legacy-gl")]
    Legacy {
        renderer: &'a mut crate::legacy_gfx::TerrainHudRenderer,
        visible: crate::legacy_gfx::frame::VisibleSections<'a>,
        options: crate::legacy_gfx::frame::Options,
    },
}
impl<'a> RendererFacade<'a> {
    pub fn modern(renderer: &'a mut gfx::Renderer) -> Self {
        Self {
            inner: Adapter::Modern(renderer),
        }
    }
    #[cfg(feature = "legacy-gl")]
    pub fn legacy(
        renderer: &'a mut crate::legacy_gfx::TerrainHudRenderer,
        visible: crate::legacy_gfx::frame::VisibleSections<'a>,
        options: crate::legacy_gfx::frame::Options,
    ) -> Self {
        Self {
            inner: Adapter::Legacy {
                renderer,
                visible,
                options,
            },
        }
    }
    pub fn backend(&self) -> Backend {
        match self.inner {
            Adapter::Modern(_) => Backend::Modern,
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { .. } => Backend::LegacyGl,
        }
    }
    pub fn size(&mut self) -> [u32; 2] {
        match &mut self.inner {
            Adapter::Modern(renderer) => [renderer.config.width, renderer.config.height],
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => renderer.size(),
        }
    }
    pub fn capabilities(&self) -> Capabilities {
        let backend = self.backend();
        Capabilities {
            backend,
            features: match backend {
                Backend::Modern => &[Feature::Panorama, Feature::GpuPoses, Feature::ShaderPacks],
                Backend::LegacyGl => &[],
            },
        }
    }
    /// Modern render already presents internally. Legacy submits then presents
    /// once; offscreen success is distinct from a displayed window frame.
    pub fn render(&mut self, frame: &Frame<'_>) -> Result<FrameOutcome> {
        match &mut self.inner {
            Adapter::Modern(renderer) => renderer.render(frame).map(|drawn| {
                if drawn && renderer.is_offscreen() {
                    FrameOutcome::Offscreen
                } else if drawn {
                    FrameOutcome::Presented
                } else {
                    FrameOutcome::Deferred
                }
            }),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy {
                renderer,
                visible,
                options,
            } => {
                if renderer.size().contains(&0) {
                    return Ok(FrameOutcome::Deferred);
                }
                renderer.render_game_frame(frame, *visible, *options)?;
                Ok(if renderer.present()? {
                    FrameOutcome::Presented
                } else if renderer.is_windowed() {
                    FrameOutcome::Deferred
                } else {
                    FrameOutcome::Offscreen
                })
            }
        }
    }
    /// Queue a screenshot; the next rendered frame writes it. The legacy
    /// adapter has no readback yet (U01), so it rejects the request rather
    /// than silently dropping it.
    pub fn request_screenshot(&mut self, request: ScreenshotRequest) -> Result<()> {
        match &mut self.inner {
            Adapter::Modern(renderer) => {
                renderer.screenshot = Some(request);
                Ok(())
            }
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { .. } => {
                Err("legacy GL2 screenshot readback is not implemented (U01)".into())
            }
        }
    }
    /// Whether a screenshot request waits for the next frame.
    pub fn screenshot_pending(&self) -> bool {
        match &self.inner {
            Adapter::Modern(renderer) => renderer.screenshot.is_some(),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { .. } => false,
        }
    }
    /// Takes the waiting request without drawing it: a replacement renderer
    /// (pack reload, device recovery) carries it over to the new one.
    pub fn take_screenshot_request(&mut self) -> Option<ScreenshotRequest> {
        match &mut self.inner {
            Adapter::Modern(renderer) => renderer.screenshot.take(),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { .. } => None,
        }
    }
    /// Puts back a finished screenshot the old renderer held when it was replaced.
    pub fn restore_screenshot_result(&mut self, result: std::result::Result<[u32; 2], String>) {
        match &mut self.inner {
            Adapter::Modern(renderer) => renderer.screenshot_result = Some(result),
            // The legacy adapter produces no result, so there is none to carry.
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { .. } => {}
        }
    }
    /// A finished screenshot's size, or why it failed; each result is reported once.
    pub fn take_screenshot_result(&mut self) -> Option<std::result::Result<[u32; 2], String>> {
        match &mut self.inner {
            Adapter::Modern(renderer) => renderer.screenshot_result.take(),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { .. } => None,
        }
    }
    pub fn resize(&mut self, size: [u32; 2]) -> Result<()> {
        match &mut self.inner {
            Adapter::Modern(renderer) => renderer.resize(size[0], size[1]),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => {
                // The native owner reports minimize via size=0. Explicit zero
                // backing allocations are rejected, never silently ignored.
                renderer.resize(size)?;
            }
        }
        Ok(())
    }
    pub fn upload_section(&mut self, key: SectionKey, mesh: &SectionMesh) -> Result<()> {
        match &mut self.inner {
            Adapter::Modern(renderer) => renderer.upload((key[0], key[1], key[2]), mesh),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => renderer.upload_section(key, mesh)?,
        }
        Ok(())
    }
}

/// Compatibility seam while Game still owns concrete modern state. Keeps
/// modern errors intact and its existing offscreen/frame_drawn bool semantics.
pub fn render_modern(renderer: &mut gfx::Renderer, frame: &Frame<'_>) -> Result<bool> {
    RendererFacade::modern(renderer)
        .render(frame)
        .map(|outcome| outcome != FrameOutcome::Deferred)
}

/// The drawable size, without making Game read `SurfaceConfiguration`. Read
/// only: a legacy window has no Game-owned size until G01.
pub fn drawable_size(renderer: &gfx::Renderer) -> [u32; 2] {
    [renderer.config.width, renderer.config.height]
}

#[cfg(all(test, feature = "legacy-gl", target_os = "linux"))]
mod tests;
