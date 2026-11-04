//! PR 8: additive facade and settings stubs for the future GL2 renderer.
//!
//! This is deliberately not wired into `game.rs`: the default and `modern-gl`
//! paths continue to use `gfx::Renderer`. The facade mirrors the field and
//! method surface recorded in `CONTRACT.md` with unit-typed fields, so the
//! legacy API can be discussed and tested without wgpu-typed members or a
//! real GL context. No GL calls are made here.

use super::diagnostics::UNAVAILABLE_METHODS;

/// Zero-sized frame placeholder; PR 8 only defines the facade surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame;

impl Frame {
    pub fn empty() -> Self {
        Self
    }
}

macro_rules! noop_methods {
    ($($name:ident),+ $(,)?) => {
        $(
            #[allow(clippy::unused_self)]
            pub fn $name(&self) {}
        )+
    };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Renderer {
    pub add_mod_signs: (),
    pub boot: (),
    pub capture: (),
    pub capture_result: (),
    pub cit: (),
    pub clear_lods: (),
    pub clear_sections: (),
    pub config: (),
    pub culled_sections: (),
    pub camera_frustum: (),
    pub define_pack_macros: (),
    pub device: (),
    pub display_changed: (),
    pub drawn_sections: (),
    pub drop_shader_packs: (),
    pub frame_generation_share: (),
    pub gamma: (),
    pub gpu_culling: (),
    pub gpu_frame: (),
    pub gpu_times: (),
    pub hdr_line: (),
    pub hdr_on: (),
    pub hud: (),
    pub info: (),
    pub last_encode: (),
    pub last_wait: (),
    pub light_color: (),
    pub lod_detail_revision: (),
    pub native_shadow: (),
    pub own_pass_times: (),
    pub pack_entity_shadows: (),
    pub pack_inputs: (),
    pub paced_elsewhere: (),
    pub queue: (),
    pub queue_lods: (),
    pub recreate_surface: (),
    pub release_surface: (),
    pub remove_columns: (),
    pub render: (),
    pub resize: (),
    pub resolve_with_rule: (),
    pub screenshot: (),
    pub screenshot_result: (),
    pub section_draws: (),
    pub section_sides: (),
    pub section_vertex_bytes: (),
    pub set_flat_draws: (),
    pub set_gpu_culling: (),
    pub set_hdr: (),
    pub set_lines_shader: (),
    pub set_mipmap_levels: (),
    pub set_pass_timing: (),
    pub set_present_mode: (),
    pub set_separate_ao: (),
    pub set_shader_pack: (),
    pub set_terrain_loading: (),
    pub set_texture_filtering: (),
    pub set_world_post: (),
    pub shader_pack_dimension: (),
    pub shader_pass_times: (),
    pub skins_entities: (),
    pub skins_tagged: (),
    pub skinned_models: (),
    pub sprite_split: (),
    pub start_occlusion: (),
    pub static_block_entity_frame_stats: (),
    pub title_artwork: (),
    pub upload: (),
    pub upload_lods: (),
    pub visible_block_entities: (),
    pub wait_in_flight: (),
    pub wants_water_plane: (),
    pub world: (),
}

impl Renderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn device_lost() -> bool {
        false
    }

    pub fn pack_shadow_casters() -> u32 {
        0
    }

    pub fn take_dropped_sections() -> Vec<()> {
        Vec::new()
    }

    pub fn unavailable() -> String {
        format!(
            "PR 8 legacy facade: {} unavailable modern methods: {}",
            UNAVAILABLE_METHODS.len(),
            UNAVAILABLE_METHODS.join(", ")
        )
    }

    #[allow(clippy::unused_self)]
    pub fn render(&self, _frame: &Frame) {}

    noop_methods!(
        add_mod_signs,
        boot,
        capture,
        capture_result,
        cit,
        clear_lods,
        clear_sections,
        config,
        culled_sections,
        camera_frustum,
        define_pack_macros,
        device,
        display_changed,
        drawn_sections,
        drop_shader_packs,
        frame_generation_share,
        gamma,
        gpu_culling,
        gpu_frame,
        gpu_times,
        hdr_line,
        hdr_on,
        hud,
        info,
        last_encode,
        last_wait,
        light_color,
        lod_detail_revision,
        native_shadow,
        own_pass_times,
        pack_entity_shadows,
        pack_inputs,
        paced_elsewhere,
        queue,
        queue_lods,
        recreate_surface,
        release_surface,
        remove_columns,
        resize,
        resolve_with_rule,
        screenshot,
        screenshot_result,
        section_draws,
        section_sides,
        section_vertex_bytes,
        set_flat_draws,
        set_gpu_culling,
        set_hdr,
        set_lines_shader,
        set_mipmap_levels,
        set_pass_timing,
        set_present_mode,
        set_separate_ao,
        set_shader_pack,
        set_terrain_loading,
        set_texture_filtering,
        set_world_post,
        shader_pack_dimension,
        shader_pass_times,
        skins_entities,
        skins_tagged,
        skinned_models,
        sprite_split,
        start_occlusion,
        static_block_entity_frame_stats,
        title_artwork,
        upload,
        upload_lods,
        visible_block_entities,
        wait_in_flight,
        wants_water_plane,
        world,
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LegacyGraphicsApi {
    Auto,
    Vulkan,
    Dx12,
    OpenGl,
    LegacyGl,
}

impl LegacyGraphicsApi {
    pub const ALL: [Self; 5] = [
        Self::Auto,
        Self::Vulkan,
        Self::Dx12,
        Self::OpenGl,
        Self::LegacyGl,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Vulkan => "vulkan",
            Self::Dx12 => "dx12",
            Self::OpenGl => "opengl",
            Self::LegacyGl => "legacy-gl",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Vulkan => "Vulkan",
            Self::Dx12 => "DirectX 12",
            Self::OpenGl => "OpenGL (modern)",
            Self::LegacyGl => "Legacy GL 2.1",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "vulkan" => Some(Self::Vulkan),
            "dx12" => Some(Self::Dx12),
            "opengl" => Some(Self::OpenGl),
            "legacy-gl" | "legacy_gl" | "legacygl" | "gl21" | "gl2.1" => Some(Self::LegacyGl),
            _ => None,
        }
    }

    pub fn choices() -> &'static [Self] {
        &Self::ALL
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LegacySettings {
    pub api: LegacyGraphicsApi,
    pub texture_budget_mib: u32,
    pub render_distance: u32,
    pub legacy_gl: bool,
    pub frames: u32,
    pub game: bool,
    pub seed: u64,
    pub world: String,
    pub fov: f32,
    pub sensitivity: f32,
    pub help: bool,
    pub launch_api: bool,
    pub disabled_features: Vec<&'static str>,
}

impl Default for LegacySettings {
    fn default() -> Self {
        Self {
            api: LegacyGraphicsApi::Auto,
            texture_budget_mib: 128,
            render_distance: 8,
            legacy_gl: false,
            frames: 1,
            game: false,
            seed: 0x48_4F_52_49_5A_4F_4E,
            world: String::new(),
            fov: 70.0,
            sensitivity: 0.0025,
            help: false,
            launch_api: false,
            disabled_features: Vec::new(),
        }
    }
}

impl LegacySettings {
    pub fn reference() -> Self {
        Self {
            api: LegacyGraphicsApi::LegacyGl,
            texture_budget_mib: 48,
            render_distance: 4,
            legacy_gl: true,
            frames: 1,
            game: false,
            seed: 0x51DE,
            world: String::new(),
            fov: 70.0,
            sensitivity: 0.0025,
            help: false,
            launch_api: false,
            disabled_features: vec!["HDR", "SSBO", "compute culling", "texture arrays"],
        }
    }

    pub fn startup_note(&self) -> String {
        format!(
            "graphics-api={} budget={}MiB distance={} frames={} game={} seed={} world={} disabled={}",
            self.api.name(),
            self.texture_budget_mib,
            self.render_distance,
            self.frames,
            self.game,
            self.seed,
            if self.world.is_empty() {
                "default"
            } else {
                &self.world
            },
            self.disabled_features.len()
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LegacyArgs {
    pub settings: LegacySettings,
    pub unknown: Vec<String>,
}

impl LegacyArgs {
    pub fn parse(args: &[String]) -> Self {
        let mut settings = LegacySettings::default();
        let mut unknown = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--legacy-gl" => {
                    settings.legacy_gl = true;
                    settings.api = LegacyGraphicsApi::LegacyGl;
                }
                "--graphics-api" => {
                    if let Some(value) = iter.next() {
                        if let Some(api) = LegacyGraphicsApi::from_name(value) {
                            settings.api = api;
                            settings.legacy_gl = api == LegacyGraphicsApi::LegacyGl;
                        } else {
                            unknown.push(value.clone());
                        }
                    }
                }
                "--texture-budget-mib" => {
                    if let Some(value) = iter.next() {
                        match value.parse::<u32>() {
                            Ok(budget) => settings.texture_budget_mib = budget,
                            Err(_) => unknown.push(value.clone()),
                        }
                    }
                }
                "--frames" => {
                    if let Some(value) = iter.next() {
                        match value.parse::<u32>() {
                            Ok(frames) => settings.frames = frames,
                            Err(_) => unknown.push(value.clone()),
                        }
                    }
                }
                "--game" => settings.game = true,
                "--seed" => {
                    if let Some(value) = iter.next() {
                        match value.parse::<u64>() {
                            Ok(seed) => settings.seed = seed,
                            Err(_) => unknown.push(value.clone()),
                        }
                    }
                }
                "--world" => {
                    if let Some(value) = iter.next() {
                        settings.world = value.clone();
                    }
                }
                "--fov" => {
                    if let Some(value) = iter.next() {
                        match value.parse::<f32>() {
                            Ok(fov) => settings.fov = fov.clamp(30.0, 110.0),
                            Err(_) => unknown.push(value.clone()),
                        }
                    }
                }
                "--sensitivity" => {
                    if let Some(value) = iter.next() {
                        match value.parse::<f32>() {
                            Ok(value) => settings.sensitivity = value.clamp(0.0005, 0.02),
                            Err(_) => unknown.push(value.clone()),
                        }
                    }
                }
                "--help" | "-h" => settings.help = true,
                "--launch-api" => settings.launch_api = true,
                "--render-distance" => {
                    if let Some(value) = iter.next() {
                        match value.parse::<u32>() {
                            Ok(distance) => settings.render_distance = distance,
                            Err(_) => unknown.push(value.clone()),
                        }
                    }
                }
                other => unknown.push(other.to_owned()),
            }
        }
        Self { settings, unknown }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn facade_is_zero_sized_and_explicitly_unavailable() {
        assert_eq!(size_of::<Renderer>(), 0);
        assert_eq!(size_of::<Frame>(), 0);
        assert!(Renderer::unavailable().contains("PR 8"));
        assert!(!Renderer::device_lost());
        assert_eq!(Renderer::pack_shadow_casters(), 0);
        assert!(Renderer::take_dropped_sections().is_empty());
        Renderer::new().render(&Frame::empty());
    }

    #[test]
    fn api_names_and_choices_have_independent_values() {
        assert_eq!(LegacyGraphicsApi::ALL.len(), 5);
        assert_eq!(LegacyGraphicsApi::choices().len(), 5);
        assert_eq!(LegacyGraphicsApi::LegacyGl.name(), "legacy-gl");
        assert_eq!(LegacyGraphicsApi::LegacyGl.label(), "Legacy GL 2.1");
        assert_eq!(
            LegacyGraphicsApi::from_name("gl21"),
            Some(LegacyGraphicsApi::LegacyGl)
        );
        assert_eq!(
            LegacyGraphicsApi::from_name("legacygl"),
            Some(LegacyGraphicsApi::LegacyGl)
        );
        assert_eq!(LegacyGraphicsApi::from_name("metal"), None);
    }

    #[test]
    fn reference_settings_are_explicit() {
        let settings = LegacySettings::reference();
        assert_eq!(settings.api, LegacyGraphicsApi::LegacyGl);
        assert_eq!(settings.texture_budget_mib, 48);
        assert_eq!(settings.render_distance, 4);
        assert!(settings.legacy_gl);
        assert_eq!(settings.disabled_features.len(), 4);
        assert_eq!(
            settings.startup_note(),
            "graphics-api=legacy-gl budget=48MiB distance=4 frames=1 game=false seed=20958 world=default disabled=4"
        );
    }

    #[test]
    fn args_parse_explicit_flags_and_keep_unknown_values() {
        let args = [
            "--legacy-gl",
            "--texture-budget-mib",
            "64",
            "--render-distance",
            "5",
            "--frames",
            "0",
            "--game",
            "--seed",
            "99",
            "--world",
            "/tmp/world.json",
            "--fov",
            "80",
            "--sensitivity",
            "0.005",
            "--help",
            "--unknown",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect::<Vec<_>>();
        let parsed = LegacyArgs::parse(&args);
        assert_eq!(parsed.settings.api, LegacyGraphicsApi::LegacyGl);
        assert_eq!(parsed.settings.texture_budget_mib, 64);
        assert_eq!(parsed.settings.render_distance, 5);
        assert_eq!(parsed.settings.frames, 0);
        assert!(parsed.settings.game);
        assert_eq!(parsed.settings.seed, 99);
        assert_eq!(parsed.settings.world, "/tmp/world.json");
        assert_eq!(parsed.settings.fov, 80.0);
        assert_eq!(parsed.settings.sensitivity, 0.005);
        assert!(parsed.settings.help);
        assert_eq!(parsed.unknown, vec!["--unknown"]);
    }
}
