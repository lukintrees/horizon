//! Virtual GUI atlas: triangle clipping, one-texel gutters and bounded GL2 residency.
//! No compression/decoder dependency. The caller supplies a checked page loader.
use crate::{Image, RenderBackend, TextureId, Vertex, gui};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    size: [u32; 2],
    side: u32,
    span: u32,
    grid: [u32; 2],
}
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidLayout,
    InvalidPage,
    InvalidPixels,
    InvalidMesh,
    VertexBudget,
    OutOfMemory,
}
impl Layout {
    pub fn new(size: [u32; 2], side: u32, max_pages: usize) -> Result<Self, Error> {
        if size.contains(&0) || !side.is_power_of_two() || !(4..=4096).contains(&side) {
            return Err(Error::InvalidLayout);
        }
        let span = side - 2;
        let grid = size.map(|v| v.div_ceil(span));
        let count = (grid[0] as usize)
            .checked_mul(grid[1] as usize)
            .ok_or(Error::InvalidLayout)?;
        if count > max_pages {
            return Err(Error::InvalidLayout);
        }
        Ok(Self {
            size,
            side,
            span,
            grid,
        })
    }
    pub fn size(self) -> [u32; 2] {
        self.size
    }
    pub fn page_size(self) -> u32 {
        self.side
    }
    pub fn page_bytes(self) -> usize {
        self.side as usize * self.side as usize * 4
    }
    pub fn page_count(self) -> usize {
        self.grid[0] as usize * self.grid[1] as usize
    }
    pub fn origin(self, page: usize) -> Result<[u32; 2], Error> {
        if page >= self.page_count() {
            return Err(Error::InvalidPage);
        }
        Ok([
            (page % self.grid[0] as usize) as u32 * self.span,
            (page / self.grid[0] as usize) as u32 * self.span,
        ])
    }
    /// Source texel corresponding to a page texel, including repeated outer edges.
    pub fn source_texel(self, page: usize, at: [u32; 2]) -> Result<[u32; 2], Error> {
        if at.iter().any(|&v| v >= self.side) {
            return Err(Error::InvalidPixels);
        }
        let origin = self.origin(page)?;
        Ok(std::array::from_fn(|i| {
            (u64::from(origin[i]) + u64::from(at[i]))
                .saturating_sub(1)
                .min(u64::from(self.size[i] - 1)) as u32
        }))
    }
    pub fn pixels(self, page: usize, source: &[u8]) -> Result<Vec<u8>, Error> {
        self.origin(page)?;
        let length = (self.size[0] as usize)
            .checked_mul(self.size[1] as usize)
            .and_then(|n| n.checked_mul(4));
        if length != Some(source.len()) {
            return Err(Error::InvalidPixels);
        }
        let mut result = Vec::new();
        result
            .try_reserve_exact(self.page_bytes())
            .map_err(|_| Error::OutOfMemory)?;
        result.resize(self.page_bytes(), 0);
        for y in 0..self.side {
            for x in 0..self.side {
                let [sx, sy] = self.source_texel(page, [x, y])?;
                let src = (sy as usize * self.size[0] as usize + sx as usize) * 4;
                let dst = (y as usize * self.side as usize + x as usize) * 4;
                result[dst..dst + 4].copy_from_slice(&source[src..src + 4]);
            }
        }
        Ok(result)
    }
    /// Include neighbouring gutters: editing a border texel dirties both tiles.
    pub fn affected(self, at: [u32; 2], size: [u32; 2]) -> Result<Vec<usize>, Error> {
        if size.contains(&0) {
            return Err(Error::InvalidPixels);
        }
        let mut lo = [0; 2];
        let mut hi = [0; 2];
        for i in 0..2 {
            let end = at[i]
                .checked_add(size[i])
                .filter(|&v| v <= self.size[i])
                .ok_or(Error::InvalidPixels)?;
            lo[i] = at[i].saturating_sub(1) / self.span;
            hi[i] = end.min(self.size[i] - 1) / self.span;
        }
        Ok((lo[1]..=hi[1])
            .flat_map(|y| {
                (lo[0]..=hi[0]).map(move |x| y as usize * self.grid[0] as usize + x as usize)
            })
            .collect())
    }
}

