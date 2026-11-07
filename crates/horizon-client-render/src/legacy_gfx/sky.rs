//! Bounded classic sky geometry. Eight fan sectors reproduce the analytic WGSL
//! disc exactly: all rim vertices have equal spherical/cylindrical distances,
//! so perspective interpolation gives disc_distance's radial interpolation.
use super::{Limits, screen};
use crate::{gfx, mesher::Vertex as Source};
use glam::{Mat4, Vec3};
use horizon_render_api::{
    Clear, Draw, Fog, Frame, GameFog, Pass, RenderBackend, TextureId, Vertex,
    gl_reversed_projection,
};
use std::{borrow::Cow, sync::OnceLock};
struct Run {
    vertices: Vec<Vertex>,
    matrix: [f32; 16],
    fog: Option<Fog>,
    additive: bool,
    flatten: bool,
}
pub(super) struct Prepared<'a> {
    before: Vec<Run>,
    after: Vec<Run>,
    pub celestial: Cow<'a, [Source]>,
    pub matrix: [f32; 16],
    pub basic_vertices: usize,
}
fn vertex(pos: [f32; 3], color: [f32; 4]) -> Vertex {
    Vertex {
        pos,
        color,
        uv: [0.; 2],
        light_uv: [0.; 2],
    }
}
fn disc(y: f32, color: [f32; 3]) -> Result<Vec<Vertex>, String> {
    let mut vertices = Vec::new();
    vertices
        .try_reserve_exact(24)
        .map_err(|_| "sky allocation failed")?;
    let color = [color[0], color[1], color[2], 1.];
    for i in 0..8 {
        let rim = |i: usize| {
            let (s, c) = (i as f32 * std::f32::consts::FRAC_PI_4).sin_cos();
            [c * 512., y, s * 512.]
        };
        vertices.extend([
            vertex([0., y, 0.], color),
            vertex(rim(i), color),
            vertex(rim((i + 1) % 8), color),
        ]);
    }
    Ok(vertices)
}
impl<'a> Prepared<'a> {
    pub fn new(f: &'a gfx::Frame<'_>, size: [u32; 2], limits: Limits) -> Result<Self, String> {
        let view = gfx::View::screen(
            f.yaw,
            f.pitch,
            f.fov_degrees,
            size[0] as f32 / size[1] as f32,
            f.depth_far,
        );
        let matrix = gl_reversed_projection(view.view_proj(f.bob * f.spin).to_cols_array());
        let mut result = Self {
            before: Vec::new(),
            after: Vec::new(),
            celestial: Cow::Borrowed(&[]),
            matrix,
            basic_vertices: 0,
        };
        if f.sky_blocked || !(f.skybox || f.end_sky) {
            return Ok(result);
        }
        let (fog_color, _) = crate::fog::world_parameters(f);
        let end = f.fog.map_or(f.distance.min(512.), |fog| fog.sky_end);
        if !end.is_finite()
            || end < 0.
            || !fog_color.into_iter().all(unit)
            || (f.skybox
                && (!f
                    .sky
                    .sky_color
                    .into_iter()
                    .chain(f.sky.sunrise_color)
                    .all(unit)
                    || !f.sky.sun_angle.is_finite()))
        {
            return Err("invalid classic sky color/fog/sun".into());
        }
        let sky_matrix = gl_reversed_projection(view.view_proj(Mat4::IDENTITY).to_cols_array());
        let fog = Some(Fog {
            color: [fog_color[0], fog_color[1], fog_color[2]],
            start: 0.,
            end,
            game: Some(GameFog {
                start: end,
                end,
                alpha: fog_color[3],
            }),
        });
        let end_vertices = if f.end_sky { 6144 } else { 0 };
        if !f.celestial.len().is_multiple_of(4) || f.celestial.len() > 8192 {
            return Err("celestial quad budget/count exceeded".into());
        }
        if f.celestial
            .iter()
            .any(|v| !matches!(v.layer & 0xffff, 0 | 2 | 3))
        {
            return Err("unsupported celestial material".into());
        }
        let stars = if let Some((rotation, b)) = f.stars {
            if !rotation.is_finite() || !b.is_finite() {
                return Err("invalid star rotation/brightness".into());
            }
            let matrix = (Mat4::from_cols_array(&matrix) * rotation).to_cols_array();
            if matrix.iter().any(|v| !v.is_finite()) {
                return Err("star projection overflow".into());
            }
            let b = b.clamp(0., 1.);
            if b > 0. {
                static STARS: OnceLock<Vec<[f32; 3]>> = OnceLock::new();
                Some((STARS.get_or_init(gfx::stars::star_triangles), matrix, b))
            } else {
                None
            }
        } else {
            None
        };
        let count = f
            .celestial
            .len()
            .checked_add(end_vertices)
            .and_then(|n| {
                n.checked_add(if f.skybox {
                    24 + if f.sky.sunrise_color[3] > 0.001 {
                        48
                    } else {
                        0
                    }
                } else {
                    0
                })
            })
            .and_then(|n| n.checked_add(if f.dark_disc { 24 } else { 0 }))
            .and_then(|n| n.checked_add(stars.as_ref().map_or(0, |(v, _, _)| v.len())))
            .filter(|&n| n <= limits.sky_vertices)
            .ok_or("sky vertex budget exceeded")?;
        if f.skybox {
            result.before.push(Run {
                vertices: disc(16., f.sky.sky_color)?,
                matrix: sky_matrix,
                fog,
                additive: false,
                flatten: true,
            });
            if f.sky.sunrise_color[3] > 0.001 {
                let angle = f.sky.sun_angle.to_radians();
                let sun = Vec3::new(0., angle.cos(), -angle.sin());
                let mut vertices = Vec::new();
                vertices
                    .try_reserve_exact(48)
                    .map_err(|_| "sky allocation failed")?;
                vertices.extend(
                    gfx::sunrise_fan::mesh(f.sky.sunrise_color[3], sun).map(|p| {
                        vertex(
                            [p[0], p[1], p[2]],
                            [
                                f.sky.sunrise_color[0],
                                f.sky.sunrise_color[1],
                                f.sky.sunrise_color[2],
                                p[3] * f.sky.sunrise_color[3],
                            ],
                        )
                    }),
                );
                result.before.push(Run {
                    vertices,
                    matrix: sky_matrix,
                    fog: None,
                    additive: false,
                    flatten: false,
                });
            }
        }
        result.celestial = if f.end_sky {
            let uv = f.end_sky_sprite.ok_or("missing End sky sprite")?;
            if !uv.into_iter().all(unit) || uv[0] >= uv[2] || uv[1] >= uv[3] {
                return Err("invalid End sky sprite".into());
            }
            let mut vertices = Vec::new();
            vertices
                .try_reserve_exact(end_vertices + f.celestial.len())
                .map_err(|_| "sky allocation failed")?;
            vertices.extend(gfx::end_sky::vertices(uv));
            vertices.extend_from_slice(f.celestial);
            Cow::Owned(vertices)
        } else {
            Cow::Borrowed(f.celestial)
        };
        if let Some((stars, matrix, b)) = stars {
            let mut vertices = Vec::new();
            vertices
                .try_reserve_exact(stars.len())
                .map_err(|_| "sky allocation failed")?;
            vertices.extend(stars.iter().map(|&p| vertex(p, [b; 4]))); // SDR stars add b*b
            result.after.push(Run {
                vertices,
                matrix,
                fog: None,
                additive: true,
                flatten: false,
            });
        }
        if f.dark_disc {
            result.after.push(Run {
                vertices: disc(-4., [0.; 3])?,
                matrix: sky_matrix,
                fog,
                additive: false,
                flatten: true,
            });
        }
        result.basic_vertices = count - result.celestial.len();
        Ok(result)
    }
    pub fn is_empty(&self) -> bool {
        self.before.is_empty() && self.after.is_empty() && self.celestial.is_empty()
    }
    pub fn largest_mesh(&self) -> usize {
        self.before
            .iter()
            .chain(&self.after)
            .map(|r| r.vertices.len() * (std::mem::size_of::<Vertex>() + 2))
            .max()
            .unwrap_or(0)
    }
}
fn unit(f: f32) -> bool {
    f.is_finite() && (0. ..=1.).contains(&f)
}
impl screen::Resources {
    pub fn render_sky<B: RenderBackend<Error = String>>(
        &mut self,
        backend: &mut B,
        prepared: &Prepared<'_>,
        before: bool,
        white: TextureId,
        camera: [f64; 3],
    ) -> Result<(), String> {
        self.collect(backend)?;
        let indices: [u16; 9000] = std::array::from_fn(|i| i as u16);
        for run in if before {
            &prepared.before
        } else {
            &prepared.after
        } {
            // Largest mesh and all CPU scratch were reserved before clear.
            let mesh = backend.create_mesh(&run.vertices, &indices[..run.vertices.len()])?;
            self.meshes.push(mesh);
            backend.render(&Frame {
                view_projection: run.matrix,
                camera,
                clear: Clear::Load,
                fog: run.fog,
                draws: &[Draw {
                    mesh,
                    texture: white,
                    lightmap: white,
                    origin: camera,
                    pass: Pass::Sky {
                        additive: run.additive,
                        flatten_depth: run.flatten,
                    },
                    scissor: None,
                }],
            })?;
            self.collect(backend)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hud::HudBatch, legacy_gfx::frame};
    #[test]
    fn sky_orders_shared_end_celestial_stars_and_dark_disc_with_distinct_fog_and_pose() {
        let hud = HudBatch::new([64; 2]);
        let mut f = frame::tests::source(&hud);
        f.skybox = true;
        f.end_sky = true;
        f.end_sky_sprite = Some([0., 0., 1., 1.]);
        f.dark_disc = true;
        f.sky.sky_color = [0.2, 0.4, 0.8];
        f.sky.sunrise_color = [1., 0.5, 0., 0.75];
        f.sky.sun_angle = 30.;
        f.stars = Some((Mat4::IDENTITY, 2.));
        f.bob = Mat4::from_rotation_z(0.2);
        let extra = [Source {
            pos: [1., 2., 3.],
            uv: [0.5; 2],
            normal: [0; 4],
            color: u32::MAX,
            layer: 2,
            light: 0,
            glint_uv: [0.; 2],
        }; 4];
        f.celestial = &extra;
        let p = Prepared::new(&f, [64; 2], Limits::default()).unwrap();
        assert_eq!(p.before.len(), 2);
        assert_eq!(p.after.len(), 2);
        assert_eq!(p.celestial.len(), 6148);
        assert_eq!(p.celestial[6144].pos, [1., 2., 3.]);
        assert_eq!(p.celestial[0].color, 0xff282828);
        assert!(p.before[0].flatten);
        assert!(!p.before[1].flatten);
        assert!(p.before[0].fog.is_some());
        assert!(p.before[1].fog.is_none());
        assert_ne!(p.before[0].matrix, p.matrix); // sky fan ignores world bob/spin
        assert!(p.after[0].additive);
        assert_eq!(p.after[0].vertices[0].color, [1.; 4]);
        assert!(!p.after[1].additive);
        assert_eq!(p.after[1].vertices[0].pos, [0., -4., 0.]);
        assert!(p.basic_vertices > 48);
        assert!(p.largest_mesh() > 2208);
        f.sky_blocked = true;
        f.celestial = &extra[..3];
        f.sky.sky_color = [f32::NAN; 3];
        assert!(
            Prepared::new(
                &f,
                [64; 2],
                Limits {
                    sky_vertices: 0,
                    ..Limits::default()
                }
            )
            .unwrap()
            .is_empty()
        );
    }
    #[test]
    fn sky_rejects_requested_invalid_metadata_and_counts_without_truncation() {
        let hud = HudBatch::new([64; 2]);
        for case in 0..8 {
            let mut f = frame::tests::source(&hud);
            f.skybox = true;
            match case {
                0 => f.sky.sky_color[0] = f32::NAN,
                1 => f.sky.sunrise_color[3] = 2.,
                2 => f.sky.sun_angle = f32::INFINITY,
                3 => f.stars = Some((Mat4::IDENTITY, f32::NAN)),
                4 => {
                    f.end_sky = true;
                }
                5 => {
                    f.end_sky = true;
                    f.end_sky_sprite = Some([1., 0., 0., 1.]);
                }
                6 => f.stars = Some((Mat4::from_cols_array(&[f32::INFINITY; 16]), 1.)),
                _ => {
                    f.fog = Some(crate::fog::FogData {
                        color: [0.; 4],
                        environmental_start: 0.,
                        environmental_end: 1.,
                        render_distance_start: 0.,
                        render_distance_end: 1.,
                        sky_end: -1.,
                        cloud_end: 1.,
                    })
                }
            }
            assert!(
                Prepared::new(&f, [64; 2], Limits::default()).is_err(),
                "case {case}"
            );
        }
        let mut f = frame::tests::source(&hud);
        f.skybox = true;
        f.sky.sunrise_color = [1.; 4];
        assert!(
            Prepared::new(
                &f,
                [64; 2],
                Limits {
                    sky_vertices: 71,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(
            Prepared::new(
                &f,
                [64; 2],
                Limits {
                    sky_vertices: 72,
                    ..Limits::default()
                }
            )
            .is_ok()
        );
        let extra = [Source {
            pos: [0.; 3],
            uv: [0.5; 2],
            normal: [0; 4],
            color: 0,
            layer: 37,
            light: 0,
            glint_uv: [0.; 2],
        }; 4];
        f.celestial = &extra;
        assert!(Prepared::new(&f, [64; 2], Limits::default()).is_err());
        f.celestial = &extra[..3];
        assert!(Prepared::new(&f, [64; 2], Limits::default()).is_err());
        let extra = vec![
            Source {
                layer: 2,
                ..extra[0]
            };
            8196
        ];
        f.celestial = &extra;
        assert!(Prepared::new(&f, [64; 2], Limits::default()).is_err());
    }
}
