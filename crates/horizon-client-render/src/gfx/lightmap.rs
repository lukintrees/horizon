//! Shared CPU lightmap generation for the wgpu and experimental GL paths.
use crate::environment::Sky;
use glam::Vec3;

pub(crate) fn pixels(
    sky: &Sky,
    darkness: f32,
    gamma: f32,
    night_vision: f32,
    flicker: f32,
    boss_darken: f32,
    fullbright: bool,
) -> [u8; 16 * 16 * 4] {
    let brightness = |level: f32| level / (4.0 - 3.0 * level);
    let mut pixels = [0u8; 16 * 16 * 4];
    for s in 0..16 {
        for b in 0..16 {
            let block_level = b as f32 / 15.0;
            let sky_level = s as f32 / 15.0;
            let block_brightness = brightness(block_level) * (1.4 + flicker);
            let sky_brightness = brightness(sky_level) * sky.sky_light_factor;
            let mut color =
                Vec3::from(sky.ambient_light_color).max(Vec3::splat(0.6 * night_vision));
            color += Vec3::from(sky.sky_light_color) * sky_brightness;
            let parabolic = (2.0 * block_level - 1.0).powi(2);
            let block_color = Vec3::from(sky.block_light_tint).lerp(Vec3::ONE, 0.9 * parabolic);
            color += block_color * block_brightness;
            color = color.lerp(color * Vec3::new(0.7, 0.6, 0.6), boss_darken);
            color -= Vec3::splat(darkness);
            color = color.clamp(Vec3::ZERO, Vec3::ONE);
            let max = color.max_element().max(1e-6);
            let inverted = 1.0 - max;
            let scaled = 1.0 - inverted.powi(4);
            let not_gamma = color * (scaled / max);
            color = color.lerp(not_gamma, gamma);
            let i = (s * 16 + b) * 4;
            pixels[i] = (color.x * 255.0) as u8;
            pixels[i + 1] = (color.y * 255.0) as u8;
            pixels[i + 2] = (color.z * 255.0) as u8;
            pixels[i + 3] = 255;
        }
    }
    if fullbright {
        pixels.fill(255);
    }
    pixels
}
