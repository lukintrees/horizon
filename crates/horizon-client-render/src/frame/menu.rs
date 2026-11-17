//! CPU menu request. Static menus cannot carry/drop a world geometry stream.
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_requests_do_not_inherit_world_payloads() {
        let hud = HudBatch::new([64; 2]);
        let frame = crate::gfx::menu_frame(&hud);
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
