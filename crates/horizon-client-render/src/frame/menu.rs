//! CPU menu request. Static menus cannot carry/drop a world geometry stream.
use super::Frame;
use crate::hud::HudBatch;
pub use horizon_render_api::render_contract::MenuBackground;

pub struct MenuFrame<'a> {
    pub hud: &'a HudBatch,
    pub background: MenuBackground,
    pub yaw: f32,
    pub pitch: f32,
}
impl<'a> MenuFrame<'a> {
    pub fn panorama(hud: &'a HudBatch, yaw: f32, pitch: f32) -> Self {
        Self {
            hud,
            background: MenuBackground::Panorama,
            yaw,
            pitch,
        }
    }
    pub fn static_color(hud: &'a HudBatch, rgba: [f32; 4]) -> Self {
        Self {
            hud,
            background: MenuBackground::Static(rgba),
            yaw: 0.,
            pitch: 10.,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.yaw.is_finite()
            || !self.pitch.is_finite()
            || !self.background.valid()
            || !self.hud.scale.is_finite()
            || self.hud.scale <= 0.
            || !self.hud.width.is_finite()
            || self.hud.width <= 0.
            || !self.hud.height.is_finite()
            || self.hud.height <= 0.
        {
            return Err("invalid menu view/background");
        }
        Ok(())
    }
    /// A resize invalidates the old layout. Validate the window-sized request
    /// before any atlas patch or clear, not against a scaled screenshot target.
    pub fn validate_target(&self, size: [u32; 2]) -> Result<(), &'static str> {
        self.validate()?;
        if size.contains(&0)
            || self.hud.width != (size[0] as f32 / self.hud.scale).ceil()
            || self.hud.height != (size[1] as f32 / self.hud.scale).ceil()
        {
            return Err("menu HUD dimensions changed; rebuild after resize");
        }
        Ok(())
    }
}

impl<'a> Frame<'a> {
    /// The canonical menu-only frame; shared by adapters, never a stripped world
    /// frame. Defaults match the original Game loading/title constructor.
    pub fn menu(hud: &'a HudBatch) -> Self {
        Self {
            weather: &[],
            weather_time: crate::weather::Time::new(0, 0.0),
            camera: glam::DVec3::new(0.5, 100.0, 0.5),
            yaw: 0.0,
            pitch: 10.0,
            fov_degrees: 85.0,
            eye: glam::DVec3::new(0.5, 100.0, 0.5),
            eye_rotation: [0.0, 10.0],
            first_person: true,
            bob: glam::Mat4::IDENTITY,
            spin: glam::Mat4::IDENTITY,
            sky: crate::environment::Sky::default(),
            shadow_scene_revision: (0, 0),
            shadow_cache: false,
            distance: 12.0,
            detail_distance: 12.0,
            depth_far: 1000.0,
            underwater: false,
            water_plane: None,
            cave_culling: false,
            title: true,
            sky_blocked: false,
            skybox: false,
            end_sky: false,
            end_sky_sprite: None,
            environment_fog: None,
            water_fog: [0.0; 3],
            fog: None,
            dark_disc: false,
            outline: Vec::new(),
            outline_after_terrain: false,
            outline_high_contrast: false,
            debug_lines: &[],
            debug_lines_on_top: &[],
            debug_crosshair: &[],
            hud,
            glint_alpha: 0.75,
            glint_ticks: 0.0,
            nether_lighting: false,
            entities: &[],
            static_block_entities: &[],
            visible_static_block_entity_sections: &[],
            static_block_entity_stats: Default::default(),
            entity_motion: &[],
            entity_spans: &[],
            skin: None,
            glowing: &[],
            dynamic_lights: &[],
            colored_lights: &[],
            emissive_layers: &[],
            clouds: None,
            lightning: Default::default(),
            additive: &[],
            see_through: &[],
            shadow_only: &[],
            celestial: &[],
            stars: None,
            hand: &[],
            hand_fov_degrees: 70.0,
            screen: &[],
            pack: Default::default(),
            border: None,
            game_time: 0.0,
            shader_game_time: 0.0,
            end_portal_sprites: [[0.0; 4]; 2],
            crumbling_sprite: [0.0; 4],
            ticks: 0,
            darkness: (0.0, 0.0),
            night_vision: 0.0,
            block_flicker: 0.0,
            boss_darken: 0.0,
            chunk_fade: 0.0,
            post_effects: crate::post_chain::Request::none(),
            menu_blur: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_requests_do_not_inherit_world_payloads() {
        let hud = HudBatch::new([64; 2]);
        let frame = Frame::menu(&hud);
        assert!(frame.title);
        assert!(!frame.skybox);
        assert!(frame.entities.is_empty() && frame.weather.is_empty() && frame.hand.is_empty());
        assert!(frame.skin.is_none() && frame.clouds.is_none());
        assert_eq!((frame.yaw, frame.pitch, frame.fov_degrees), (0., 10., 85.));
        assert!(MenuFrame::panorama(&hud, 25., 10.).validate().is_ok());
        assert!(MenuFrame::panorama(&hud, f32::NAN, 10.).validate().is_err());
        for color in [
            [f32::NAN, 0., 0., 1.],
            [-0.1, 0., 0., 1.],
            [0., 0., 0., 1.1],
        ] {
            assert!(MenuFrame::static_color(&hud, color).validate().is_err());
        }
    }
    #[test]
    fn resize_and_nonfinite_layout_require_a_fresh_request() {
        let mut hud = HudBatch::new([64; 2]);
        assert!(
            MenuFrame::static_color(&hud, [0.; 4])
                .validate_target([64; 2])
                .is_ok()
        );
        assert!(
            MenuFrame::static_color(&hud, [0.; 4])
                .validate_target([32; 2])
                .is_err()
        );
        assert!(
            MenuFrame::static_color(&hud, [0.; 4])
                .validate_target([0; 2])
                .is_err()
        );
        hud.scale = f32::NAN;
        assert!(MenuFrame::panorama(&hud, 0., 10.).validate().is_err());
        hud.scale = 1.;
        hud.width = f32::INFINITY;
        assert!(MenuFrame::static_color(&hud, [0.; 4]).validate().is_err());
    }
}