pub struct Run<V = Vertex> {
    pub page: usize,
    pub geometry: gui::Run<V>,
}
#[derive(Clone, Copy)]
struct ClipVertex {
    pos: [f64; 3],
    uv: [f64; 2],
    light: [f64; 2],
    color: [f64; 4],
    extra: [f64; 8],
}
impl ClipVertex {
    fn new<V: AtlasVertex>(vertex: &V, size: [u32; 2]) -> Self {
        let v = vertex.base();
        Self {
            extra: vertex.extra().map(f64::from),
            pos: v.pos.map(f64::from),
            uv: std::array::from_fn(|i| f64::from(v.uv[i]) * f64::from(size[i])),
            light: v.light_uv.map(f64::from),
            color: v.color.map(f64::from),
        }
    }
    fn mix(self, other: Self, t: f64) -> Self {
        Self {
            extra: std::array::from_fn(|i| self.extra[i] + (other.extra[i] - self.extra[i]) * t),
            pos: std::array::from_fn(|i| self.pos[i] + (other.pos[i] - self.pos[i]) * t),
            uv: std::array::from_fn(|i| self.uv[i] + (other.uv[i] - self.uv[i]) * t),
            light: std::array::from_fn(|i| self.light[i] + (other.light[i] - self.light[i]) * t),
            color: std::array::from_fn(|i| self.color[i] + (other.color[i] - self.color[i]) * t),
        }
    }
    fn vertex<V: AtlasVertex>(self, origin: [u32; 2], side: u32) -> V {
        V::from_parts(
            Vertex {
                pos: self.pos.map(|v| v as f32),
                color: self.color.map(|v| v as f32),
                light_uv: self.light.map(|v| v as f32),
                uv: std::array::from_fn(|i| {
                    ((self.uv[i] - f64::from(origin[i]) + 1.) / f64::from(side)) as f32
                }),
            },
            self.extra.map(|v| v as f32),
        )
    }
}
fn clip(poly: Vec<ClipVertex>, axis: usize, edge: f64, below: bool) -> Vec<ClipVertex> {
    let mut output = Vec::new();
    let inside = |v: ClipVertex| {
        if below {
            v.uv[axis] <= edge
        } else {
            v.uv[axis] >= edge
        }
    };
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (ia, ib) = (inside(a), inside(b));
        if ia {
            output.push(a);
        }
        if ia != ib {
            let mut v = a.mix(b, (edge - a.uv[axis]) / (b.uv[axis] - a.uv[axis]));
            v.uv[axis] = edge;
            output.push(v);
        }
    }
    output
}
/// Clip in texture-pixel space using f64, interpolating position/depth/colour as
/// well as UVs. Never reorder triangles by page, including alpha-blended quads.
trait AtlasVertex: Copy {
    const ENTITY: bool;
    fn base(&self) -> &Vertex;
    fn extra(&self) -> [f32; 8];
    fn from_parts(base: Vertex, extra: [f32; 8]) -> Self;
}
impl AtlasVertex for Vertex {
    const ENTITY: bool = false;
    fn base(&self) -> &Vertex {
        self
    }
    fn extra(&self) -> [f32; 8] {
        [0.; 8]
    }
    fn from_parts(base: Vertex, _: [f32; 8]) -> Self {
        base
    }
}
impl AtlasVertex for crate::entity::LitVertex {
    const ENTITY: bool = true;
    fn base(&self) -> &Vertex {
        &self.base
    }
    fn extra(&self) -> [f32; 8] {
        std::array::from_fn(|i| {
            if i < 4 {
                self.back_color[i]
            } else {
                self.light_color[i - 4]
            }
        })
    }
    fn from_parts(base: Vertex, extra: [f32; 8]) -> Self {
        Self {
            base,
            back_color: std::array::from_fn(|i| extra[i]),
            light_color: std::array::from_fn(|i| extra[i + 4]),
        }
    }
}
pub fn split(layout: Layout, source: &[gui::Run], max_vertices: usize) -> Result<Vec<Run>, Error> {
    split_vertices(layout, source, max_vertices)
}
pub fn split_entities(
    layout: Layout,
    source: &[gui::Run<crate::entity::LitVertex>],
    max_vertices: usize,
) -> Result<Vec<Run<crate::entity::LitVertex>>, Error> {
    split_vertices(layout, source, max_vertices)
}
fn split_vertices<V: AtlasVertex>(
    layout: Layout,
    source: &[gui::Run<V>],
    max_vertices: usize,
) -> Result<Vec<Run<V>>, Error> {
    let mut output: Vec<Run<V>> = Vec::new();
    let mut total = 0usize;
    let mut visited = 0usize;
    for run in source {
        if (if V::ENTITY {
            !matches!(run.pass, crate::Pass::Entity(_))
        } else {
            !run.pass.is_overlay()
        }) || (run.clear_depth && run.pass != crate::Pass::OverlayDepth)
            || run.indices.len() % 3 != 0
            || run
                .indices
                .iter()
                .any(|&i| i as usize >= run.vertices.len())
            || run.vertices.iter().any(|vertex| {
                let v = vertex.base();
                vertex.extra().iter().any(|x| !x.is_finite())
                    || v.pos
                        .iter()
                        .chain(&v.uv)
                        .chain(&v.color)
                        .chain(&v.light_uv)
                        .any(|x| !x.is_finite())
                    || v.uv.iter().any(|x| !(0.0..=1.0).contains(x))
            })
        {
            return Err(Error::InvalidMesh);
        }
        let mut first = true;
        for tri in run.indices.chunks_exact(3) {
            let tri: [ClipVertex; 3] = std::array::from_fn(|i| {
                ClipVertex::new(&run.vertices[tri[i] as usize], layout.size)
            });
            let mut lo = [0; 2];
            let mut hi = [0; 2];
            for axis in 0..2 {
                let min = tri.iter().map(|v| v.uv[axis]).fold(f64::INFINITY, f64::min);
                let max = tri
                    .iter()
                    .map(|v| v.uv[axis])
                    .fold(f64::NEG_INFINITY, f64::max);
                lo[axis] =
                    ((min / f64::from(layout.span)).floor() as u32).min(layout.grid[axis] - 1);
                hi[axis] =
                    ((max / f64::from(layout.span)).floor() as u32).min(layout.grid[axis] - 1);
            }
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    visited = visited
                        .checked_add(1)
                        .filter(|&n| n <= max_vertices.saturating_add(1024))
                        .ok_or(Error::VertexBudget)?;
                    let page = y as usize * layout.grid[0] as usize + x as usize;
                    let origin = layout.origin(page)?;
                    let mut polygon = tri.to_vec();
                    for (axis, &start) in origin.iter().enumerate() {
                        polygon = clip(polygon, axis, f64::from(start), false);
                        polygon = clip(
                            polygon,
                            axis,
                            (u64::from(start) + u64::from(layout.span))
                                .min(u64::from(layout.size[axis]))
                                as f64,
                            true,
                        );
                    }
                    for n in 1..polygon.len().saturating_sub(1) {
                        let triangle = [polygon[0], polygon[n], polygon[n + 1]];
                        let [a, b, c] = triangle.map(|v| v.pos);
                        let ab: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
                        let ac: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
                        let area = [
                            ab[0] * ac[1] - ab[1] * ac[0],
                            ab[1] * ac[2] - ab[2] * ac[1],
                            ab[2] * ac[0] - ab[0] * ac[2],
                        ];
                        if if V::ENTITY {
                            area.iter().all(|&v| v == 0.)
                        } else {
                            area[0] == 0.
                        } {
                            continue;
                        }
                        total = total
                            .checked_add(3)
                            .filter(|&n| n <= max_vertices)
                            .ok_or(Error::VertexBudget)?;
                        let clear_depth = first && run.clear_depth;
                        if clear_depth
                            || output.last().is_none_or(|last| {
                                last.page != page
                                    || last.geometry.pass != run.pass
                                    || last.geometry.scissor != run.scissor
                                    || last.geometry.vertices.len() > 65532
                            })
                        {
                            output.push(Run {
                                page,
                                geometry: gui::Run {
                                    pass: run.pass,
                                    scissor: run.scissor,
                                    clear_depth,
                                    vertices: Vec::new(),
                                    indices: Vec::new(),
                                },
                            });
                        }
                        let last = &mut output.last_mut().unwrap().geometry;
                        let base = last.vertices.len() as u16;
                        last.vertices
                            .extend(triangle.map(|v| v.vertex::<V>(origin, layout.side)));
                        last.indices.extend([base, base + 1, base + 2]);
                        first = false;
                    }
                }
            }
        }
    }
    Ok(output)
}

