//! Ordinary entity/item quad conversion for the partial GL backend.
//! Uses the shared paged HUD atlas. Special overlays (glint, hurt, text,
//! translucent item materials and others) are rejected, not silently dropped.
//! Geometry is camera-relative as in `gfx::Frame::entities`.
use super::{hud::Atlas, terrain::lightmap_rgb};
use crate::mesher::Vertex as SourceVertex;
use horizon_legacy_gl::{Batch, Draw, Faces, Pass, Vertex};

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    IncompleteQuad,
    MixedMaterial,
    UnsupportedOverlay(u32),
    InvalidVertex,
    TooManyVertices,
    AllocationFailed,
}

#[derive(Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub draws: Vec<Draw>,
}

#[derive(Clone, Copy)]
struct Point {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
}
impl Point {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            pos: std::array::from_fn(|i| self.pos[i] + (other.pos[i] - self.pos[i]) * t),
            uv: std::array::from_fn(|i| self.uv[i] + (other.uv[i] - self.uv[i]) * t),
            color: std::array::from_fn(|i| self.color[i] + (other.color[i] - self.color[i]) * t),
        }
    }
}

fn clip(poly: &[Point], axis: usize, limit: f32, lower: bool) -> Vec<Point> {
    let mut output = Vec::new();
    let inside = |p: &Point| {
        if lower {
            p.uv[axis] >= limit
        } else {
            p.uv[axis] <= limit
        }
    };
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let (ia, ib) = (inside(&a), inside(&b));
        if ia {
            output.push(a);
        }
        if ia != ib {
            let t = (limit - a.uv[axis]) / (b.uv[axis] - a.uv[axis]);
            let mut p = a.lerp(b, t);
            p.uv[axis] = limit;
            output.push(p);
        }
    }
    output
}

/// CPU equivalent of `entity.wgsl`'s cardinal light, for one face direction.
fn directional(normal: [i8; 4], nether: bool, back: bool) -> f32 {
    if normal[3].unsigned_abs() < 64 {
        return 1.0;
    }
    let n = [normal[0], normal[1], normal[2]].map(|v| (f32::from(v) / 127.0).max(-1.0));
    let sign = if back { -1.0 } else { 1.0 };
    let inv = 1.0 / 1.53_f32.sqrt();
    let dot = |x: f32, y: f32, z: f32| (sign * (x * n[0] + y * n[1] + z * n[2]) * inv).max(0.0);
    ((dot(0.2, 1.0, -0.7) + dot(-0.2, if nether { -1.0 } else { 1.0 }, 0.7)) * 0.6 + 0.4).min(1.0)
}

/// Expand game quads to paged triangles and bake ordinary entity lighting.
/// A quad crossing pages is UV-clipped; input order is preserved. Two-sided
/// normals emit front/back draws with separate colors. The GL path multiplies
/// vertex tint and light before interpolation, an approximation when both vary
/// within a triangle. Dynamic/colored lights and special overlays are not yet
/// supported. It is a partial adapter, not the game's selectable renderer.
pub fn build(
    atlas: &Atlas,
    vertices: &[SourceVertex],
    lightmap: &[u8; 1024],
    nether: bool,
) -> Result<Mesh, Error> {
    if !vertices.len().is_multiple_of(4) {
        return Err(Error::IncompleteQuad);
    }
    let mut mesh = Mesh::default();
    for quad in vertices.as_chunks::<4>().0 {
        let overlay = quad[0].layer & 0xffff;
        if quad
            .iter()
            .any(|v| v.layer & 0xffff != overlay || v.normal[3] != quad[0].normal[3])
        {
            return Err(Error::MixedMaterial);
        }
        if overlay != 0 {
            return Err(Error::UnsupportedOverlay(overlay));
        }
        if quad.iter().any(|v| {
            v.pos.iter().any(|p| !p.is_finite())
                || v.uv
                    .iter()
                    .any(|u| !u.is_finite() || !(0.0..=1.0).contains(u))
        }) {
            return Err(Error::InvalidVertex);
        }
        let w = quad[0].normal[3];
        let cull_back = (w > 0 && w < 64) || (w < 0 && w > -127);
        let two_sided = w == -127;
        let faces: &[(Faces, bool)] = if two_sided {
            &[(Faces::Front, false), (Faces::Back, true)]
        } else if cull_back {
            &[(Faces::Front, false)]
        } else {
            &[(Faces::Both, false)]
        };
        for &(faces, back) in faces {
            let points = quad.map(|v| {
                let rgba = v.color.to_le_bytes().map(|c| f32::from(c) / 255.0);
                let light = lightmap_rgb(v.light as u16, lightmap);
                let shade = directional(v.normal, nether, back);
                Point {
                    pos: v.pos,
                    uv: [v.uv[0] * atlas.width as f32, v.uv[1] * atlas.height as f32],
                    color: [
                        rgba[0] * light[0] * shade,
                        rgba[1] * light[1] * shade,
                        rgba[2] * light[2] * shade,
                        rgba[3],
                    ],
                }
            });
            for indices in [[0, 1, 2], [0, 2, 3]] {
                let triangle = indices.map(|i| points[i]);
                emit_triangle(atlas, &mut mesh, &triangle, faces)?;
            }
        }
    }
    Ok(mesh)
}

