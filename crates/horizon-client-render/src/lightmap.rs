//! Shared CPU lightmap calculation. Kept byte-identical for wgpu and GL2.
use crate::environment::Sky;
use glam::Vec3;

#[derive(Clone, Copy, Debug, Default)]
pub struct Parameters {
    pub darkness: f32,
    pub gamma: f32,
    pub night_vision: f32,
    pub flicker: f32,
    pub boss_darken: f32,
    pub fullbright: bool,
}

/// Rows are sky light, columns block light, RGBA8, texel centers at (n+.5)/16.
/// Caller supplies the effective gamma after the darkness brightness penalty.
pub fn pixels(sky: &Sky, parameters: Parameters) -> [u8; 16 * 16 * 4] {
    let brightness = |level: f32| level / (4.0 - 3.0 * level);
    let mut pixels = [0u8; 16 * 16 * 4];
    for s in 0..16 {
        for b in 0..16 {
            let block_level = b as f32 / 15.0;
            let sky_level = s as f32 / 15.0;
            let block_brightness = brightness(block_level) * (1.4 + parameters.flicker);
            let sky_brightness = brightness(sky_level) * sky.sky_light_factor;
            // NightVisionColor defaults to 0x999999.
            let mut color =
                Vec3::from(sky.ambient_light_color).max(Vec3::splat(0.6 * parameters.night_vision));
            color += Vec3::from(sky.sky_light_color) * sky_brightness;
            let parabolic = (2.0 * block_level - 1.0).powi(2);
            let block_color = Vec3::from(sky.block_light_tint).lerp(Vec3::ONE, 0.9 * parabolic);
            color += block_color * block_brightness;
            // lightmap.fsh: a boss bar mixes the light toward 70/60/60%.
            color = color.lerp(color * Vec3::new(0.7, 0.6, 0.6), parameters.boss_darken);
            color -= Vec3::splat(parameters.darkness);
            color = color.clamp(Vec3::ZERO, Vec3::ONE);
            let max = color.max_element().max(1e-6);
            let inverted = 1.0 - max;
            let scaled = 1.0 - inverted.powi(4);
            let not_gamma = color * (scaled / max);
            color = color.lerp(not_gamma, parameters.gamma);
            let i = (s * 16 + b) * 4;
            pixels[i] = (color.x * 255.0) as u8;
            pixels[i + 1] = (color.y * 255.0) as u8;
            pixels[i + 2] = (color.z * 255.0) as u8;
            pixels[i + 3] = 255;
        }
    }
    if parameters.fullbright {
        pixels.fill(255);
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn texel_axes_darkness_boss_and_fullbright_keep_exact_rgba8_contract() {
        let sky = Sky {
            ambient_light_color: [0.25, 0.5, 0.75],
            sky_light_color: [0.5, 0.25, 0.],
            block_light_tint: [1., 0., 0.],
            sky_light_factor: 1.,
            ..Sky::default()
        };
        let normal = pixels(&sky, Parameters::default());
        assert_eq!(&normal[..4], &[63, 127, 191, 255]);
        assert_eq!(&normal[15 * 16 * 4..15 * 16 * 4 + 4], &[191, 191, 191, 255]);
        assert_eq!(&normal[15 * 4..15 * 4 + 4], &[255; 4]);
        let boss = pixels(
            &sky,
            Parameters {
                boss_darken: 1.,
                ..Parameters::default()
            },
        );
        assert_eq!(&boss[..4], &[44, 76, 114, 255]);
        let dark = pixels(
            &sky,
            Parameters {
                darkness: 1.,
                ..Parameters::default()
            },
        );
        assert_eq!(&dark[..4], &[0, 0, 0, 255]);
        let vision = pixels(
            &Sky::default(),
            Parameters {
                night_vision: 1.,
                ..Parameters::default()
            },
        );
        assert_eq!(&vision[..4], &[153, 153, 153, 255]);
        assert_eq!(
            pixels(
                &sky,
                Parameters {
                    darkness: 1.,
                    fullbright: true,
                    ..Parameters::default()
                }
            ),
            [255; 1024]
        );
    }
}
