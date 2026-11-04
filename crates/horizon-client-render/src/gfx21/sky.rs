//! PR 7: sky, fog and day/night state for the GL2 `sky` program.
//!
//! The gradient is computed with integer arithmetic on the CPU, so the
//! day/night and fog colours are deterministic and do not depend on a shader
//! compiler. No GL calls are made by this module.

use super::state::{GL_BLEND, GL_DEPTH_TEST, GlCommand};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyState {
    pub top: [u8; 3],
    pub bottom: [u8; 3],
    pub fog: [u8; 3],
    pub sun: [f32; 3],
    pub time: u16,
}

fn lerp(a: u8, b: u8, t: u32) -> u8 {
    ((u32::from(a) * (255 - t) + u32::from(b) * t) / 255) as u8
}

impl SkyState {
    pub const MAX_TIME: u16 = 24_000;

    pub fn reference() -> Self {
        Self {
            top: [24, 36, 56],
            bottom: [72, 96, 120],
            fog: [90, 110, 130],
            sun: [0.4, 0.8, 0.2],
            time: 6_000,
        }
    }

    pub fn gradient(&self, y: u32, height: u32) -> [u8; 3] {
        let h = height.max(1);
        let t = y.min(h - 1) * 255 / h.max(2).saturating_sub(1);
        [
            lerp(self.top[0], self.bottom[0], t),
            lerp(self.top[1], self.bottom[1], t),
            lerp(self.top[2], self.bottom[2], t),
        ]
    }

    pub fn day_night(&self) -> f32 {
        let t = f32::from(self.time % Self::MAX_TIME) / f32::from(Self::MAX_TIME);
        if t < 0.5 { 1.0 } else { 0.25 }
    }

    pub fn gl_commands(&self) -> Vec<GlCommand> {
        vec![
            GlCommand::UseProgram(3),
            GlCommand::Disable(GL_DEPTH_TEST),
            GlCommand::Disable(GL_BLEND),
            GlCommand::DepthMask(false),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_gradient_endpoints_are_explicit() {
        let sky = SkyState::reference();
        assert_eq!(sky.gradient(0, 60), [24, 36, 56]);
        assert_eq!(sky.gradient(59, 60), [72, 96, 120]);
        assert_eq!(sky.fog, [90, 110, 130]);
        assert_eq!(sky.sun, [0.4, 0.8, 0.2]);
        assert_eq!(sky.day_night(), 1.0);
    }

    #[test]
    fn night_reduces_the_day_night_factor() {
        let mut sky = SkyState::reference();
        sky.time = 18_000;
        assert_eq!(sky.day_night(), 0.25);
    }

    #[test]
    fn sky_commands_disable_depth_and_blend() {
        assert_eq!(
            SkyState::reference().gl_commands(),
            vec![
                GlCommand::UseProgram(3),
                GlCommand::Disable(GL_DEPTH_TEST),
                GlCommand::Disable(GL_BLEND),
                GlCommand::DepthMask(false),
            ]
        );
    }
}
