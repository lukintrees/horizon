//! C01 common submission boundary. Intentionally borrowed during R02 migration:
//! Game still owns its modern renderer; no Deref or public GPU fields here.
//! Submissions use the CPU Frame contract; gfx::Frame remains a compatibility export.
pub use crate::frame::{MenuBackground, MenuFrame};
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
    /// Published CPU bundle identity, if this owner was built from one. The
    /// old modern/synthetic constructors do not track it: None is not zero.
    pub fn resource_generation(
        &self,
    ) -> Option<horizon_render_api::render_contract::ResourceGeneration> {
        match &self.inner {
            Adapter::Modern(_) => None,
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => renderer.resource_generation(),
        }
    }
    pub fn capabilities(&self) -> Capabilities {
        let backend = self.backend();
        Capabilities {
            backend,
            features: match backend {
                Backend::Modern => &[
                    Feature::StaticMenu,
                    Feature::Panorama,
                    Feature::GpuPoses,
                    Feature::ShaderPacks,
                ],
                Backend::LegacyGl => &[Feature::StaticMenu],
            },
        }
    }
    /// Explicit native resource-owner tick. Modern samples within its draw path
    /// and rejects this maintenance command rather than advancing twice.
    pub fn advance_animations(
        &mut self,
        generation: horizon_render_api::render_contract::ResourceGeneration,
        tick: u64,
    ) -> Result<usize> {
        match &mut self.inner {
            Adapter::Modern(_) => {
                let _ = (generation, tick);
                Err("modern animation clock belongs to frame submission".into())
            }
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => renderer
                .advance_animations(generation, tick)
                .map_err(Into::into),
        }
    }
    /// Synchronous native resource publication. The opt-in preparation policy
    /// does not replace the modern Game's existing reload path. Callers must
    /// publish matching CPU producers and rebuild UV-bearing batches/sections.
    #[cfg(feature = "legacy-gl")]
    pub fn reload_assets(
        &mut self,
        assets: &crate::assets::RenderAssets,
        hud: &crate::hud::HudAtlas,
        expected: horizon_render_api::render_contract::ResourceGeneration,
        next: horizon_render_api::render_contract::ResourceGeneration,
        tick: u64,
        preparation: crate::legacy_gfx::ResourceLimits,
    ) -> Result<horizon_render_api::render_contract::ResourceGeneration> {
        match &mut self.inner {
            Adapter::Modern(_) => Err("modern resource reload belongs to its Game owner".into()),
            Adapter::Legacy { renderer, .. } => renderer
                .stage_reload_assets(assets, hud, expected, next, tick, preparation)?
                .commit()
                .map_err(Into::into),
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
                renderer.finish_screenshot();
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
    /// Menu policy is explicit and separate from world Frame submission. A
    /// panorama request on legacy fails; static is never an implicit fallback.
    pub fn render_menu(&mut self, menu: &MenuFrame<'_>) -> Result<FrameOutcome> {
        menu.validate()?;
        match &mut self.inner {
            Adapter::Modern(renderer) => {
                menu.validate_target([renderer.config.width, renderer.config.height])?;
                let mut frame = Frame::menu(menu.hud);
                frame.yaw = menu.yaw;
                frame.pitch = menu.pitch;
                let drawn = match menu.background {
                    MenuBackground::Panorama => renderer.render(&frame)?,
                    MenuBackground::Static(clear) => renderer.render_static_menu(&frame, clear)?,
                };
                Ok(if !drawn {
                    FrameOutcome::Deferred
                } else if renderer.is_offscreen() {
                    FrameOutcome::Offscreen
                } else {
                    FrameOutcome::Presented
                })
            }
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => {
                let MenuBackground::Static(clear) = menu.background else {
                    return Err("legacy GL2 panorama is not implemented; select an explicit static menu profile".into());
                };
                if renderer.size().contains(&0) {
                    return Ok(FrameOutcome::Deferred);
                }
                renderer.render_menu(menu.hud, clear)?;
                renderer.finish_screenshot();
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

    /// Queue a screenshot; the next successful frame writes it before present.
    /// Legacy supports bounded native-resolution/centre-cropped SDR PNGs;
    /// supersampling and overwriting an outstanding command are explicit errors.
    pub fn request_screenshot(&mut self, request: ScreenshotRequest) -> Result<()> {
        match &mut self.inner {
            Adapter::Modern(renderer) => {
                renderer.screenshot = Some(request);
                Ok(())
            }
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => {
                renderer.request_screenshot(request).map_err(Into::into)
            }
        }
    }
    /// Whether a screenshot request waits for the next frame.
    pub fn screenshot_pending(&self) -> bool {
        match &self.inner {
            Adapter::Modern(renderer) => renderer.screenshot.is_some(),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => renderer.screenshots.pending.is_some(),
        }
    }
    /// Takes the waiting request without drawing it: a replacement renderer
    /// (pack reload, device recovery) carries it over to the new one.
    pub fn take_screenshot_request(&mut self) -> Option<ScreenshotRequest> {
        match &mut self.inner {
            Adapter::Modern(renderer) => renderer.screenshot.take(),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => renderer.screenshots.pending.take(),
        }
    }
    /// Puts back a finished screenshot the old renderer held when it was replaced.
    pub fn restore_screenshot_result(&mut self, result: std::result::Result<[u32; 2], String>) {
        match &mut self.inner {
            Adapter::Modern(renderer) => renderer.screenshot_result = Some(result),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => renderer.screenshots.result = Some(result),
        }
    }
    /// A finished screenshot's size, or why it failed; each result is reported once.
    pub fn take_screenshot_result(&mut self) -> Option<std::result::Result<[u32; 2], String>> {
        match &mut self.inner {
            Adapter::Modern(renderer) => renderer.screenshot_result.take(),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => renderer.screenshots.result.take(),
        }
    }
    /// Hands the window surface away before a replacement renderer takes it.
    /// The legacy owner keeps its own native surface (G01), so asking here is
    /// an error, not a silent no-op that would leave two owners.
    pub fn release_surface(&mut self) -> Result<()> {
        match &mut self.inner {
            Adapter::Modern(renderer) => {
                renderer.release_surface();
                Ok(())
            }
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { .. } => {
                Err("legacy GL2 surface handover belongs to the native context (G01)".into())
            }
        }
    }
    /// Re-creates the surface a failed handover set aside, or says why not.
    pub fn recreate_surface(&mut self) -> Result<()> {
        match &mut self.inner {
            Adapter::Modern(renderer) => renderer.recreate_surface(),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { .. } => Err("legacy GL2 has no wgpu surface to recreate".into()),
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
    pub fn has_section(&self, key: SectionKey) -> bool {
        match &self.inner {
            Adapter::Modern(renderer) => has_section(renderer, key),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => renderer.has_section(key),
        }
    }
    pub fn resident_sections(&self) -> usize {
        match &self.inner {
            Adapter::Modern(renderer) => renderer.section_count(),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => renderer.resident_sections(),
        }
    }
    /// Reset world membership and vertical origin together. Resource/layout
    /// generations and stale-job rejection remain the W01 owner's responsibility.
    pub fn clear_sections(&mut self, layout: crate::chunk::ColumnLayout) -> Result<()> {
        match &mut self.inner {
            Adapter::Modern(renderer) => renderer.clear_sections(layout),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => renderer.clear_sections(layout)?,
        }
        Ok(())
    }
    /// May partially remove a batch on backend cleanup failure; errors propagate.
    /// A higher-level legacy owner must stop/recover rather than render stale keys.
    pub fn remove_columns(&mut self, columns: impl IntoIterator<Item = (i32, i32)>) -> Result<()> {
        match &mut self.inner {
            Adapter::Modern(renderer) => renderer.remove_columns(columns),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => renderer.remove_columns(columns)?,
        }
        Ok(())
    }
    pub fn upload_section(&mut self, key: SectionKey, mesh: &SectionMesh) -> Result<()> {
        match &mut self.inner {
            Adapter::Modern(renderer) => renderer.upload((key[0], key[1], key[2]), mesh),
            #[cfg(feature = "legacy-gl")]
            Adapter::Legacy { renderer, .. } => renderer.upload_game_section(key, mesh)?,
        }
        Ok(())
    }
}

/// Adapter facts in backend-neutral terms: what far terrain caps on and what
/// `horizon-ctl` reports. Game reads this instead of `wgpu::AdapterInfo`, so a
/// legacy context (G01) can fill the same shape from its GL strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuInfo {
    pub name: String,
    /// PCI vendor id, 0 when the backend has only a string.
    pub vendor: u32,
    pub device_class: DeviceClass,
    /// The graphics API, as `wgpu`/GL name it.
    pub backend: String,
    pub driver: String,
}
/// What kind of GPU the adapter is, in the terms far terrain and the debug
/// overlay care about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceClass {
    Integrated,
    Discrete,
    Virtual,
    Cpu,
    Other,
}
impl DeviceClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Integrated => "integrated",
            Self::Discrete => "discrete",
            Self::Virtual => "virtual",
            Self::Cpu => "cpu",
            Self::Other => "other",
        }
    }
}
/// The adapter's facts, read-only: `horizon-ctl` and logs ask here rather than
/// touching `Renderer` fields.
pub fn gpu_info(renderer: &gfx::Renderer) -> GpuInfo {
    let info = &renderer.info;
    GpuInfo {
        name: info.name.clone(),
        vendor: info.vendor,
        device_class: match info.device_type {
            wgpu::DeviceType::IntegratedGpu => DeviceClass::Integrated,
            wgpu::DeviceType::DiscreteGpu => DeviceClass::Discrete,
            wgpu::DeviceType::VirtualGpu => DeviceClass::Virtual,
            wgpu::DeviceType::Cpu => DeviceClass::Cpu,
            wgpu::DeviceType::Other => DeviceClass::Other,
        },
        backend: format!("{:?}", info.backend),
        driver: format!("{} {}", info.driver, info.driver_info)
            .trim()
            .to_owned(),
    }
}
/// How the backend presents, without Game naming a `wgpu` type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentMode {
    Immediate,
    Mailbox,
    Fifo,
    FifoRelaxed,
    Other,
}
impl PresentMode {
    /// The FPS line's suffix, empty when there is nothing to say.
    pub fn label(self) -> &'static str {
        match self {
            Self::Immediate => " (immediate)",
            Self::Mailbox => " (mailbox)",
            Self::Fifo => " (fifo)",
            Self::FifoRelaxed => " (fifo relaxed)",
            Self::Other => "",
        }
    }
}
/// The mode the surface presents with. Legacy (G01) reports its swap interval.
pub fn present_mode(renderer: &gfx::Renderer) -> PresentMode {
    match renderer.config.present_mode {
        wgpu::PresentMode::Immediate => PresentMode::Immediate,
        wgpu::PresentMode::Mailbox => PresentMode::Mailbox,
        wgpu::PresentMode::Fifo => PresentMode::Fifo,
        wgpu::PresentMode::FifoRelaxed => PresentMode::FifoRelaxed,
        _ => PresentMode::Other,
    }
}
/// `distance` as far terrain applies it on this adapter. Legacy has no adapter
/// class of its own yet, so G01 adds the same cap once it can classify the GPU.
pub fn lod_distance(renderer: &gfx::Renderer, distance: u32) -> u32 {
    crate::graphics::lod_distance_capped(distance, &renderer.info)
}