struct Resident {
    page: usize,
    texture: TextureId,
    dirty: bool,
}
/// Resource state only; release it with its backend before dropping/reloading.
/// The backend still owns all IDs, and reclaims them on backend destruction.
pub struct Cache {
    layout: Layout,
    capacity: usize,
    resident: VecDeque<Resident>,
    retired_meshes: Vec<crate::MeshId>,
}
#[derive(Debug)]
pub enum RenderError<E, L> {
    InvalidPage,
    InvalidPixels,
    Backend(E),
    Load(L),
    Gui(gui::RenderError<E>),
}
pub struct WorldView {
    pub view_projection: [f32; 16],
    pub camera: [f64; 3],
    pub fog: Option<crate::Fog>,
    pub lightmap: TextureId,
}
impl Cache {
    pub fn new(layout: Layout, capacity: usize) -> Result<Self, Error> {
        if capacity == 0 {
            return Err(Error::InvalidLayout);
        }
        Ok(Self {
            layout,
            capacity,
            resident: VecDeque::new(),
            retired_meshes: Vec::new(),
        })
    }
    pub fn invalidate(&mut self, pages: &[usize]) {
        for r in &mut self.resident {
            if pages.contains(&r.page) {
                r.dirty = true;
            }
        }
    }
    pub fn collect_garbage<B: RenderBackend>(&mut self, backend: &mut B) -> Result<(), B::Error> {
        while let Some(&id) = self.retired_meshes.last() {
            backend.remove_mesh(id)?;
            self.retired_meshes.pop();
        }
        Ok(())
    }
    pub fn release<B: RenderBackend>(&mut self, backend: &mut B) -> Result<(), B::Error> {
        self.collect_garbage(backend)?;
        while let Some(r) = self.resident.front() {
            backend.remove_texture(r.texture)?;
            self.resident.pop_front();
        }
        Ok(())
    }
    pub fn render<B: RenderBackend, L>(
        &mut self,
        backend: &mut B,
        runs: &[Run],
        white: TextureId,
        mut load: impl FnMut(usize) -> Result<Vec<u8>, L>,
    ) -> Result<(), RenderError<B::Error, L>> {
        if runs.iter().any(|r| r.page >= self.layout.page_count()) {
            return Err(RenderError::InvalidPage);
        }
        for run in runs {
            let texture = self.texture(backend, run.page, &mut load)?;
            gui::render(backend, std::slice::from_ref(&run.geometry), texture, white)
                .map_err(RenderError::Gui)?;
        }
        Ok(())
    }
    fn texture<B: RenderBackend, L>(
        &mut self,
        backend: &mut B,
        page: usize,
        load: &mut impl FnMut(usize) -> Result<Vec<u8>, L>,
    ) -> Result<TextureId, RenderError<B::Error, L>> {
        Ok(
            if let Some(at) = self
                .resident
                .iter()
                .position(|r| r.page == page && !r.dirty)
            {
                let r = self.resident.remove(at).unwrap();
                let id = r.texture;
                self.resident.push_back(r);
                id
            } else {
                let pixels = load(page).map_err(RenderError::Load)?;
                if pixels.len() != self.layout.page_bytes() {
                    return Err(RenderError::InvalidPixels);
                }
                if let Some(at) = self.resident.iter().position(|r| r.page == page) {
                    backend
                        .remove_texture(self.resident[at].texture)
                        .map_err(RenderError::Backend)?;
                    self.resident.remove(at);
                }
                if self.resident.len() >= self.capacity {
                    backend
                        .remove_texture(self.resident.front().unwrap().texture)
                        .map_err(RenderError::Backend)?;
                    self.resident.pop_front();
                }
                let id = backend
                    .create_texture(Image {
                        width: self.layout.side,
                        height: self.layout.side,
                        rgba: &pixels,
                    })
                    .map_err(RenderError::Backend)?;
                self.resident.push_back(Resident {
                    page,
                    texture: id,
                    dirty: false,
                });
                id
            },
        )
    }
    /// Camera-relative entity positions: origin equals camera, so subtraction is
    /// exactly zero even at the world border. No clear or full-frame rollback.
    pub fn render_world<B: RenderBackend, L>(
        &mut self,
        backend: &mut B,
        runs: &[Run<crate::entity::LitVertex>],
        view: &WorldView,
        mut load: impl FnMut(usize) -> Result<Vec<u8>, L>,
    ) -> Result<(), RenderError<B::Error, L>> {
        if runs.iter().any(|r| {
            r.page >= self.layout.page_count()
                || !matches!(r.geometry.pass, crate::Pass::Entity(_))
                || r.geometry.clear_depth
        }) {
            return Err(RenderError::InvalidPage);
        }
        self.collect_garbage(backend)
            .map_err(RenderError::Backend)?;
        for run in runs {
            let texture = if matches!(run.geometry.pass, crate::Pass::Entity(m) if m.kind == crate::entity::Kind::WaterMask)
            {
                view.lightmap
            } else {
                self.texture(backend, run.page, &mut load)?
            };
            let mesh = backend
                .create_entity_mesh(&run.geometry.vertices, &run.geometry.indices)
                .map_err(RenderError::Backend)?;
            self.retired_meshes.push(mesh);
            let draw = crate::Draw {
                mesh,
                texture,
                lightmap: view.lightmap,
                origin: view.camera,
                pass: run.geometry.pass,
                scissor: run.geometry.scissor,
            };
            let result = backend.render(&crate::Frame {
                view_projection: view.view_projection,
                camera: view.camera,
                clear: crate::Clear::Load,
                fog: view.fog,
                draws: &[draw],
            });
            let cleanup = self.collect_garbage(backend);
            result.map_err(RenderError::Backend)?;
            cleanup.map_err(RenderError::Backend)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Pass,
        gui::{Blend, Input},
    };
    fn geometry() -> Vec<gui::Run> {
        let p = [[0., 0.], [60., 0.], [60., 60.], [0., 60.]];
        let uv = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
        let input = [0, 1, 2, 0, 2, 3].map(|i| Input {
            pos: p[i],
            uv: uv[i],
            color: 0x80402010,
            depth: 0.5,
        });
        gui::runs(
            &input,
            &[(0..6, Blend::Depth([0, 0, 60, 60]))],
            [60; 2],
            1.,
            [1.; 2],
            6,
        )
        .unwrap()
    }
    #[test]
    fn pages_include_neighbour_gutters_and_replicated_image_edges() {
        let l = Layout::new([3, 1], 4, 2).unwrap();
        let pixels = [1, 1, 1, 255, 2, 2, 2, 255, 3, 3, 3, 255];
        let a = l.pixels(0, &pixels).unwrap();
        let b = l.pixels(1, &pixels).unwrap();
        assert_eq!(
            &a[..16],
            &[1, 1, 1, 255, 1, 1, 1, 255, 2, 2, 2, 255, 3, 3, 3, 255]
        );
        assert_eq!(
            &b[..16],
            &[2, 2, 2, 255, 3, 3, 3, 255, 3, 3, 3, 255, 3, 3, 3, 255]
        );
        assert_eq!(&a[..16], &a[48..]);
        assert_eq!(l.affected([1, 0], [1, 1]).unwrap(), [0, 1]);
        assert_eq!(l.affected([2, 0], [1, 1]).unwrap(), [0, 1]);
        assert!(l.pixels(2, &pixels).is_err());
        assert!(Layout::new([3, 1], 4, 1).is_err());
    }
    #[test]
    fn clipping_preserves_order_area_alpha_and_a_single_depth_clear() {
        let l = Layout::new([6, 2], 4, 3).unwrap();
        let result = split(l, &geometry(), 100).unwrap();
        assert_eq!(
            result.iter().map(|r| r.page).collect::<Vec<_>>(),
            [0, 1, 2, 0, 1, 2]
        );
        assert_eq!(result.iter().filter(|r| r.geometry.clear_depth).count(), 1);
        assert!(result[0].geometry.clear_depth);
        let mut area = 0.;
        for run in &result {
            assert_eq!(run.geometry.pass, Pass::OverlayDepth);
            for tri in run.geometry.vertices.chunks_exact(3) {
                let [a, b, c] = [tri[0].pos, tri[1].pos, tri[2].pos];
                area += f64::from(
                    ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs(),
                ) * 0.5;
                assert!(tri.iter().all(|v| v.color[3] == 128. / 255.));
            }
        }
        assert!((area - 4.).abs() < 1e-6, "area {area}");
        assert!(matches!(split(l, &geometry(), 5), Err(Error::VertexBudget)));
    }
    #[test]
    fn clipping_interpolates_colour_depth_and_texture_coordinates() {
        let l = Layout::new([4, 1], 4, 2).unwrap();
        let vertices = [
            Input {
                pos: [0., 0.],
                uv: [0., 0.5],
                color: 0xff000000,
                depth: 0.,
            },
            Input {
                pos: [60., 0.],
                uv: [1., 0.5],
                color: 0xffffffff,
                depth: 1.,
            },
            Input {
                pos: [0., 60.],
                uv: [0., 0.5],
                color: 0xff000000,
                depth: 0.,
            },
        ];
        let source =
            gui::runs(&vertices, &[(0..3, Blend::Alpha)], [60; 2], 1., [1.; 2], 3).unwrap();
        let result = split(l, &source, 30).unwrap();
        let boundary = result[0]
            .geometry
            .vertices
            .iter()
            .find(|v| v.pos[0] == 0.)
            .unwrap();
        assert_eq!(boundary.pos[2], 0.);
        assert_eq!(boundary.color, [0.5, 0.5, 0.5, 1.]);
        assert_eq!(boundary.uv[0], 0.75);
        let boundary = result[1]
            .geometry
            .vertices
            .iter()
            .find(|v| v.pos[0] == 0.)
            .unwrap();
        assert_eq!(boundary.uv[0], 0.25);
    }
}
