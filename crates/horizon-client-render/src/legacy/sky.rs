//! Small untextured sky and void discs, matching the game's octagonal sky fan.
//! Sunrise, End sky and the title panorama are not implemented.
use crate::gfx::Frame;
use horizon_legacy_gl::{Batch, Draw, Fog, Pass, Vertex};

/// Sky rotation and intensity of the deterministic star geometry.
pub fn stars(
    frame: &Frame<'_>,
    view_projection: [f32; 16],
) -> Result<Option<([f32; 16], f32)>, &'static str> {
    if frame.title || frame.sky_blocked || !(frame.skybox || frame.end_sky) {
        return Ok(None);
    }
    let Some((rotation, brightness)) = frame.stars else {
        return Ok(None);
    };
    let transform = (glam::Mat4::from_cols_array(&view_projection) * rotation).to_cols_array();
    if !brightness.is_finite() || transform.iter().any(|value| !value.is_finite()) {
        return Err("invalid star rotation or brightness");
    }
    Ok(Some((transform, brightness.clamp(0., 1.))))
}

pub struct Sky {
    pub vertices: Vec<Vertex>,
    pub draws: Vec<Draw>,
    pub fog: Fog,
}

/// The original sky fan has a centre at y=16 and an eight-sided rim at
/// radius 512; the optional void disc has the same rim at y=-4. Fog values
/// interpolate per vertex, as in the game's sky shader.
pub fn build(frame: &Frame<'_>, fog_color: [f32; 4]) -> Result<Option<Sky>, &'static str> {
    if frame.title || frame.sky_blocked || !frame.skybox {
        return Ok(None);
    }
    let end = frame.fog.map_or(frame.distance.min(512.0), |f| f.sky_end);
    if !end.is_finite()
        || frame
            .sky
            .sky_color
            .iter()
            .chain(fog_color.iter())
            .any(|c| !c.is_finite())
    {
        return Err("invalid sky colour or fog distance");
    }
    let mut vertices = Vec::new();
    vertices
        .try_reserve_exact(if frame.dark_disc { 48 } else { 24 })
        .map_err(|_| "sky allocation failed")?;
    let mut fan = |y: f32, color: [f32; 4]| {
        let vertex = |position| Vertex {
            position,
            sprite_uv: [0.; 2],
            atlas_rect: [0.; 4],
            color,
        };
        for i in 0..8 {
            let angle = i as f32 * std::f32::consts::FRAC_PI_4;
            let next = (i + 1) as f32 * std::f32::consts::FRAC_PI_4;
            vertices.extend([
                vertex([0., y, 0.]),
                vertex([512. * angle.cos(), y, 512. * angle.sin()]),
                vertex([512. * next.cos(), y, 512. * next.sin()]),
            ]);
        }
    };
    let [r, g, b] = frame.sky.sky_color;
    fan(16., [r, g, b, 1.]);
    if frame.dark_disc {
        fan(-4., [0., 0., 0., 1.]);
    }
    let mut draws = vec![Draw {
        batch: Batch {
            page: 0,
            first: 0,
            count: 24,
        },
        pass: Pass::Sky,
    }];
    if frame.dark_disc {
        draws.push(Draw {
            batch: Batch {
                page: 0,
                first: 24,
                count: 24,
            },
            pass: Pass::Sky,
        });
    }
    Ok(Some(Sky {
        vertices,
        draws,
        fog: Fog {
            color: fog_color,
            ranges: [0., end, end, end],
            section_offset: [0.; 3],
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy::scene::tests::test_frame;

    #[test]
    fn disc_and_void_follow_frame_visibility_and_fog() {
        let hud = crate::hud::HudBatch::new([32, 32]);
        let mut frame = test_frame(&hud);
        assert!(build(&frame, [0.; 4]).unwrap().is_none());
        frame.skybox = true;
        frame.dark_disc = true;
        frame.sky.sky_color = [0.2, 0.4, 0.8];
        frame.distance = 64.;
        let sky = build(&frame, [0.3, 0.5, 0.7, 1.]).unwrap().unwrap();
        assert_eq!(sky.vertices.len(), 48);
        assert_eq!(sky.draws[0].batch.count, 24);
        assert_eq!(sky.draws[1].batch.first, 24);
        assert_eq!(sky.vertices[0].position, [0., 16., 0.]);
        assert_eq!(sky.vertices[24].position, [0., -4., 0.]);
        assert_eq!(sky.fog.ranges, [0., 64., 64., 64.]);
        frame.stars = Some((glam::Mat4::IDENTITY, 1.5));
        assert_eq!(
            stars(&frame, glam::Mat4::IDENTITY.to_cols_array())
                .unwrap()
                .unwrap()
                .1,
            1.
        );
        frame.sky_blocked = true;
        assert!(
            stars(&frame, glam::Mat4::IDENTITY.to_cols_array())
                .unwrap()
                .is_none()
        );
        assert!(build(&frame, [0.; 4]).unwrap().is_none());
        frame.sky_blocked = false;
        frame.title = true;
        assert!(build(&frame, [0.; 4]).unwrap().is_none());
    }
}
