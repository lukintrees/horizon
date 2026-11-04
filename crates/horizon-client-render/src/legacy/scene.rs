//! Adapts the game's real frame camera, fog and lightmap to the partial GL
//! world draw planner. This does not yet install the backend in the game loop.
use super::world::{PlanError, SectionDraw, SectionKey, World};
use crate::gfx::{Frame, View};
use glam::{Mat4, Vec4};
use horizon_legacy_gl::Fog;

/// Generate exactly the same RGBA lightmap as the existing renderer. Gamma and
/// fullbright are renderer settings rather than fields in `Frame`.
pub fn lightmap(frame: &Frame<'_>, gamma: f32, fullbright: bool) -> [u8; 16 * 16 * 4] {
    crate::gfx::lightmap::pixels(
        &frame.sky,
        frame.darkness.1,
        (gamma - frame.darkness.0).max(0.0),
        frame.night_vision,
        frame.block_flicker,
        frame.boss_darken,
        fullbright,
    )
}

/// Convert reversed-Z wgpu clip depth (near=1, far=0) to conventional OpenGL
/// clip depth (near=-1, far=1). This operates on homogeneous clip coordinates,
/// not on a perspective-divided depth value.
pub fn gl_view_projection(view: View, pose: Mat4) -> Mat4 {
    let depth = Mat4::from_cols(
        Vec4::X,
        Vec4::Y,
        Vec4::new(0.0, 0.0, -2.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0, 1.0),
    );
    depth * view.view_proj(pose)
}

fn frame_fog(frame: &Frame<'_>) -> Fog {
    let (color, ranges) = if let Some(fog) = frame.fog {
        (
            fog.color,
            [
                fog.environmental_start,
                fog.environmental_end,
                fog.render_distance_start,
                fog.render_distance_end,
            ],
        )
    } else if frame.underwater {
        (
            [
                frame.water_fog[0],
                frame.water_fog[1],
                frame.water_fog[2],
                1.0,
            ],
            [-8.0, 96.0, -8.0, 96.0],
        )
    } else {
        let (start, end) = frame.environment_fog.unwrap_or((f32::MAX, f32::MAX));
        let d = frame.distance;
        (
            [
                frame.sky.fog_color[0],
                frame.sky.fog_color[1],
                frame.sky.fog_color[2],
                1.0,
            ],
            [start, end, (d - d.min(64.0) * 0.25).max(0.0), d],
        )
    };
    Fog {
        color,
        ranges,
        section_offset: [0.0; 3],
    }
}

/// Plan the world's solid and translucent passes from an actual game frame.
/// This is flat-screen only, uses externally supplied visibility, and leaves
/// entities, hand, sky, far terrain and HUD to subsequent passes.
pub fn plan<'a>(
    world: &'a mut World,
    frame: &Frame<'_>,
    viewport: [u32; 2],
    min_y: i32,
    visible: &[SectionKey],
) -> Result<Vec<SectionDraw<'a>>, PlanError> {
    if viewport.contains(&0)
        || !frame.fov_degrees.is_finite()
        || !(0.0..180.0).contains(&frame.fov_degrees)
        || frame.fov_degrees == 0.0
        || !frame.depth_far.is_finite()
        || frame.depth_far <= 0.05
        || !frame.yaw.is_finite()
        || !frame.pitch.is_finite()
    {
        return Err(PlanError::InvalidTransform);
    }
    let aspect = viewport[0] as f32 / viewport[1] as f32;
    let view = View::screen(
        frame.yaw,
        frame.pitch,
        frame.fov_degrees,
        aspect,
        frame.depth_far,
    );
    let matrix = gl_view_projection(view, frame.bob * frame.spin).to_cols_array();
    world.plan(
        visible,
        frame.camera.to_array(),
        min_y,
        matrix,
        Some(frame_fog(frame)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reversed_wgpu_depth_becomes_normal_gl_depth() {
        let view = View::screen(0.0, 0.0, 70.0, 1.0, 512.0);
        let wgpu = view.projection;
        let gl = gl_view_projection(
            View {
                view: Mat4::IDENTITY,
                ..view
            },
            Mat4::IDENTITY,
        );
        for z in [-0.05, -1.0, -512.0] {
            let input = Vec4::new(0.0, 0.0, z, 1.0);
            let a = wgpu * input;
            let b = gl * input;
            assert!((b.z / b.w - (1.0 - 2.0 * a.z / a.w)).abs() < 0.00001);
        }
        let near = gl * Vec4::new(0.0, 0.0, -0.05, 1.0);
        let far = gl * Vec4::new(0.0, 0.0, -512.0, 1.0);
        assert!((near.z / near.w + 1.0).abs() < 0.00001);
        assert!((far.z / far.w - 1.0).abs() < 0.00001);
    }
}
