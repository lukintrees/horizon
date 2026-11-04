//! Classic SDR entity contract, matching the game's camera-relative quad stream.
//! Lighting is sampled on the CPU: GL2 does not guarantee vertex texture fetch.
use crate::{Pass, Vertex, gui};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Overlay {
    None,
    Hurt,
    White(u8),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Material {
    pub overlay: Overlay,
    pub cull: bool,
    /// Translucent particles draw after terrain without depth writes. Ordinary
    /// entities and translucent items retain depth writes in the classic path.
    pub particle: bool,
}
impl Material {
    pub fn overlay_color(self) -> Option<[f32; 4]> {
        Some(match self.overlay {
            Overlay::None => [0., 0., 0., 1.],
            Overlay::Hurt => [1., 0., 0., 178. / 255.],
            Overlay::White(u) if u <= 15 => {
                let alpha = ((1. - f32::from(u) / 15. * 0.75) * 255.) as u8;
                [1., 1., 1., f32::from(alpha) / 255.]
            }
            Overlay::White(_) => return None,
        })
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LitVertex {
    pub base: Vertex,
    pub back_color: [f32; 4],
    pub light_color: [f32; 4],
}
#[derive(Clone, Copy, Debug)]
pub struct Input {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub layer: u32,
    pub color: u32,
    pub light: u32,
    pub normal: [i8; 4],
}
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidLightmap,
    IncompleteQuad,
    MixedMaterial,
    UnsupportedMaterial(u32),
    InvalidVertex,
    VertexBudget,
}
fn shade(normal: [f32; 3], nether: bool) -> f32 {
    let length = 1.53f32.sqrt();
    let dot = |v: [f32; 3]| {
        normal
            .iter()
            .zip(v)
            .map(|(a, b)| a * b / length)
            .sum::<f32>()
            .max(0.)
    };
    ((dot([0.2, 1., -0.7]) + dot([-0.2, if nether { -1. } else { 1. }, 0.7])) * 0.6 + 0.4).min(1.)
}
fn sample(lightmap: &[u8], light: u32) -> [f32; 4] {
    let x = (((light >> 8) & 255) as f32 / 16.).min(15.);
    let y = ((light & 255) as f32 / 16.).min(15.);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(15), (y0 + 1).min(15));
    std::array::from_fn(|c| {
        let at = |x: usize, y: usize| f32::from(lightmap[(y * 16 + x) * 4 + c]) / 255.;
        let row = |y| at(x0, y) + (at(x1, y) - at(x0, y)) * (x - x0 as f32);
        row(y0) + (row(y1) - row(y0)) * (y - y0 as f32)
    })
}
pub fn runs(
    input: impl IntoIterator<Item = Input>,
    lightmap: &[u8],
    nether: bool,
    max_vertices: usize,
) -> Result<Vec<gui::Run<LitVertex>>, Error> {
    if lightmap.len() != 16 * 16 * 4 {
        return Err(Error::InvalidLightmap);
    }
    let mut iter = input.into_iter();
    let mut total = 0usize;
    let mut result: Vec<gui::Run<LitVertex>> = Vec::new();
    while let Some(first) = iter.next() {
        let quad = [
            first,
            iter.next().ok_or(Error::IncompleteQuad)?,
            iter.next().ok_or(Error::IncompleteQuad)?,
            iter.next().ok_or(Error::IncompleteQuad)?,
        ];
        total = total
            .checked_add(4)
            .filter(|&n| n <= max_vertices)
            .ok_or(Error::VertexBudget)?;
        if quad
            .iter()
            .any(|v| v.layer != first.layer || v.normal[3] != first.normal[3])
        {
            return Err(Error::MixedMaterial);
        }
        // HDR tier and classic-path OIT tag do not change SDR shading. Unknown
        // high flags and shader-specialised overlays are deliberately rejected.
        if first.layer & !0x01ff_ffff != 0 {
            return Err(Error::UnsupportedMaterial(first.layer));
        }
        let code = first.layer & 0xffff;
        let overlay = match code {
            0 | 15 | 32 => Overlay::None,
            1 => Overlay::Hurt,
            5 => Overlay::White(15),
            16..=31 => Overlay::White((code - 16) as u8),
            _ => return Err(Error::UnsupportedMaterial(code)),
        };
        let w = (f32::from(first.normal[3]) / 127.).max(-1.);
        let material = Material {
            overlay,
            cull: (w > 0. && w < 0.5) || (w < 0. && w > -0.996),
            particle: code == 15,
        };
        if quad.iter().any(|v| {
            v.pos.iter().chain(&v.uv).any(|f| !f.is_finite())
                || v.uv.iter().any(|f| !(0.0..=1.0).contains(f))
        }) {
            return Err(Error::InvalidVertex);
        }
        let vertices = quad.map(|v| {
            let color = v.color.to_le_bytes().map(|b| f32::from(b) / 255.);
            let normal = [v.normal[0], v.normal[1], v.normal[2]];
            let normal: [f32; 3] = normal.map(|b: i8| (f32::from(b) / 127.).max(-1.));
            let front = if w.abs() >= 0.5 {
                shade(normal, nether)
            } else {
                1.
            };
            let back = if w <= -0.5 {
                shade(normal.map(|v| -v), nether)
            } else {
                front
            };
            let tint = |shade| {
                [
                    color[0] * shade,
                    color[1] * shade,
                    color[2] * shade,
                    color[3],
                ]
            };
            LitVertex {
                base: Vertex {
                    pos: v.pos,
                    uv: v.uv,
                    light_uv: crate::light_uv(v.light as u16),
                    color: tint(front),
                },
                back_color: tint(back),
                light_color: sample(lightmap, v.light),
            }
        });
        let pass = Pass::Entity(material);
        if result
            .last()
            .is_none_or(|r| r.pass != pass || r.vertices.len() > 65532)
        {
            result.push(gui::Run {
                pass,
                scissor: None,
                clear_depth: false,
                vertices: Vec::new(),
                indices: Vec::new(),
            });
        }
        let run = result.last_mut().unwrap();
        let base = run.vertices.len() as u16;
        run.vertices.extend(vertices);
        run.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> Input {
        Input {
            pos: [0.; 3],
            uv: [0.5; 2],
            layer: 0,
            color: u32::MAX,
            light: 0xf0f0,
            normal: [0, 127, 0, -127],
        }
    }
    #[test]
    fn two_sided_normals_overlay_and_light_interpolation_match_shader_order() {
        let mut light = vec![0; 1024];
        for y in 0..16 {
            for x in 0..16 {
                light[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4].copy_from_slice(&[
                    (x * 16) as u8,
                    (y * 16) as u8,
                    64,
                    255,
                ]);
            }
        }
        let v = Input {
            light: 0x8848,
            layer: 1,
            ..input()
        };
        let runs = runs([v; 4], &light, false, 4).unwrap();
        let vertex = runs[0].vertices[0];
        assert_eq!(vertex.base.color, [1.; 4]);
        assert_eq!(vertex.back_color, [0.4, 0.4, 0.4, 1.]);
        assert_eq!(
            vertex.light_color,
            [136. / 255., 72. / 255., 64. / 255., 1.]
        );
        assert_eq!(
            runs[0].pass,
            Pass::Entity(Material {
                overlay: Overlay::Hurt,
                cull: false,
                particle: false
            })
        );
    }
    #[test]
    fn rejects_special_materials_bad_geometry_and_budgets_without_dropping_quads() {
        for layer in [2, 7, 9, 14, 33, 35, 38, 1 << 31] {
            assert!(matches!(
                runs([Input { layer, ..input() }; 4], &[255; 1024], false, 4),
                Err(Error::UnsupportedMaterial(_))
            ));
        }
        assert_eq!(
            runs([input(); 3], &[255; 1024], false, 4).err(),
            Some(Error::IncompleteQuad)
        );
        assert_eq!(
            runs([input(); 4], &[255; 1024], false, 3).err(),
            Some(Error::VertexBudget)
        );
        let mut quad = [input(); 4];
        quad[3].normal[3] = -126;
        assert_eq!(
            runs(quad, &[255; 1024], false, 4).err(),
            Some(Error::MixedMaterial)
        );
        quad = [input(); 4];
        quad[3].uv[0] = f32::NAN;
        assert_eq!(
            runs(quad, &[255; 1024], false, 4).err(),
            Some(Error::InvalidVertex)
        );
        assert_eq!(
            runs([], &[0; 4], false, 4).err(),
            Some(Error::InvalidLightmap)
        );
    }
}
