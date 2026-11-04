//! Methods of the OpenGL 2 renderer that do nothing yet: each is a feature of the wgpu renderer that
//! the OpenGL 2 path does not draw until its milestone (docs/plans/opengl21-radeon-x1650.md). The
//! enum forwards them here, so the game never asks which backend is drawing. Generated from the
//! signatures of `gfx::Renderer` by the same generator as `renderer.rs`; keep the two in step.
#![allow(unused_variables)]

use std::sync::Arc;
use std::time::Duration;

use glam::DVec3;
use winit::window::Window;

use crate::Result;
use crate::block_entity_render::{StaticCacheFrameStats, StaticSectionRevision};
use crate::fast_hash::FxHashSet;
use crate::gfx::compositor::Compositor;
use crate::gfx::{HdrCalibration, block_light};
use crate::lod::TileKey;
use crate::mesher::SectionMesh;
use super::renderer::Renderer;

impl Renderer {
    pub fn lod_quads(&self) -> u64 { Default::default() }

    pub fn lod_levels(&self) -> Vec<[u64; 2]> { Default::default() }

    pub fn section_vertex_bytes(&self) -> [u64; 2] { Default::default() }

    pub fn gpu_census(&self) -> Vec<(&'static str, u64)> { Default::default() }

    pub fn gpu_memory_by_label(&self) -> Option<(u64, u64, Vec<(String, u64)>)> { Default::default() }

    pub fn cpu_census(&self) -> [(&'static str, usize); 4] { Default::default() }

    pub fn set_lines_shader(&mut self, _program: Option<&horizon_shaders::CoreProgram>) {}

    pub fn release_surface(&mut self) {}

    pub fn recreate_surface(&mut self) -> Result<()> { Ok(()) }

    pub fn set_present_mode(&mut self, _vsync: bool, _mode: crate::graphics::PresentMode) {}

    pub fn flat_draws(&self) -> bool { Default::default() }

    pub fn set_flat_draws(&mut self, _on: bool) -> Result<()> { Ok(()) }

    pub fn wants_water_plane(&self) -> bool { Default::default() }

    pub fn section_sides(&self) -> bool { Default::default() }

    pub fn set_section_sides(&mut self, _on: bool) {}

    pub fn section_cull(&self) -> bool { Default::default() }

    pub fn set_section_cull(&mut self, _on: bool) -> Result<()> { Ok(()) }

    pub fn culled_sections(&self) -> Option<([usize; 2], f32)> { Default::default() }

    pub fn gpu_culling(&self) -> bool { Default::default() }

    pub fn gpu_frame(&self) -> bool { Default::default() }

    pub fn set_gpu_culling(&mut self, _on: bool) -> Result<()> { Ok(()) }

    pub fn has_light_colors(&self) -> bool { Default::default() }

    pub fn light_color(&self, _name: &str) -> Option<[f32; 3]> { Default::default() }

    pub fn particle_lights(&self) -> Option<&[horizon_shader_api::manifest::ParticleLight]> { Default::default() }

    pub fn native_shadow(&self) -> bool { Default::default() }

    pub fn pack_shadow_casters(&self) -> (bool, bool) { Default::default() }

    pub fn pack_entity_shadows(&self) -> bool { Default::default() }

    pub fn start_occlusion(&mut self, _camera: DVec3, _distance: f32) {}

    pub fn block_entity_revision(&self) -> u64 { Default::default() }

    pub fn block_entity_section_revision(&self, _section: [i32; 3]) -> Option<StaticSectionRevision> { Default::default() }

    pub fn static_block_entity_frame_stats(&self) -> StaticCacheFrameStats { Default::default() }

    pub fn take_dropped_sections(&mut self) -> bool { Default::default() }

    pub fn set_terrain_loading(&mut self, _loading: bool) {}

    pub fn lod_detail_revision(&self) -> u64 { Default::default() }

    pub fn set_hdr_calibration(&mut self, _calibration: Option<HdrCalibration>) {}

    pub fn hdr_on(&self) -> bool { Default::default() }

    pub fn hdr_line(&self) -> String { Default::default() }

    pub fn set_hdr(&mut self, _on: bool, _paper_white: f32, _peak: f32, _black: f32, _ui: f32, _marker: &std::path::Path) {}

    pub fn measure_emission(&mut self) {}

    pub fn display_changed(&mut self) -> bool { Default::default() }

    pub fn skinned_models(&self) -> Option<u32> { Default::default() }

    pub fn skins_entities(&self) -> bool { Default::default() }

    pub fn skins_tagged(&self) -> bool { Default::default() }

    pub fn frame_generation_share(&self) -> Option<f32> { Default::default() }

    pub fn set_frame_generation(&mut self, _on: bool) {}

    pub fn set_world_post(&mut self, _scale: f32, _antialias: bool, _taa: bool, _upscale: f32) {}

    pub fn set_shader_pack(&mut self, _dimension: i32, _pack: Option<horizon_shaders::Post>, _block_ids: &[i32]) -> Option<(Vec<String>, Vec<String>)> { Default::default() }

    pub fn drop_shader_packs(&mut self) {}

    pub fn set_separate_ao(&mut self, _separate_ao: bool) {}

    pub fn set_native_shader_pack(&mut self, _pack: Option<(horizon_shader_api::Report, Vec<(String, f64)>, Vec<(u32, u32)>, Vec<(u32, u32)>, block_light::LightTable)>) -> std::result::Result<Vec<String>, String> { Ok(Default::default()) }

    pub fn pack_inputs(&mut self) -> horizon_shaders::GameInputs { Default::default() }

    pub fn set_pass_timing(&mut self, _on: bool) {}

    pub fn shader_pass_times(&self) -> Vec<(String, f32)> { Default::default() }

    pub fn own_pass_times(&self) -> Vec<(String, f32)> { Default::default() }

    pub fn define_pack_macros(&self, _macros: &mut horizon_shaders::Macros) {}

    pub fn shader_pack_dimension(&self) -> Option<i32> { Default::default() }

    pub fn set_texture_filtering(&mut self, _filtering: i32, _bit: i32) {}

    pub fn sprite_split(&self) -> u32 { Default::default() }

    pub fn set_mipmap_levels(&mut self, _levels: u32) {}

    pub fn device_lost(&self) -> bool { Default::default() }

    pub fn wait_in_flight(&mut self) {}

    pub fn keep_capture_copies(&mut self, _on: bool) {}

    pub fn keeps_capture_copies(&self) -> bool { Default::default() }

    pub fn compositor(&self, _window: Arc<Window>) -> Result<Compositor> { Err("the split-screen compositor draws with the wgpu renderer only".into()) }

    pub fn offscreen_frame(&self) -> Option<&wgpu::Texture> { Default::default() }

    pub fn take_back(&self, _compositor: Compositor) {}

    pub fn upload_lod(&mut self, _key: TileKey, _mesh: &SectionMesh, _keep: bool) {}

    pub fn remove_lod(&mut self, _key: TileKey) {}

    pub fn clear_lods(&mut self) {}

    pub fn queue_lods(&mut self, _meshes: Vec<(TileKey, SectionMesh)>, _removed: Vec<TileKey>, _cutout: Option<FxHashSet<(i32, i32)>>) {}

    pub fn upload_lods(&mut self, _budget: Duration) {}

    pub fn set_lod_cutout(&mut self, _cutout: crate::fast_hash::FxHashSet<(i32, i32)>) {}
}
