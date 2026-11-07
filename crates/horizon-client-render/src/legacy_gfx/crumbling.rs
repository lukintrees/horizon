//! Destroy-stage decals from actual world entity quads (overlays 11 and 12).
//! Extract stages, not repeating atlas UVs: wrapping must follow interpolation.
use super::screen::{self, Sprite};
use crate::{mesher::Vertex as Source, paged_hud::Backing};
use horizon_render_api::{
    Clear, Draw, Frame, Image, Pass, RenderBackend, Vertex,
    entity::{self, Kind, LitVertex, Material, Overlay},
    paged_gui::WorldView,
};
use std::{borrow::Cow, collections::HashMap};
struct Run {
    sprite: usize,
    vertices: [LitVertex; 4],
    translucent: bool,
}
pub(super) struct Prepared<'a> {
    pub ordinary: Cow<'a, [Source]>,
    sprites: Vec<Sprite>,
    runs: Vec<Run>,
}
impl<'a> Prepared<'a> {
    pub fn new(
        vertices: &'a [Source],
        extent: Option<[f32; 4]>,
        backing: &Backing,
        nether: bool,
        quad_limit: usize,
        byte_limit: usize,
        max_texture: u32,
    ) -> Result<Self, String> {
        if !vertices.len().is_multiple_of(4) {
            return Err("incomplete crumbling/entity quad".into());
        }
        let is_crack = |v: &Source| matches!(v.layer & 0xffff, 11 | 12);
        let count = vertices
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|q| q.iter().any(is_crack))
            .count();
        if count > quad_limit {
            return Err("crumbling quad budget exceeded".into());
        }
        let mut result = Self {
            ordinary: Cow::Borrowed(vertices),
            sprites: Vec::new(),
            runs: Vec::new(),
        };
        if count == 0 {
            return Ok(result);
        }
        let extent = extent.ok_or("crumbling requires actual Frame sprite extent")?;
        if extent[2..]
            .iter()
            .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
        {
            return Err("invalid crumbling sprite extent".into());
        }
        let size = backing.layout().size();
        let dimensions: [u32; 2] =
            std::array::from_fn(|i| (extent[i + 2] * size[i] as f32).round().max(1.) as u32);
        if dimensions.iter().any(|&v| v > max_texture) {
            return Err("crumbling sprite exceeds native texture limit".into());
        }
        let mut ordinary = Vec::new();
        ordinary
            .try_reserve_exact(vertices.len() - count * 4)
            .map_err(|_| "crumbling allocation failed")?;
        result
            .runs
            .try_reserve_exact(count)
            .map_err(|_| "crumbling allocation failed")?;
        result
            .sprites
            .try_reserve_exact(count)
            .map_err(|_| "crumbling allocation failed")?;
        let mut indices = HashMap::new();
        indices
            .try_reserve(count)
            .map_err(|_| "crumbling allocation failed")?;
        let mut bytes = backing.layout().page_bytes();
        for quad in vertices.as_chunks::<4>().0 {
            if !quad.iter().any(is_crack) {
                ordinary.extend_from_slice(quad);
                continue;
            }
            let first = quad[0];
            if first.layer & !0x01ff_ffff != 0
                || quad.iter().any(|v| {
                    v.layer != first.layer
                        || v.light != first.light
                        || v.pos.iter().chain(&v.uv).any(|v| !v.is_finite())
                })
            {
                return Err("invalid/mixed crumbling quad or stage".into());
            }
            let origin: [u32; 2] = std::array::from_fn(|i| {
                let packed = (first.light >> (16 * i)) & 65535;
                (packed as f32 / 65535. * size[i] as f32).round() as u32
            });
            let rect = [origin[0], origin[1], dimensions[0], dimensions[1]];
            if (0..2).any(|i| {
                origin[i]
                    .checked_add(dimensions[i])
                    .is_none_or(|end| end > size[i])
            }) {
                return Err("crumbling stage outside atlas".into());
            }
            let sprite = if let Some(&index) = indices.get(&rect) {
                index
            } else {
                let len = (dimensions[0] as usize)
                    .checked_mul(dimensions[1] as usize)
                    .and_then(|n| n.checked_mul(4))
                    .ok_or("crumbling size overflow")?;
                bytes = bytes
                    .checked_add(len)
                    .filter(|&n| n <= byte_limit)
                    .ok_or("crumbling staging byte budget exceeded")?;
                let pixels = backing.extract(rect, byte_limit)?;
                let index = result.sprites.len();
                result.sprites.push(Sprite {
                    size: dimensions,
                    pixels,
                });
                indices.insert(rect, index);
                index
            };
            let vertices = quad.map(|v| {
                let mut color = v.color.to_le_bytes().map(|v| f32::from(v) / 255.);
                if (f32::from(v.normal[3]) / 127.).abs() >= 0.5 {
                    let shade = entity::shade(
                        std::array::from_fn(|i| (f32::from(v.normal[i]) / 127.).max(-1.)),
                        nether,
                    );
                    for c in &mut color[..3] {
                        *c *= shade;
                    }
                }
                LitVertex {
                    base: Vertex {
                        pos: v.pos,
                        uv: v.uv,
                        color,
                        light_uv: [0.; 2],
                    },
                    back_color: color,
                    light_color: [1.; 4],
                    glint_uv: [0.; 2],
                }
            });
            result.runs.push(Run {
                sprite,
                vertices,
                translucent: first.layer & 0xffff == 12,
            });
        }
        result.ordinary = Cow::Owned(ordinary);
        Ok(result)
    }
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
    pub fn vertex_count(&self) -> usize {
        self.runs.len() * 4
    }
    pub fn largest_texture(&self) -> usize {
        self.sprites
            .iter()
            .map(|s| s.pixels.len())
            .max()
            .unwrap_or(0)
    }
}
impl screen::Resources {
    pub fn render_crumbling<B: RenderBackend<Error = String>>(
        &mut self,
        backend: &mut B,
        prepared: &Prepared<'_>,
        view: &WorldView,
        translucent: bool,
    ) -> Result<(), String> {
        self.collect(backend)?;
        let mut active = None;
        for run in prepared
            .runs
            .iter()
            .filter(|r| r.translucent == translucent)
        {
            let texture = match active {
                Some((index, texture)) if index == run.sprite => texture,
                _ => {
                    self.collect(backend)?;
                    let sprite = &prepared.sprites[run.sprite];
                    let texture = backend.create_texture(Image {
                        width: sprite.size[0],
                        height: sprite.size[1],
                        rgba: &sprite.pixels,
                    })?;
                    self.textures.push(texture);
                    active = Some((run.sprite, texture));
                    texture
                }
            };
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
                        cull: true,
                        kind: Kind::Crumbling,
                        glint: None,
                    }),
                    scissor: None,
                }],
            })?;
            while let Some(&id) = self.meshes.last() {
                backend.remove_mesh(id)?;
                self.meshes.pop();
            }
        }
        self.collect(backend)
    }
}
#[cfg(test)]
mod tests;