/// Compatibility seam while Game still owns concrete modern state. Keeps
/// modern errors intact and its existing offscreen/frame_drawn bool semantics.
pub fn render_modern(renderer: &mut gfx::Renderer, frame: &Frame<'_>) -> Result<bool> {
    RendererFacade::modern(renderer)
        .render(frame)
        .map(|outcome| outcome != FrameOutcome::Deferred)
}

/// Read-only membership seam while Game still owns a concrete modern renderer.
/// Section Y is relative to the configured column minimum, as for uploads.
pub fn has_section(renderer: &gfx::Renderer, key: SectionKey) -> bool {
    renderer.has_section((key[0], key[1], key[2]))
}

/// The drawable size, without making Game read `SurfaceConfiguration`. Read
/// only: a legacy window has no Game-owned size until G01.
pub fn drawable_size(renderer: &gfx::Renderer) -> [u32; 2] {
    [renderer.config.width, renderer.config.height]
}

/// Compatibility for Game loading/menu call sites: preserve its historical
/// bool pacing while the public facade distinguishes offscreen completion.
pub fn render_menu_modern(renderer: &mut gfx::Renderer, menu: &MenuFrame<'_>) -> Result<bool> {
    RendererFacade::modern(renderer)
        .render_menu(menu)
        .map(|outcome| outcome != FrameOutcome::Deferred)
}

#[cfg(all(test, feature = "legacy-gl", target_os = "linux"))]
mod tests;
