//! Unsplit real weather quads. Animate on the CPU, repeat only in the fragment
//! shader on independently extracted sprites; no UV tiling or atlas-wide repeat.
use super::screen::{self, Sprite};
use crate::{
    paged_hud::Backing,
    weather::{Time, Vertex as Source},
};
use horizon_render_api::{
    Clear, Draw, Frame, Image, Pass, RenderBackend, Vertex,
    entity::{self, Kind, LitVertex, Material, Overlay},
    paged_gui::WorldView,
};
use std::collections::HashMap;

struct Run {
    sprite: usize,
    vertices: [LitVertex; 4],
}
pub(super) struct Prepared {
    sprites: Vec<Sprite>,
    runs: Vec<Run>,
}
impl Prepared {
    pub fn new(
        vertices: &[Source],
        time: Time,
        backing: &Backing,
        lightmap: &[u8],
        quad_limit: usize,
        byte_limit: usize,
        max_texture: u32,
    ) -> Result<Self, String> {
        if !vertices.len().is_multiple_of(4) || vertices.len() / 4 > quad_limit {
            return Err("weather quad budget/count exceeded".into());
        }
        let lightmap: &[u8; 1024] = lightmap
            .try_into()
            .map_err(|_| "invalid weather lightmap")?;
        if !vertices.is_empty() {
            let [ticks, short, partial, padding] = time.values;
            if !ticks.is_finite()
                || !(0. ..=131071.).contains(&ticks)
                || ticks.fract() != 0.
                || short != ticks % 512.
                || !partial.is_finite()
                || !(0. ..=1.).contains(&partial)
                || padding != 0.
            {
                return Err("invalid masked weather clock".into());
            }
        }
        let mut runs = Vec::new();
        runs.try_reserve_exact(vertices.len() / 4)
            .map_err(|_| "weather allocation failed")?;
        let mut sprites = Vec::new();
        let mut indices = HashMap::new();
        indices
            .try_reserve(vertices.len() / 4)
            .map_err(|_| "weather allocation failed")?;
        sprites
            .try_reserve_exact(vertices.len() / 4)
            .map_err(|_| "weather allocation failed")?;
        let mut bytes = if vertices.is_empty() {
            0
        } else {
            backing.layout().page_bytes()
        };
        for quad in vertices.as_chunks::<4>().0.iter() {
            let first = &quad[0];
            if quad.iter().any(|v| {
                v.rect != first.rect
                    || v.animation != first.animation
                    || v.bases != first.bases
                    || v.snow != first.snow
                    || v.snow > 1
                    || v.light & !0xffff != 0
                    || v.pos
                        .iter()
                        .chain(&v.uv)
                        .chain(&v.animation)
                        .any(|f| !f.is_finite())
            }) {
                return Err("invalid weather quad/attributes".into());
            }
            // Weather rect is xy + EXTENT, whereas screen rect is xy + END.
            let [x, y, w, h] = first.rect;
            if w <= 0. || h <= 0. {
                return Err("empty weather sprite".into());
            }
            let rect = screen::pixel_rect([x, y, x + w, y + h], backing.layout().size())?;
            if rect[2] > max_texture || rect[3] > max_texture {
                return Err("weather texture limit exceeded".into());
            }
            let phase = phase(first, time)?;
            let mut output = [LitVertex {
                base: Vertex {
                    pos: [0.; 3],
                    uv: [0.; 2],
                    color: [0.; 4],
                    light_uv: [0.; 2],
                },
                back_color: [0.; 4],
                light_color: [0.; 4],
                glint_uv: [0.; 2],
            }; 4];
            for (out, v) in output.iter_mut().zip(quad) {
                let uv = [v.uv[0] + phase[0], v.uv[1] + phase[1]];
                if uv.iter().any(|f| !f.is_finite()) {
                    return Err("weather UV arithmetic overflow".into());
                }
                let color = v.color.to_le_bytes().map(|c| f32::from(c) / 255.);
                *out = LitVertex {
                    base: Vertex {
                        pos: v.pos,
                        uv,
                        color,
                        light_uv: [0.; 2],
                    },
                    back_color: color,
                    light_color: entity::light_color(lightmap, v.light),
                    glint_uv: [0.; 2],
                };
            }
            let sprite = if let Some(&index) = indices.get(&rect) {
                index
            } else {
                let len = (rect[2] as usize)
                    .checked_mul(rect[3] as usize)
                    .and_then(|n| n.checked_mul(4))
                    .ok_or("weather size overflow")?;
                bytes = bytes
                    .checked_add(len)
                    .filter(|&n| n <= byte_limit)
                    .ok_or("weather decoded-byte budget exceeded")?;
                let index = sprites.len();
                sprites.push(Sprite {
                    size: [rect[2], rect[3]],
                    pixels: backing.extract(rect, len)?,
                });
                indices.insert(rect, index);
                index
            };
            runs.push(Run {
                sprite,
                vertices: output,
            });
        }
        Ok(Self { sprites, runs })
    }
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
    pub fn largest_texture(&self) -> usize {
        self.sprites
            .iter()
            .map(|s| s.pixels.len())
            .max()
            .unwrap_or(0)
    }
}
fn phase(v: &Source, time: Time) -> Result<[f32; 2], String> {
    let [ticks, short, partial, _] = time.values;
    let a = v.animation;
    let result = if v.snow == 0 {
        [0., (-((ticks + a[1]) + partial) / 32. * a[0]) % 32.]
    } else {
        let base = |lo: u32, hi: u32| f64::from_bits(u64::from(lo) | (u64::from(hi) << 32));
        let u = base(v.bases[0], v.bases[1]);
        let w = base(v.bases[2], v.bases[3]);
        if !u.is_finite() || !w.is_finite() {
            return Err("invalid snow bases".into());
        }
        let t = ticks + partial;
        [
            (u + f64::from((t * 0.01) * a[2])) as f32,
            -(short + partial) / 512. + (w + f64::from((t * a[3]) * 0.001)) as f32,
        ]
    };
    if result.iter().any(|v| !v.is_finite()) {
        return Err("weather phase overflow".into());
    }
    Ok(result)
}
impl screen::Resources {
    /// One sprite and one 348-byte lit mesh at a time; depth/fog remain world-space.
    pub fn render_weather<B: RenderBackend<Error = String>>(
        &mut self,
        backend: &mut B,
        prepared: &Prepared,
        view: &WorldView,
    ) -> Result<(), String> {
        self.collect(backend)?;
        for run in &prepared.runs {
            let sprite = &prepared.sprites[run.sprite];
            let texture = backend.create_texture(Image {
                width: sprite.size[0],
                height: sprite.size[1],
                rgba: &sprite.pixels,
            })?;
            self.textures.push(texture);
            let mesh = backend.create_entity_mesh(&run.vertices, &[0, 1, 2, 0, 2, 3])?;
            self.meshes.push(mesh);
            backend.render(&Frame {
                view_projection: view.view_projection,
                camera: view.camera,
                clear: Clear::Load,
                fog: view.fog,
                draws: &[Draw {
                    mesh,
                    texture,
                    lightmap: view.lightmap,
                    origin: view.camera,
                    pass: Pass::Entity(Material {
                        overlay: Overlay::None,
                        cull: false,
                        kind: Kind::Weather,
                        glint: None,
                    }),
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
    use crate::{font::Patch, weather::Parameters};
    use horizon_render_api::paged_gui::Layout;
    fn vertex() -> Source {
        Source {
            pos: [0., 0., 2.],
            uv: [-0.25, 2.25],
            rect: [1. / 7., 1. / 5., 5. / 7., 3. / 5.],
            animation: [3.5, 17., 0., 0.],
            bases: [0; 4],
            color: 0x80402010,
            light: 0x0080,
            snow: 0,
        }
    }
    #[test]
    fn phase_retains_masked_rain_and_double_plus_float_snow_reference_rounding() {
        let v = vertex();
        assert_eq!(phase(&v, Time::new(31, 0.5)).unwrap(), [0., -5.3046875]);
        for (x, z) in [(0, 0), (17, -33), (-30_000_000, 30_000_000)] {
            let p = Parameters::new(x, z);
            for ticks in [i64::MIN + 53, -1, 0, 511, 131071, i64::MAX] {
                for partial in [0., 0.125, 0.999] {
                    for snow in [false, true] {
                        let mut v = vertex();
                        v.animation = [
                            p.rain_speed,
                            p.rain_tick_offset,
                            p.snow_drift[0],
                            p.snow_drift[1],
                        ];
                        let [u, w] = p.snow_base.map(f64::to_bits);
                        v.bases = [u as u32, (u >> 32) as u32, w as u32, (w >> 32) as u32];
                        v.snow = u32::from(snow);
                        let expected = p.column(snow, ticks, x, 0, 16, z, 0, partial);
                        assert_eq!(
                            phase(&v, Time::new(ticks, partial)).unwrap(),
                            [expected.u_offset, expected.v_offset]
                        );
                    }
                }
            }
        }
        let p = Parameters {
            rain_speed: 3.,
            rain_tick_offset: 0.,
            snow_base: [f64::from_bits(0x3fe0000010000001), 0.25],
            snow_drift: [0.; 2],
        };
        let [u, w] = p.snow_base.map(f64::to_bits);
        let mut v = vertex();
        v.snow = 1;
        v.animation = [0.; 4];
        v.bases = [u as u32, (u >> 32) as u32, w as u32, (w >> 32) as u32];
        assert_eq!(
            phase(&v, Time::new(0, 0.)).unwrap(),
            [f32::from_bits(0x3f000001), 0.25]
        );
    }
    #[test]
    fn extracted_repeat_quads_keep_order_light_alpha_seams_and_patch_visibility() {
        let layout = Layout::new([7, 5], 4, 12).unwrap();
        let pixels: Vec<_> = (0..35u8).flat_map(|n| [n, 2 * n, 255 - n, 255]).collect();
        let mut backing = Backing::new(layout, &pixels, 4096).unwrap();
        let mut lm = [0; 1024];
        for (i, p) in lm.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            p.copy_from_slice(&[(i / 16 * 16) as u8, (i % 16 * 16) as u8, 64, 128]);
        }
        let mut v = vertex();
        v.uv = [-0.25, 1_000_000.25];
        let mut quads = vec![v; 8];
        quads[4..].iter_mut().for_each(|q| q.color = 0x40201008);
        let plan = Prepared::new(&quads, Time::new(31, 0.5), &backing, &lm, 2, 124, 64).unwrap();
        assert_eq!(plan.runs.len(), 2);
        assert_eq!(plan.sprites.len(), 1);
        assert_eq!(
            plan.runs[0].vertices[0].base.uv,
            [-0.25, 1_000_000.25 - 5.3046875]
        ); // never split/wrap on CPU
        assert_eq!(
            plan.runs[0].vertices[0].base.color,
            [16. / 255., 32. / 255., 64. / 255., 128. / 255.]
        );
        assert_eq!(
            plan.runs[1].vertices[0].base.color,
            [8. / 255., 16. / 255., 32. / 255., 64. / 255.]
        );
        assert_eq!(
            plan.runs[0].vertices[0].light_color,
            [128. / 255., 0., 64. / 255., 128. / 255.]
        );
        let expected: Vec<_> = (1..4)
            .flat_map(|y| pixels[(y * 7 + 1) * 4..(y * 7 + 6) * 4].iter().copied())
            .collect();
        assert_eq!(plan.sprites[0].pixels, expected);
        backing
            .patch(
                &[Patch {
                    x: 2,
                    y: 2,
                    width: 1,
                    height: 1,
                    rgba: vec![9; 4],
                }],
                1024,
            )
            .unwrap();
        let plan = Prepared::new(&quads, Time::new(31, 0.5), &backing, &lm, 2, 124, 64).unwrap();
        assert_eq!(&plan.sprites[0].pixels[24..28], &[9; 4]);
    }
    #[test]
    fn rejects_bad_clock_quad_metadata_nonfinite_and_bounded_extraction() {
        let layout = Layout::new([7, 5], 4, 12).unwrap();
        let backing = Backing::new(layout, &[255; 140], 4096).unwrap();
        let lm = [255; 1024];
        let v = vertex();
        let q = [v; 4];
        let time = Time::new(0, 0.);
        let build = |q: &[Source], t: Time, limit, bytes, max| {
            Prepared::new(q, t, &backing, &lm, limit, bytes, max)
        };
        assert!(build(&q[..3], time, 1, 124, 64).is_err());
        assert!(build(&q, time, 0, 124, 64).is_err());
        assert!(build(&q, time, 1, 123, 64).is_err());
        assert!(build(&q, time, 1, 124, 4).is_err());
        for values in [
            [0., 1., 0., 0.],
            [131072., 0., 0., 0.],
            [0.5, 0.5, 0., 0.],
            [0., 0., f32::NAN, 0.],
            [0., 0., 2., 0.],
            [0., 0., 0., 1.],
        ] {
            assert!(build(&q, Time { values }, 1, 124, 64).is_err());
        }
        for bad in 0..8 {
            let mut q = q;
            match bad {
                0 => q[1].rect[2] = 0.,
                1 => q[0].pos[0] = f32::INFINITY,
                2 => q[0].uv[0] = f32::NAN,
                3 => q[0].snow = 2,
                4 => q[0].light = 0x10000,
                5 => q[0].animation[0] = f32::NAN,
                6 => {
                    for v in &mut q {
                        v.snow = 1;
                        v.bases = [0, u32::MAX, 0, 0];
                    }
                }
                _ => {
                    for v in &mut q {
                        v.rect = [0., 0., 0.1, 1.];
                    }
                }
            }
            assert!(build(&q, time, 1, 1024, 64).is_err(), "case {bad}");
        }
        assert!(Prepared::new(&q, time, &backing, &[0; 4], 1, 124, 64).is_err());
    }
}
