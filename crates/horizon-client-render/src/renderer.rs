//! The renderer the game draws with: wgpu (`gfx::Renderer`), or OpenGL 2 (`gl2::Renderer`) on GPUs
//! that have no OpenGL 3.3, such as the Radeon X1650 under Windows 7 (docs/plans/opengl21-radeon-x1650.md).
//!
//! The game talks to this enum. Every method both backends share is forwarded to whichever one is
//! drawing; the wgpu-only ones are reached through `wgpu()` and `wgpu_mut()`.

use std::sync::Arc;

use glam::{DVec3, Mat4};
use winit::window::Window;

use crate::Result;
use crate::block_entity_render::{StaticCacheFrameStats, StaticSectionRevision};
use crate::fast_hash::FxHashSet;
use crate::gfx::compositor::Compositor;
use crate::gfx::{
    self, Frame, Frustum, HdrCalibration, PreparedTextures, ScreenshotRequest, SectionKey, ViewSet,
    block_light, panorama,
};
use crate::gl2;
use crate::graphics::GraphicsApi;
use crate::lod::TileKey;
use crate::mesher::SectionMesh;
use crate::sprites::SpriteAnimation;
use std::time::Duration;

pub enum Renderer {
    Wgpu(Box<gfx::Renderer>),
    Gl2(Box<gl2::Renderer>),
}

