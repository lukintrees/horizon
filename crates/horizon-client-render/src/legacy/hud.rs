//! A limited, paged HUD path for GL 2.1. The shared GUI atlas is often wider
//! than old GPUs support, so split it into power-of-two pages and clip each
//! triangle in UV space. Consecutive runs preserve transparent draw order.
//! Glint, depth-scissored 3D previews and other special HUD pipelines are not
//! implemented; they return an error instead of silently drawing incorrectly.
use crate::hud::{Blend, HudAtlas as SourceAtlas, HudBatch};
use horizon_legacy_gl::{Batch, Pass, Vertex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidDimensions,
    InvalidPixels,
    InvalidPatch,
    InvalidViewport,
    InvalidTriangles,
    UnsupportedBlend,
    UnsupportedUv,
    SizeOverflow,
    AllocationFailed,
}

#[derive(Debug)]
pub struct Page {
    pub side: u32,
    pub pixels: Vec<u8>,
}

/// Tightly packed square pages, padded with zeros at the outer atlas edges.
/// Use nearest sampling and no mipmaps, matching the basic HUD sampler.
pub struct Atlas {
    width: u32,
    height: u32,
    side: u32,
    columns: u32,
    rows: u32,
    pages: Vec<Page>,
}

impl Atlas {
    pub fn from_hud(source: &SourceAtlas, max_texture_size: u32) -> Result<Self, Error> {
        Self::new(
            source.width,
            source.height,
            &source.pixels(),
            max_texture_size,
        )
    }

