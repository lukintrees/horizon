//! C01 common submission boundary. Intentionally borrowed during R02 migration:
//! Game still owns its modern renderer; no Deref or public GPU fields here.
//! The real Frame is temporarily gfx::Frame (R01 extraction remains separate).
use crate::{Result, gfx, mesher::SectionMesh};
use horizon_render_api::{
    render_contract::{Backend, Capabilities, Feature, FrameOutcome},
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
    pub fn render(&mut self, frame: &gfx::Frame<'_>) -> Result<FrameOutcome> {
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
pub fn render_modern(renderer: &mut gfx::Renderer, frame: &gfx::Frame<'_>) -> Result<bool> {
    RendererFacade::modern(renderer)
        .render(frame)
        .map(|outcome| outcome != FrameOutcome::Deferred)
}

#[cfg(all(test, feature = "legacy-gl", target_os = "linux"))]
mod tests;