impl Renderer {
    /// Creates the wgpu renderer. Under `GraphicsApi::Auto` a failure on Windows is tried once more
    /// on OpenGL 2: a GPU whose OpenGL is older than 3.3 cannot run wgpu's backends, and the window
    /// still gets its context. The wgpu error is returned when that fails too.
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        window: Arc<Window>,
        offscreen: bool,
        sprite_pixels: &[u8],
        sprite_size: u32,
        sprite_minimum_size: u32,
        layers: u32,
        sprite_mips: &[crate::mipmap::Settings],
        animations: &[SpriteAnimation],
        hud_atlas: (&[u8], u32, u32),
        glint: (&[u8], u32, u32),
        glint_armor: (&[u8], u32, u32),
        forcefield: (&[u8], u32),
        panorama_faces: Arc<panorama::Faces>,
        prepared: Option<PreparedTextures>,
        previous: Option<&Self>,
        api: GraphicsApi,
        translucent_batching: bool,
        shader_pack: bool,
    ) -> Result<Self> {
        let previous = previous.and_then(Self::wgpu);
        let fallback = !offscreen && matches!(api, GraphicsApi::Auto) && cfg!(windows);
        let attempt = gfx::Renderer::new(
            Arc::clone(&window),
            offscreen,
            sprite_pixels,
            sprite_size,
            sprite_minimum_size,
            layers,
            sprite_mips,
            animations,
            hud_atlas,
            glint,
            glint_armor,
            forcefield,
            panorama_faces,
            prepared,
            previous,
            api,
            translucent_batching,
            shader_pack,
        )
        .await;
        match attempt {
            Ok(renderer) => Ok(Self::Wgpu(Box::new(renderer))),
            Err(error) if fallback => {
                match gl2::Renderer::new(&window, sprite_pixels, sprite_size, layers, hud_atlas) {
                    Ok(renderer) => {
                        log::warn!("the wgpu renderer failed ({error}); drawing with OpenGL 2");
                        Ok(Self::Gl2(Box::new(renderer)))
                    }
                    Err(gl_error) => {
                        log::info!("OpenGL 2 is not available either: {gl_error}");
                        Err(error)
                    }
                }
            }
            Err(error) => Err(error),
        }
    }

    /// The wgpu renderer, when it is the one drawing.
    pub fn wgpu(&self) -> Option<&gfx::Renderer> {
        match self {
            Self::Wgpu(r) => Some(&**r),
            Self::Gl2(_) => None,
        }
    }

    /// The wgpu renderer, mutably, when it is the one drawing.
    pub fn wgpu_mut(&mut self) -> Option<&mut gfx::Renderer> {
        match self {
            Self::Wgpu(r) => Some(&mut **r),
            Self::Gl2(_) => None,
        }
    }

    /// The surface's size in pixels.
    pub fn surface_size(&self) -> [u32; 2] {
        match self {
            Self::Wgpu(r) => [r.config.width, r.config.height],
            Self::Gl2(r) => r.surface_size(),
        }
    }

    /// The GPU's description. OpenGL 2 has none to give.
    pub fn adapter_info(&self) -> Option<&wgpu::AdapterInfo> {
        match self {
            Self::Wgpu(r) => Some(&r.info),
            Self::Gl2(_) => None,
        }
    }

    /// Sections drawn by the last frame.
    pub fn drawn_sections(&self) -> usize {
        match self {
            Self::Wgpu(r) => r.drawn_sections,
            Self::Gl2(r) => r.drawn_sections(),
        }
    }

    pub fn set_gamma(&mut self, gamma: f32) {
        match self {
            Self::Wgpu(r) => r.gamma = gamma,
            Self::Gl2(r) => r.gamma = gamma,
        }
    }

    pub fn set_fullbright(&mut self, on: bool) {
        match self {
            Self::Wgpu(r) => r.fullbright = on,
            Self::Gl2(r) => r.fullbright = on,
        }
    }

    pub fn set_paced_elsewhere(&mut self, on: bool) {
        match self {
            Self::Wgpu(r) => r.paced_elsewhere = on,
            Self::Gl2(r) => r.paced_elsewhere = on,
        }
    }

    /// Asks for a screenshot of the next frame. OpenGL 2 has none yet, so its answer is an error.
    pub fn request_screenshot(&mut self, request: ScreenshotRequest) {
        match self {
            Self::Wgpu(r) => r.screenshot = Some(request),
            Self::Gl2(r) => r.screenshot_result = Some(Err(gl2::SCREENSHOT_UNSUPPORTED.into())),
        }
    }

    /// Takes the pending screenshot request, to hand it to a renderer that replaces this one.
    pub fn take_screenshot(&mut self) -> Option<ScreenshotRequest> {
        match self {
            Self::Wgpu(r) => r.screenshot.take(),
            Self::Gl2(_) => None,
        }
    }

    /// Sets the pending screenshot request (the world icon's, or a request carried over).
    pub fn set_screenshot(&mut self, request: Option<ScreenshotRequest>) {
        match self {
            Self::Wgpu(r) => r.screenshot = request,
            Self::Gl2(_) => {}
        }
    }

    /// Sets the answer to the last screenshot request.
    pub fn set_screenshot_result(&mut self, result: Option<std::result::Result<[u32; 2], String>>) {
        match self {
            Self::Wgpu(r) => r.screenshot_result = result,
            Self::Gl2(r) => r.screenshot_result = result,
        }
    }

    /// Whether a requested screenshot is still waiting for its frame.
    pub fn screenshot_pending(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.screenshot.is_some(),
            Self::Gl2(_) => false,
        }
    }

    /// The answer to the last screenshot request, once.
    pub fn take_screenshot_result(&mut self) -> Option<std::result::Result<[u32; 2], String>> {
        match self {
            Self::Wgpu(r) => r.screenshot_result.take(),
            Self::Gl2(r) => r.screenshot_result.take(),
        }
    }

    /// Block entities of the sections that are loaded (wgpu only; OpenGL 2 has none yet).
    pub fn block_entities(&self) -> Box<dyn Iterator<Item = &([i32; 3], u32)> + '_> {
        match self {
            Self::Wgpu(r) => Box::new(r.block_entities()),
            Self::Gl2(_) => Box::new(std::iter::empty()),
        }
    }

    pub fn visible_block_entities(
        &self,
        fade_seconds: f32,
    ) -> Box<dyn Iterator<Item = &([i32; 3], u32)> + '_> {
        match self {
            Self::Wgpu(r) => Box::new(r.visible_block_entities(fade_seconds)),
            Self::Gl2(_) => Box::new(std::iter::empty()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn visible_block_entities_in_frustum<'a>(
        &'a self,
        fade_seconds: f32,
        camera: DVec3,
        frustum: Option<Frustum>,
        keep_section: impl Fn(u32) -> bool + 'a,
        keep_long_range: impl Fn(u32) -> bool + 'a,
    ) -> Box<dyn Iterator<Item = (&'a ([i32; 3], u32), bool)> + 'a> {
        match self {
            Self::Wgpu(r) => Box::new(r.visible_block_entities_in_frustum(
                fade_seconds,
                camera,
                frustum,
                keep_section,
                keep_long_range,
            )),
            Self::Gl2(_) => Box::new(std::iter::empty()),
        }
    }

    /// The columns of the detail level of detail (wgpu only).
    pub fn lod_detail_columns(&self) -> Box<dyn Iterator<Item = (i32, i32)> + '_> {
        match self {
            Self::Wgpu(r) => Box::new(r.lod_detail_columns()),
            Self::Gl2(_) => Box::new(std::iter::empty()),
        }
    }

    /// The CPU time the frame waited on the GPU (the wgpu path only; OpenGL 2 waits in the driver).
    pub fn last_wait(&self) -> Duration {
        match self {
            Self::Wgpu(r) => r.last_wait,
            Self::Gl2(_) => Duration::ZERO,
        }
    }

    /// The CPU time spent encoding the frame's commands (the wgpu path only).
    pub fn last_encode(&self) -> Duration {
        match self {
            Self::Wgpu(r) => r.last_encode,
            Self::Gl2(_) => Duration::ZERO,
        }
    }

    pub fn resident_sections(&self) -> usize {
        match self {
            Self::Wgpu(r) => r.resident_sections(),
            Self::Gl2(r) => r.resident_sections(),
        }
    }

    pub fn lod_quads(&self) -> u64 {
        match self {
            Self::Wgpu(r) => r.lod_quads(),
            Self::Gl2(r) => r.lod_quads(),
        }
    }

    pub fn lod_levels(&self) -> Vec<[u64; 2]> {
        match self {
            Self::Wgpu(r) => r.lod_levels(),
            Self::Gl2(r) => r.lod_levels(),
        }
    }

    pub fn section_vertex_bytes(&self) -> [u64; 2] {
        match self {
            Self::Wgpu(r) => r.section_vertex_bytes(),
            Self::Gl2(r) => r.section_vertex_bytes(),
        }
    }

    pub fn gpu_census(&self) -> Vec<(&'static str, u64)> {
        match self {
            Self::Wgpu(r) => r.gpu_census(),
            Self::Gl2(r) => r.gpu_census(),
        }
    }

    pub fn gpu_memory_by_label(&self) -> Option<(u64, u64, Vec<(String, u64)>)> {
        match self {
            Self::Wgpu(r) => r.gpu_memory_by_label(),
            Self::Gl2(r) => r.gpu_memory_by_label(),
        }
    }

    pub fn cpu_census(&self) -> [(&'static str, usize); 4] {
        match self {
            Self::Wgpu(r) => r.cpu_census(),
            Self::Gl2(r) => r.cpu_census(),
        }
    }

    pub fn set_lines_shader(&mut self, program: Option<&horizon_shaders::CoreProgram>) {
        match self {
            Self::Wgpu(r) => r.set_lines_shader(program),
            Self::Gl2(r) => r.set_lines_shader(program),
        }
    }

    pub fn release_surface(&mut self) {
        match self {
            Self::Wgpu(r) => r.release_surface(),
            Self::Gl2(r) => r.release_surface(),
        }
    }

    pub fn recreate_surface(&mut self) -> Result<()> {
        match self {
            Self::Wgpu(r) => r.recreate_surface(),
            Self::Gl2(r) => r.recreate_surface(),
        }
    }

    pub fn set_present_mode(&mut self, vsync: bool, mode: crate::graphics::PresentMode) {
        match self {
            Self::Wgpu(r) => r.set_present_mode(vsync, mode),
            Self::Gl2(r) => r.set_present_mode(vsync, mode),
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        match self {
            Self::Wgpu(r) => r.resize(width, height),
            Self::Gl2(r) => r.resize(width, height),
        }
    }

    pub fn flat_draws(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.flat_draws(),
            Self::Gl2(r) => r.flat_draws(),
        }
    }

    pub fn set_flat_draws(&mut self, on: bool) -> Result<()> {
        match self {
            Self::Wgpu(r) => r.set_flat_draws(on),
            Self::Gl2(r) => r.set_flat_draws(on),
        }
    }

    pub fn wants_water_plane(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.wants_water_plane(),
            Self::Gl2(r) => r.wants_water_plane(),
        }
    }

    pub fn section_sides(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.section_sides(),
            Self::Gl2(r) => r.section_sides(),
        }
    }

    pub fn set_section_sides(&mut self, on: bool) {
        match self {
            Self::Wgpu(r) => r.set_section_sides(on),
            Self::Gl2(r) => r.set_section_sides(on),
        }
    }

    pub fn section_cull(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.section_cull(),
            Self::Gl2(r) => r.section_cull(),
        }
    }

    pub fn set_section_cull(&mut self, on: bool) -> Result<()> {
        match self {
            Self::Wgpu(r) => r.set_section_cull(on),
            Self::Gl2(r) => r.set_section_cull(on),
        }
    }

    pub fn culled_sections(&self) -> Option<([usize; 2], f32)> {
        match self {
            Self::Wgpu(r) => r.culled_sections(),
            Self::Gl2(r) => r.culled_sections(),
        }
    }

    pub fn gpu_culling(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.gpu_culling(),
            Self::Gl2(r) => r.gpu_culling(),
        }
    }

    pub fn gpu_frame(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.gpu_frame(),
            Self::Gl2(r) => r.gpu_frame(),
        }
    }

    pub fn set_gpu_culling(&mut self, on: bool) -> Result<()> {
        match self {
            Self::Wgpu(r) => r.set_gpu_culling(on),
            Self::Gl2(r) => r.set_gpu_culling(on),
        }
    }

    pub fn section_count(&self) -> usize {
        match self {
            Self::Wgpu(r) => r.section_count(),
            Self::Gl2(r) => r.section_count(),
        }
    }

    pub fn has_light_colors(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.has_light_colors(),
            Self::Gl2(r) => r.has_light_colors(),
        }
    }

    pub fn light_color(&self, name: &str) -> Option<[f32; 3]> {
        match self {
            Self::Wgpu(r) => r.light_color(name),
            Self::Gl2(r) => r.light_color(name),
        }
    }

    pub fn particle_lights(&self) -> Option<&[horizon_shader_api::manifest::ParticleLight]> {
        match self {
            Self::Wgpu(r) => r.particle_lights(),
            Self::Gl2(r) => r.particle_lights(),
        }
    }

    pub fn native_shadow(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.native_shadow(),
            Self::Gl2(r) => r.native_shadow(),
        }
    }

    pub fn pack_shadow_casters(&self) -> (bool, bool) {
        match self {
            Self::Wgpu(r) => r.pack_shadow_casters(),
            Self::Gl2(r) => r.pack_shadow_casters(),
        }
    }

    pub fn pack_entity_shadows(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.pack_entity_shadows(),
            Self::Gl2(r) => r.pack_entity_shadows(),
        }
    }

    pub fn has_section(&self, key: SectionKey) -> bool {
        match self {
            Self::Wgpu(r) => r.has_section(key),
            Self::Gl2(r) => r.has_section(key),
        }
    }

    pub fn start_occlusion(&mut self, camera: DVec3, distance: f32) {
        match self {
            Self::Wgpu(r) => r.start_occlusion(camera, distance),
            Self::Gl2(r) => r.start_occlusion(camera, distance),
        }
    }

    pub fn upload(&mut self, key: SectionKey, mesh: &SectionMesh) {
        match self {
            Self::Wgpu(r) => r.upload(key, mesh),
            Self::Gl2(r) => r.upload(key, mesh),
        }
    }

    pub fn block_entity_revision(&self) -> u64 {
        match self {
            Self::Wgpu(r) => r.block_entity_revision(),
            Self::Gl2(r) => r.block_entity_revision(),
        }
    }

    pub fn block_entity_section_revision(
        &self,
        section: [i32; 3],
    ) -> Option<StaticSectionRevision> {
        match self {
            Self::Wgpu(r) => r.block_entity_section_revision(section),
            Self::Gl2(r) => r.block_entity_section_revision(section),
        }
    }

    pub fn static_block_entity_frame_stats(&self) -> StaticCacheFrameStats {
        match self {
            Self::Wgpu(r) => r.static_block_entity_frame_stats(),
            Self::Gl2(r) => r.static_block_entity_frame_stats(),
        }
    }

    pub fn clear_sections(&mut self, layout: crate::chunk::ColumnLayout) {
        match self {
            Self::Wgpu(r) => r.clear_sections(layout),
            Self::Gl2(r) => r.clear_sections(layout),
        }
    }

    pub fn take_dropped_sections(&mut self) -> bool {
        match self {
            Self::Wgpu(r) => r.take_dropped_sections(),
            Self::Gl2(r) => r.take_dropped_sections(),
        }
    }

    pub fn set_terrain_loading(&mut self, loading: bool) {
        match self {
            Self::Wgpu(r) => r.set_terrain_loading(loading),
            Self::Gl2(r) => r.set_terrain_loading(loading),
        }
    }

    pub fn lod_detail_revision(&self) -> u64 {
        match self {
            Self::Wgpu(r) => r.lod_detail_revision(),
            Self::Gl2(r) => r.lod_detail_revision(),
        }
    }

    pub fn remove_columns(&mut self, columns: impl IntoIterator<Item = (i32, i32)>) {
        match self {
            Self::Wgpu(r) => r.remove_columns(columns),
            Self::Gl2(r) => r.remove_columns(columns),
        }
    }

    pub fn set_hdr_calibration(&mut self, calibration: Option<HdrCalibration>) {
        match self {
            Self::Wgpu(r) => r.set_hdr_calibration(calibration),
            Self::Gl2(r) => r.set_hdr_calibration(calibration),
        }
    }

    pub fn hdr_on(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.hdr_on(),
            Self::Gl2(r) => r.hdr_on(),
        }
    }

    pub fn hdr_line(&self) -> String {
        match self {
            Self::Wgpu(r) => r.hdr_line(),
            Self::Gl2(r) => r.hdr_line(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_hdr(
        &mut self,
        on: bool,
        paper_white: f32,
        peak: f32,
        black: f32,
        ui: f32,
        marker: &std::path::Path,
    ) {
        match self {
            Self::Wgpu(r) => r.set_hdr(on, paper_white, peak, black, ui, marker),
            Self::Gl2(r) => r.set_hdr(on, paper_white, peak, black, ui, marker),
        }
    }

    pub fn measure_emission(&mut self) {
        match self {
            Self::Wgpu(r) => r.measure_emission(),
            Self::Gl2(r) => r.measure_emission(),
        }
    }

    pub fn display_changed(&mut self) -> bool {
        match self {
            Self::Wgpu(r) => r.display_changed(),
            Self::Gl2(r) => r.display_changed(),
        }
    }

    pub fn skinned_models(&self) -> Option<u32> {
        match self {
            Self::Wgpu(r) => r.skinned_models(),
            Self::Gl2(r) => r.skinned_models(),
        }
    }

    pub fn skins_entities(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.skins_entities(),
            Self::Gl2(r) => r.skins_entities(),
        }
    }

    pub fn skins_tagged(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.skins_tagged(),
            Self::Gl2(r) => r.skins_tagged(),
        }
    }

    pub fn frame_generation_share(&self) -> Option<f32> {
        match self {
            Self::Wgpu(r) => r.frame_generation_share(),
            Self::Gl2(r) => r.frame_generation_share(),
        }
    }

    pub fn set_frame_generation(&mut self, on: bool) {
        match self {
            Self::Wgpu(r) => r.set_frame_generation(on),
            Self::Gl2(r) => r.set_frame_generation(on),
        }
    }

    pub fn set_world_post(&mut self, scale: f32, antialias: bool, taa: bool, upscale: f32) {
        match self {
            Self::Wgpu(r) => r.set_world_post(scale, antialias, taa, upscale),
            Self::Gl2(r) => r.set_world_post(scale, antialias, taa, upscale),
        }
    }

    pub fn set_shader_pack(
        &mut self,
        dimension: i32,
        pack: Option<horizon_shaders::Post>,
        block_ids: &[i32],
    ) -> Option<(Vec<String>, Vec<String>)> {
        match self {
            Self::Wgpu(r) => r.set_shader_pack(dimension, pack, block_ids),
            Self::Gl2(r) => r.set_shader_pack(dimension, pack, block_ids),
        }
    }

    pub fn drop_shader_packs(&mut self) {
        match self {
            Self::Wgpu(r) => r.drop_shader_packs(),
            Self::Gl2(r) => r.drop_shader_packs(),
        }
    }

    pub fn set_separate_ao(&mut self, separate_ao: bool) {
        match self {
            Self::Wgpu(r) => r.set_separate_ao(separate_ao),
            Self::Gl2(r) => r.set_separate_ao(separate_ao),
        }
    }

    pub fn set_native_shader_pack(
        &mut self,
        pack: Option<(
            horizon_shader_api::Report,
            Vec<(String, f64)>,
            Vec<(u32, u32)>,
            Vec<(u32, u32)>,
            block_light::LightTable,
        )>,
    ) -> std::result::Result<Vec<String>, String> {
        match self {
            Self::Wgpu(r) => r.set_native_shader_pack(pack),
            Self::Gl2(r) => r.set_native_shader_pack(pack),
        }
    }

    pub fn pack_inputs(&mut self) -> horizon_shaders::GameInputs {
        match self {
            Self::Wgpu(r) => r.pack_inputs(),
            Self::Gl2(r) => r.pack_inputs(),
        }
    }

    pub fn set_pass_timing(&mut self, on: bool) {
        match self {
            Self::Wgpu(r) => r.set_pass_timing(on),
            Self::Gl2(r) => r.set_pass_timing(on),
        }
    }

    pub fn shader_pass_times(&self) -> Vec<(String, f32)> {
        match self {
            Self::Wgpu(r) => r.shader_pass_times(),
            Self::Gl2(r) => r.shader_pass_times(),
        }
    }

    pub fn own_pass_times(&self) -> Vec<(String, f32)> {
        match self {
            Self::Wgpu(r) => r.own_pass_times(),
            Self::Gl2(r) => r.own_pass_times(),
        }
    }

    pub fn define_pack_macros(&self, macros: &mut horizon_shaders::Macros) {
        match self {
            Self::Wgpu(r) => r.define_pack_macros(macros),
            Self::Gl2(r) => r.define_pack_macros(macros),
        }
    }

    pub fn shader_pack_dimension(&self) -> Option<i32> {
        match self {
            Self::Wgpu(r) => r.shader_pack_dimension(),
            Self::Gl2(r) => r.shader_pack_dimension(),
        }
    }

    pub fn set_texture_filtering(&mut self, filtering: i32, bit: i32) {
        match self {
            Self::Wgpu(r) => r.set_texture_filtering(filtering, bit),
            Self::Gl2(r) => r.set_texture_filtering(filtering, bit),
        }
    }

    pub fn sprite_split(&self) -> u32 {
        match self {
            Self::Wgpu(r) => r.sprite_split(),
            Self::Gl2(r) => r.sprite_split(),
        }
    }

    pub fn set_mipmap_levels(&mut self, levels: u32) {
        match self {
            Self::Wgpu(r) => r.set_mipmap_levels(levels),
            Self::Gl2(r) => r.set_mipmap_levels(levels),
        }
    }

    pub fn views(&self, frame: &Frame) -> ViewSet {
        match self {
            Self::Wgpu(r) => r.views(frame),
            Self::Gl2(r) => r.views(frame),
        }
    }

    pub fn camera_frustum(
        &self,
        fov_degrees: f32,
        yaw: f32,
        pitch: f32,
        bob: Mat4,
        depth_far: f32,
    ) -> Frustum {
        match self {
            Self::Wgpu(r) => r.camera_frustum(fov_degrees, yaw, pitch, bob, depth_far),
            Self::Gl2(r) => r.camera_frustum(fov_degrees, yaw, pitch, bob, depth_far),
        }
    }

    pub fn render(&mut self, frame: &Frame) -> Result<bool> {
        match self {
            Self::Wgpu(r) => r.render(frame),
            Self::Gl2(r) => r.render(frame),
        }
    }

    pub fn device_lost(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.device_lost(),
            Self::Gl2(r) => r.device_lost(),
        }
    }

    pub fn wait_in_flight(&mut self) {
        match self {
            Self::Wgpu(r) => r.wait_in_flight(),
            Self::Gl2(r) => r.wait_in_flight(),
        }
    }

    pub fn keep_capture_copies(&mut self, on: bool) {
        match self {
            Self::Wgpu(r) => r.keep_capture_copies(on),
            Self::Gl2(r) => r.keep_capture_copies(on),
        }
    }

    pub fn keeps_capture_copies(&self) -> bool {
        match self {
            Self::Wgpu(r) => r.keeps_capture_copies(),
            Self::Gl2(r) => r.keeps_capture_copies(),
        }
    }

    pub fn compositor(&self, window: Arc<Window>) -> Result<Compositor> {
        match self {
            Self::Wgpu(r) => r.compositor(window),
            Self::Gl2(r) => r.compositor(window),
        }
    }

    pub fn offscreen_frame(&self) -> Option<&wgpu::Texture> {
        match self {
            Self::Wgpu(r) => r.offscreen_frame(),
            Self::Gl2(r) => r.offscreen_frame(),
        }
    }

    pub fn take_back(&self, compositor: Compositor) {
        match self {
            Self::Wgpu(r) => r.take_back(compositor),
            Self::Gl2(r) => r.take_back(compositor),
        }
    }

    pub fn upload_lod(&mut self, key: TileKey, mesh: &SectionMesh, keep: bool) {
        match self {
            Self::Wgpu(r) => r.upload_lod(key, mesh, keep),
            Self::Gl2(r) => r.upload_lod(key, mesh, keep),
        }
    }

    pub fn remove_lod(&mut self, key: TileKey) {
        match self {
            Self::Wgpu(r) => r.remove_lod(key),
            Self::Gl2(r) => r.remove_lod(key),
        }
    }

    pub fn clear_lods(&mut self) {
        match self {
            Self::Wgpu(r) => r.clear_lods(),
            Self::Gl2(r) => r.clear_lods(),
        }
    }

    pub fn queue_lods(
        &mut self,
        meshes: Vec<(TileKey, SectionMesh)>,
        removed: Vec<TileKey>,
        cutout: Option<FxHashSet<(i32, i32)>>,
    ) {
        match self {
            Self::Wgpu(r) => r.queue_lods(meshes, removed, cutout),
            Self::Gl2(r) => r.queue_lods(meshes, removed, cutout),
        }
    }

    pub fn upload_lods(&mut self, budget: Duration) {
        match self {
            Self::Wgpu(r) => r.upload_lods(budget),
            Self::Gl2(r) => r.upload_lods(budget),
        }
    }

    pub fn set_lod_cutout(&mut self, cutout: crate::fast_hash::FxHashSet<(i32, i32)>) {
        match self {
            Self::Wgpu(r) => r.set_lod_cutout(cutout),
            Self::Gl2(r) => r.set_lod_cutout(cutout),
        }
    }
}
