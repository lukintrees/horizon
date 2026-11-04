//! Camera overlays sampled from a paged GUI atlas. Sprites that cross a page
//! boundary cannot be represented by one GL 2 texture and are reported omitted.
use crate::{
    legacy::hud::Atlas,
    screen_effects::{Blend, SAMPLE_BLUR, SAMPLE_CLAMP, ScreenQuad},
};
use horizon_legacy_gl::{Batch, Draw, Pass, Vertex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidGeometry,
    UnsupportedSprite,
    AllocationFailed,
    SizeOverflow,
}

#[derive(Default, Debug)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub draws: Vec<Draw>,
    pub omitted: usize,
}

/// Preserve overlay order. Rectangles are flat attributes as in screen.wgsl.
/// No HDR light buffer exists here; `world_tint` uses ordinary alpha blending.
pub fn build(atlas: &Atlas, quads: &[ScreenQuad], skip_unsupported: bool) -> Result<Mesh, Error> {
    let mut output = Mesh::default();
    for quad in quads {
        let rect = quad.vertices[0].rect;
        if quad.sampling & !(SAMPLE_BLUR | SAMPLE_CLAMP) != 0
            || rect
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || rect[0] >= rect[2]
            || rect[1] >= rect[3]
            || quad.vertices.iter().any(|v| {
                v.rect != rect
                    || v.pos
                        .iter()
                        .chain(v.uv.iter())
                        .chain(v.color.iter())
                        .any(|x| !x.is_finite())
            })
        {
            return Err(Error::InvalidGeometry);
        }
        let pixel = |v: f32, size: u32| {
            let v = v * size as f32;
            if (v - v.round()).abs() < 0.001 {
                v.round()
            } else {
                v
            }
        };
        let min = [pixel(rect[0], atlas.width), pixel(rect[1], atlas.height)];
        let max = [pixel(rect[2], atlas.width), pixel(rect[3], atlas.height)];
        let side = atlas.side as f32;
        let px = (min[0] / side).floor() as u32;
        let py = (min[1] / side).floor() as u32;
        // At an exact page boundary the upper edge still belongs to the
        // preceding page. `ceil` avoids assigning a zero-width span to the next.
        let end_x = (max[0] / side).ceil() as u32;
        let end_y = (max[1] / side).ceil() as u32;
        let unsupported = px >= atlas.columns
            || py >= atlas.rows
            || end_x != px + 1
            || end_y != py + 1
            || max[0] - min[0] < 1.0
            || max[1] - min[1] < 1.0;
        if unsupported {
            if !skip_unsupported {
                return Err(Error::UnsupportedSprite);
            }
            output.omitted = output.omitted.checked_add(1).ok_or(Error::SizeOverflow)?;
            continue;
        }
        if output.vertices.len() > i32::MAX as usize - 6 {
            return Err(Error::SizeOverflow);
        }
        output
            .vertices
            .try_reserve(6)
            .map_err(|_| Error::AllocationFailed)?;
        output
            .draws
            .try_reserve(1)
            .map_err(|_| Error::AllocationFailed)?;
        let page = py as usize * atlas.columns as usize + px as usize;
        let page_rect = [
            (min[0] - px as f32 * side) / side,
            (min[1] - py as f32 * side) / side,
            (max[0] - min[0]) / side,
            (max[1] - min[1]) / side,
        ];
        let first = output.vertices.len();
        for i in [0, 1, 2, 0, 2, 3] {
            let v = quad.vertices[i];
            output.vertices.push(Vertex {
                position: [v.pos[0], v.pos[1], 0.0],
                sprite_uv: v.uv,
                atlas_rect: page_rect,
                color: v.color,
            });
        }
        output.draws.push(Draw {
            batch: Batch {
                page,
                first,
                count: 6,
            },
            pass: Pass::Screen {
                blur: quad.sampling & SAMPLE_BLUR != 0,
                clamp: quad.sampling & SAMPLE_CLAMP != 0,
                vignette: quad.blend == Blend::Vignette,
            },
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen_effects::ScreenVertex;

    fn quad(rect: [f32; 4], sampling: u32, blend: Blend) -> ScreenQuad {
        ScreenQuad {
            vertices: [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]].map(|pos| ScreenVertex {
                pos,
                uv: [pos[0] * 2., pos[1] * 2.],
                rect,
                color: [1.; 4],
            }),
            blend,
            sampling,
            world_tint: false,
        }
    }

    #[test]
    fn pages_and_modes_preserve_order() {
        let atlas = Atlas::new(8, 4, &[255; 8 * 4 * 4], 4).unwrap();
        let a = quad([0., 0., 0.5, 1.], 0, Blend::Alpha);
        let b = quad(
            [0.5, 0., 1., 1.],
            SAMPLE_CLAMP | SAMPLE_BLUR,
            Blend::Vignette,
        );
        let mesh = build(&atlas, &[a, b], false).unwrap();
        assert_eq!(mesh.vertices.len(), 12);
        assert_eq!(
            mesh.draws.iter().map(|d| d.batch.page).collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(
            mesh.draws[1].pass,
            Pass::Screen {
                blur: true,
                clamp: true,
                vignette: true
            }
        );
        assert_eq!(mesh.vertices[6].atlas_rect, [0., 0., 1., 1.]);
    }

    #[test]
    fn crossing_page_is_reported_and_invalid_geometry_is_rejected() {
        let atlas = Atlas::new(8, 4, &[255; 8 * 4 * 4], 4).unwrap();
        let crossing = quad([0.25, 0., 0.75, 1.], 0, Blend::Alpha);
        assert!(matches!(
            build(&atlas, std::slice::from_ref(&crossing), false),
            Err(Error::UnsupportedSprite)
        ));
        let partial = build(&atlas, &[crossing], true).unwrap();
        assert_eq!(partial.omitted, 1);
        assert!(partial.vertices.is_empty());
        let mut invalid = quad([0., 0., 0.5, 1.], 0, Blend::Alpha);
        invalid.vertices[2].uv[0] = f32::NAN;
        assert!(matches!(
            build(&atlas, &[invalid], true),
            Err(Error::InvalidGeometry)
        ));
    }
}