fn emit_triangle(
    atlas: &Atlas,
    mesh: &mut Mesh,
    points: &[Point; 3],
    faces: Faces,
) -> Result<(), Error> {
    let tile = |axis: usize, value: f32| -> u32 {
        let count = if axis == 0 { atlas.columns } else { atlas.rows };
        ((value / atlas.side as f32).floor() as u32).min(count - 1)
    };
    let bounds = |axis: usize| {
        let lo = points
            .iter()
            .map(|p| p.uv[axis])
            .fold(f32::INFINITY, f32::min);
        let hi = points
            .iter()
            .map(|p| p.uv[axis])
            .fold(f32::NEG_INFINITY, f32::max);
        (tile(axis, lo), tile(axis, hi))
    };
    let (x0, x1) = bounds(0);
    let (y0, y1) = bounds(1);
    for py in y0..=y1 {
        for px in x0..=x1 {
            let x = (px * atlas.side) as f32;
            let y = (py * atlas.side) as f32;
            let mut poly = clip(points, 0, x, true);
            poly = clip(&poly, 0, x + atlas.side as f32, false);
            poly = clip(&poly, 1, y, true);
            poly = clip(&poly, 1, y + atlas.side as f32, false);
            let page = py as usize * atlas.columns as usize + px as usize;
            for i in 1..poly.len().saturating_sub(1) {
                let tri = [poly[0], poly[i], poly[i + 1]];
                let a = std::array::from_fn::<_, 3, _>(|j| tri[1].pos[j] - tri[0].pos[j]);
                let b = std::array::from_fn::<_, 3, _>(|j| tri[2].pos[j] - tri[0].pos[j]);
                let cross = [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ];
                if cross == [0.; 3] {
                    continue;
                }
                if mesh.vertices.len() > i32::MAX as usize - 3 {
                    return Err(Error::TooManyVertices);
                }
                let pass = Pass::Entity { faces };
                if mesh
                    .draws
                    .last()
                    .is_none_or(|d| d.batch.page != page || d.pass != pass)
                {
                    mesh.draws
                        .try_reserve(1)
                        .map_err(|_| Error::AllocationFailed)?;
                    mesh.draws.push(Draw {
                        batch: Batch {
                            page,
                            first: mesh.vertices.len(),
                            count: 0,
                        },
                        pass,
                    });
                }
                mesh.vertices
                    .try_reserve(3)
                    .map_err(|_| Error::AllocationFailed)?;
                for p in tri {
                    mesh.vertices.push(Vertex {
                        position: p.pos,
                        sprite_uv: [
                            (p.uv[0] - x) / atlas.side as f32,
                            (p.uv[1] - y) / atlas.side as f32,
                        ],
                        atlas_rect: [0., 0., 1., 1.],
                        color: p.color,
                    });
                }
                mesh.draws
                    .last_mut()
                    .expect("draw was just added")
                    .batch
                    .count += 3;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> [SourceVertex; 4] {
        [[0., 0.], [1., 0.], [1., 1.], [0., 1.]].map(|p| SourceVertex {
            pos: [p[0], p[1], 0.],
            uv: p,
            layer: 0,
            color: 0xffff_ffff,
            light: 0,
            normal: [0, 127, 0, -127],
            glint_uv: [0.; 2],
        })
    }
    #[test]
    fn split_page_preserves_both_face_colors_and_order() {
        let atlas = Atlas::new(8, 4, &[255; 128], 4).unwrap();
        let mesh = build(&atlas, &quad(), &[255; 1024], false).unwrap();
        assert!(mesh.draws.iter().any(|d| d.batch.page == 0
            && d.pass
                == Pass::Entity {
                    faces: Faces::Front
                }));
        assert!(
            mesh.draws
                .iter()
                .any(|d| d.batch.page == 1 && d.pass == Pass::Entity { faces: Faces::Back })
        );
        for draw in &mesh.draws {
            let red = mesh.vertices[draw.batch.first].color[0];
            match draw.pass {
                Pass::Entity {
                    faces: Faces::Front,
                } => assert!((red - 1.).abs() < 1e-6),
                Pass::Entity { faces: Faces::Back } => assert!((red - 0.4).abs() < 1e-6),
                _ => panic!("unexpected pass"),
            }
        }
    }
    #[test]
    fn rejects_special_materials_and_invalid_geometry() {
        let atlas = Atlas::new(1, 1, &[255; 4], 4).unwrap();
        let mut q = quad();
        assert_eq!(
            build(&atlas, &q[..3], &[255; 1024], false).unwrap_err(),
            Error::IncompleteQuad
        );
        for v in &mut q {
            v.layer = 7;
        }
        assert_eq!(
            build(&atlas, &q, &[255; 1024], false).unwrap_err(),
            Error::UnsupportedOverlay(7)
        );
        q[0].layer = 0;
        assert_eq!(
            build(&atlas, &q, &[255; 1024], false).unwrap_err(),
            Error::MixedMaterial
        );
        q[0].layer = 7;
        for v in &mut q {
            v.layer = 0;
        }
        q[0].uv[0] = f32::NAN;
        assert_eq!(
            build(&atlas, &q, &[255; 1024], false).unwrap_err(),
            Error::InvalidVertex
        );
    }
    #[test]
    fn culling_flags_and_nether_directional_light() {
        let atlas = Atlas::new(1, 1, &[255; 4], 4).unwrap();
        let mut q = quad();
        for v in &mut q {
            v.normal = [0, 127, 0, -126];
        }
        let standard = build(&atlas, &q, &[255; 1024], false).unwrap();
        let nether = build(&atlas, &q, &[255; 1024], true).unwrap();
        assert!(standard.draws.iter().all(|d| d.pass
            == Pass::Entity {
                faces: Faces::Front
            }));
        assert!(standard.vertices[0].color[0] > nether.vertices[0].color[0]);
        for v in &mut q {
            v.normal = [0, 0, 0, 0];
        }
        let unlit = build(&atlas, &q, &[255; 1024], false).unwrap();
        assert!(
            unlit
                .draws
                .iter()
                .all(|d| d.pass == Pass::Entity { faces: Faces::Both })
        );
    }
}