    pub fn new(
        width: u32,
        height: u32,
        pixels: &[u8],
        max_texture_size: u32,
    ) -> Result<Self, Error> {
        if width == 0 || height == 0 || max_texture_size == 0 {
            return Err(Error::InvalidDimensions);
        }
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(Error::SizeOverflow)?;
        if pixels.len() != len {
            return Err(Error::InvalidPixels);
        }
        let limit = 1u32 << (31 - max_texture_size.leading_zeros());
        let side = width
            .max(height)
            .checked_next_power_of_two()
            .unwrap_or(u32::MAX)
            .min(limit);
        let columns = (width - 1) / side + 1;
        let rows = (height - 1) / side + 1;
        let count = (columns as usize)
            .checked_mul(rows as usize)
            .ok_or(Error::SizeOverflow)?;
        let page_bytes = (side as usize)
            .checked_mul(side as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(Error::SizeOverflow)?;
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(count)
            .map_err(|_| Error::AllocationFailed)?;
        for _ in 0..count {
            let mut data = Vec::new();
            data.try_reserve_exact(page_bytes)
                .map_err(|_| Error::AllocationFailed)?;
            data.resize(page_bytes, 0);
            pages.push(Page { side, pixels: data });
        }
        let mut atlas = Self {
            width,
            height,
            side,
            columns,
            rows,
            pages,
        };
        atlas.update_rect(0, 0, width, height, pixels)?;
        Ok(atlas)
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    /// Update atlas pixels (e.g. dynamic glyphs). Returns the page indices that
    /// need a GL upload. On invalid input no pixels are modified.
    pub fn update_rect(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Result<Vec<usize>, Error> {
        let right = x.checked_add(width).ok_or(Error::InvalidPatch)?;
        let bottom = y.checked_add(height).ok_or(Error::InvalidPatch)?;
        let bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(Error::SizeOverflow)?;
        if right > self.width || bottom > self.height || pixels.len() != bytes {
            return Err(Error::InvalidPatch);
        }
        if width == 0 || height == 0 {
            return Ok(Vec::new());
        }
        let mut changed = Vec::new();
        let first_x = x / self.side;
        let last_x = (right - 1) / self.side;
        let first_y = y / self.side;
        let last_y = (bottom - 1) / self.side;
        let total = ((last_x - first_x + 1) as usize)
            .checked_mul((last_y - first_y + 1) as usize)
            .ok_or(Error::SizeOverflow)?;
        changed
            .try_reserve_exact(total)
            .map_err(|_| Error::AllocationFailed)?;
        for py in first_y..=last_y {
            for px in first_x..=last_x {
                let index = (py as usize) * self.columns as usize + px as usize;
                let left = x.max(px * self.side);
                let top = y.max(py * self.side);
                let end_x = right.min((px + 1).saturating_mul(self.side));
                let end_y = bottom.min((py + 1).saturating_mul(self.side));
                for row in top..end_y {
                    let source = (((row - y) as usize * width as usize) + (left - x) as usize) * 4;
                    let dest = (((row - py * self.side) as usize * self.side as usize)
                        + (left - px * self.side) as usize)
                        * 4;
                    let len = (end_x - left) as usize * 4;
                    self.pages[index].pixels[dest..dest + len]
                        .copy_from_slice(&pixels[source..source + len]);
                }
                changed.push(index);
            }
        }
        Ok(changed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiBlend {
    Alpha,
    Invert,
    Additive,
}
impl UiBlend {
    pub fn pass(self) -> Pass {
        match self {
            Self::Alpha => Pass::HudAlpha,
            Self::Invert => Pass::HudInvert,
            Self::Additive => Pass::HudAdditive,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub batch: Batch,
    pub blend: UiBlend,
}
#[derive(Default, Debug)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub runs: Vec<Run>,
}

impl Mesh {
    /// Convert to the backend's ordered draws so the vertex buffer is uploaded once.
    pub fn draws(&self) -> Vec<horizon_legacy_gl::Draw> {
        self.runs
            .iter()
            .map(|run| horizon_legacy_gl::Draw {
                batch: run.batch,
                pass: run.blend.pass(),
            })
            .collect()
    }
}

#[derive(Clone, Copy)]
struct Point {
    pos: [f32; 2],
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

/// Clip against a tile edge in the original atlas pixel-coordinate space.
fn clip(poly: &[Point], axis: usize, limit: f32, lower: bool) -> Vec<Point> {
    let mut result = Vec::new();
    if poly.is_empty() {
        return result;
    }
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
            result.push(a);
        }
        if ia != ib {
            let t = (limit - a.uv[axis]) / (b.uv[axis] - a.uv[axis]);
            let mut p = a.lerp(b, t);
            p.uv[axis] = limit;
            result.push(p);
        }
    }
    result
}

/// Convert the existing HUD triangles and their blend runs. Window dimensions
/// are framebuffer pixels; `HudBatch::scale` maps GUI pixels to the window.
/// Atlas patch uploads are a separate step: apply `batch.patches` via
/// `Atlas::update_rect`, then update the returned pages on the GPU.
pub fn build(atlas: &Atlas, batch: &HudBatch, window: [u32; 2]) -> Result<Mesh, Error> {
    if window.contains(&0) || !batch.scale.is_finite() || batch.scale <= 0.0 {
        return Err(Error::InvalidViewport);
    }
    let mut mesh = Mesh::default();
    let mut expected = 0usize;
    for (range, blend) in batch.runs() {
        let blend = match blend {
            Blend::Alpha => UiBlend::Alpha,
            Blend::Invert => UiBlend::Invert,
            Blend::Additive => UiBlend::Additive,
            _ => return Err(Error::UnsupportedBlend),
        };
        let start = range.start as usize;
        let end = range.end as usize;
        if start != expected
            || end < start
            || end > batch.vertices.len()
            || !start.is_multiple_of(3)
            || !end.is_multiple_of(3)
        {
            return Err(Error::InvalidTriangles);
        }
        expected = end;
        for triangle in batch.vertices[start..end].chunks_exact(3) {
            let mut points = Vec::with_capacity(3);
            for v in triangle {
                if v.uv.iter().any(|u| !u.is_finite() || *u < 0.0 || *u > 1.0)
                    || v.pos.iter().any(|p| !p.is_finite())
                {
                    return Err(Error::UnsupportedUv);
                }
                let rgba = v.color.to_le_bytes();
                points.push(Point {
                    pos: v.pos,
                    uv: [v.uv[0] * atlas.width as f32, v.uv[1] * atlas.height as f32],
                    color: rgba.map(|c| f32::from(c) / 255.0),
                });
            }
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
                    let mut poly = clip(&points, 0, x, true);
                    poly = clip(&poly, 0, x + atlas.side as f32, false);
                    poly = clip(&poly, 1, y, true);
                    poly = clip(&poly, 1, y + atlas.side as f32, false);
                    if poly.len() < 3 {
                        continue;
                    }
                    let page = py as usize * atlas.columns as usize + px as usize;
                    for i in 1..poly.len() - 1 {
                        let tri = [poly[0], poly[i], poly[i + 1]];
                        let area = (tri[1].pos[0] - tri[0].pos[0])
                            * (tri[2].pos[1] - tri[0].pos[1])
                            - (tri[1].pos[1] - tri[0].pos[1]) * (tri[2].pos[0] - tri[0].pos[0]);
                        if area == 0.0 {
                            continue;
                        }
                        if mesh.vertices.len() > i32::MAX as usize - 3 {
                            return Err(Error::SizeOverflow);
                        }
                        if mesh
                            .runs
                            .last()
                            .is_none_or(|r| r.batch.page != page || r.blend != blend)
                        {
                            mesh.runs
                                .try_reserve(1)
                                .map_err(|_| Error::AllocationFailed)?;
                            mesh.runs.push(Run {
                                batch: Batch {
                                    page,
                                    first: mesh.vertices.len(),
                                    count: 0,
                                },
                                blend,
                            });
                        }
                        mesh.vertices
                            .try_reserve(3)
                            .map_err(|_| Error::AllocationFailed)?;
                        for p in tri {
                            mesh.vertices.push(Vertex {
                                position: [
                                    p.pos[0] * batch.scale * 2.0 / window[0] as f32 - 1.0,
                                    1.0 - p.pos[1] * batch.scale * 2.0 / window[1] as f32,
                                    0.0,
                                ],
                                sprite_uv: [
                                    (p.uv[0] - x) / atlas.side as f32,
                                    (p.uv[1] - y) / atlas.side as f32,
                                ],
                                atlas_rect: [0.0, 0.0, 1.0, 1.0],
                                color: p.color,
                            });
                        }
                        mesh.runs
                            .last_mut()
                            .expect("run was just added")
                            .batch
                            .count += 3;
                    }
                }
            }
        }
    }
    if expected != batch.vertices.len() {
        return Err(Error::InvalidTriangles);
    }
    Ok(mesh)
}

/// Convenience to match the HUD atlas API's pixel-space patch.
pub fn apply_patch(atlas: &mut Atlas, patch: &crate::font::Patch) -> Result<Vec<usize>, Error> {
    atlas.update_rect(patch.x, patch.y, patch.width, patch.height, &patch.rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::HudVertex;

    fn make_batch() -> HudBatch {
        let mut batch = HudBatch::new([32, 32]);
        batch.scale = 1.0;
        batch.quad(
            [[0., 0.], [32., 0.], [32., 32.], [0., 32.]],
            [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
            u32::MAX,
        );
        batch
    }

    #[test]
    fn pages_and_patches_cross_boundaries_without_corruption() {
        let pixels: Vec<u8> = (0..32).flat_map(|n| [n, 0, 0, 255]).collect();
        let mut atlas = Atlas::new(8, 4, &pixels, 4).unwrap();
        assert_eq!(atlas.pages().len(), 2);
        assert_eq!(atlas.pages()[0].pixels[0], 0);
        assert_eq!(atlas.pages()[1].pixels[0], 4);
        let original = atlas.pages()[0].pixels.clone();
        assert_eq!(
            atlas.update_rect(7, 0, 2, 1, &[0; 8]),
            Err(Error::InvalidPatch)
        );
        assert_eq!(atlas.pages()[0].pixels, original);
        assert_eq!(atlas.update_rect(3, 1, 2, 1, &[9; 8]).unwrap(), [0, 1]);
        assert_eq!(atlas.pages()[0].pixels[(4 + 3) * 4], 9);
        assert_eq!(atlas.pages()[1].pixels[4 * 4], 9);
        assert_eq!(atlas.pages()[1].pixels[(4 + 1) * 4], 13);
    }

    #[test]
    fn clipping_preserves_geometry_and_batches_by_page() {
        let atlas = Atlas::new(8, 4, &[255; 8 * 4 * 4], 4).unwrap();
        let mesh = build(&atlas, &make_batch(), [32, 32]).unwrap();
        assert!(mesh.runs.iter().any(|r| r.batch.page == 0));
        assert!(mesh.runs.iter().any(|r| r.batch.page == 1));
        assert!(
            mesh.vertices
                .iter()
                .all(|v| v.sprite_uv.iter().all(|u| (0.0..=1.0).contains(u)))
        );
        // The clipped triangles cover the original clip-space 2x2 square.
        let area: f32 = mesh
            .vertices
            .chunks_exact(3)
            .map(|t| {
                let a = t[0].position;
                let b = t[1].position;
                let c = t[2].position;
                ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() * 0.5
            })
            .sum();
        assert!((area - 4.0).abs() < 1e-5, "{area}");
    }

    #[test]
    fn solid_color_at_tile_boundary_is_not_drawn_twice() {
        let atlas = Atlas::new(8, 4, &[255; 8 * 4 * 4], 4).unwrap();
        let mut batch = make_batch();
        batch.vertices.iter_mut().for_each(|v| v.uv = [0.5, 0.5]);
        let mesh = build(&atlas, &batch, [32, 32]).unwrap();
        assert_eq!(mesh.vertices.len(), 6);
        assert!(mesh.runs.iter().all(|r| r.batch.page == 1));
    }

    #[test]
    fn preserves_blend_order_after_splitting() {
        let atlas = Atlas::new(8, 4, &[255; 8 * 4 * 4], 4).unwrap();
        let mut batch = make_batch();
        batch.blends.push((3..6, Blend::Invert));
        let mesh = build(&atlas, &batch, [32, 32]).unwrap();
        let first_invert = mesh
            .runs
            .iter()
            .position(|run| run.blend == UiBlend::Invert)
            .unwrap();
        assert!(
            mesh.runs[..first_invert]
                .iter()
                .all(|run| run.blend == UiBlend::Alpha)
        );
        assert!(
            mesh.runs[first_invert..]
                .iter()
                .all(|run| run.blend == UiBlend::Invert)
        );
    }

    #[test]
    fn unsupported_paths_and_invalid_inputs_are_explicit() {
        let atlas = Atlas::new(8, 4, &[255; 8 * 4 * 4], 4).unwrap();
        let mut batch = make_batch();
        batch.vertices[0].uv[0] = 2.0; // vanilla glint encoding
        assert_eq!(
            build(&atlas, &batch, [32, 32]).unwrap_err(),
            Error::UnsupportedUv
        );
        batch.vertices[0].uv[0] = f32::NAN;
        assert_eq!(
            build(&atlas, &batch, [32, 32]).unwrap_err(),
            Error::UnsupportedUv
        );
        batch.vertices[0] = HudVertex {
            pos: [0., 0.],
            uv: [0., 0.],
            color: u32::MAX,
            depth: 0.,
            glint_uv: [0., 0.],
            text: 0,
        };
        batch.blends.push((0..6, Blend::EndPortal));
        assert_eq!(
            build(&atlas, &batch, [32, 32]).unwrap_err(),
            Error::UnsupportedBlend
        );
        assert_eq!(
            build(&atlas, &batch, [0, 32]).unwrap_err(),
            Error::InvalidViewport
        );
    }
}
