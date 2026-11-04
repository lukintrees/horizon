//! PR 6: section draws, pass ordering and `glMultiDrawElementsEXT` grouping.
//!
//! The real extension call is left to the GL layer; this module produces the
//! exact `counts`/byte `offsets` arrays a GL2 driver would receive. Opaque and
//! cutout sections are drawn far-to-near, translucent sections are sorted and
//! blended last, and HUD/line overlays stay outside the multi-draw groups.

use std::error::Error;
use std::fmt;

use super::atlas::{Atlas, AtlasError, MAX_SAMPLERS};
use super::mesh::{GlVertex, MeshError, OPAQUE_LAYER, UV_STEPS, pack_color};
use super::sections::{Blend, OverlayRect, Scene, SectionKind};

/// Acceptance budget for the X1650 MVP.
pub const MAX_DRAWS: u32 = 200;
pub const MAX_INDEXED_VERTICES: usize = u16::MAX as usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PassKind {
    Opaque,
    Cutout,
    Translucent,
}

const PASS_KINDS: [SectionKind; 3] = [
    SectionKind::Solid,
    SectionKind::Cutout,
    SectionKind::Translucent,
];
const PASS_ORDER: [PassKind; 3] = [PassKind::Opaque, PassKind::Cutout, PassKind::Translucent];

const fn pass_for(kind: SectionKind) -> PassKind {
    match kind {
        SectionKind::Solid => PassKind::Opaque,
        SectionKind::Cutout => PassKind::Cutout,
        SectionKind::Translucent => PassKind::Translucent,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawCommand {
    pub pass: PassKind,
    pub blend: Blend,
    pub first_index: u32,
    pub index_count: u32,
    pub base_vertex: u32,
    pub layer: u16,
    pub distance: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiDraw {
    pub pass: PassKind,
    pub blend: Blend,
    pub counts: Vec<i32>,
    pub offsets: Vec<u32>,
    pub commands: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterQuad {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub color: [u8; 4],
    pub fog: u8,
    pub blend: Blend,
    pub checker: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FramePlan {
    pub atlas: Atlas,
    pub vertices: Vec<GlVertex>,
    pub indices: Vec<u16>,
    pub draws: Vec<DrawCommand>,
    pub multi_draws: Vec<MultiDraw>,
    pub raster_quads: Vec<RasterQuad>,
    pub hud: Vec<OverlayRect>,
    pub lines: Vec<OverlayRect>,
    pub sky_top: [u8; 3],
    pub sky_bottom: [u8; 3],
    pub fog_color: [u8; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub enum FrameError {
    Atlas(AtlasError),
    Mesh(MeshError),
    BadQuad { index: usize, vertices: usize },
    TooManyVertices,
    TooManyIndices,
    TooManyDraws { count: u32, limit: u32 },
    TooManySamplers { count: u32, limit: u32 },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Atlas(err) => write!(f, "atlas: {err}"),
            Self::Mesh(err) => write!(f, "mesh: {err}"),
            Self::BadQuad { index, vertices } => {
                write!(
                    f,
                    "section {index} has {vertices} vertices, not a multiple of four"
                )
            }
            Self::TooManyVertices => write!(f, "vertex buffer exceeds GL_UNSIGNED_SHORT"),
            Self::TooManyIndices => write!(f, "index buffer exceeds the frame budget"),
            Self::TooManyDraws { count, limit } => {
                write!(f, "{count} draw calls exceed the {limit} budget")
            }
            Self::TooManySamplers { count, limit } => {
                write!(f, "{count} samplers exceed the {limit} budget")
            }
        }
    }
}

impl Error for FrameError {}

impl From<AtlasError> for FrameError {
    fn from(err: AtlasError) -> Self {
        Self::Atlas(err)
    }
}

impl From<MeshError> for FrameError {
    fn from(err: MeshError) -> Self {
        Self::Mesh(err)
    }
}

impl FramePlan {
    pub fn build(scene: &Scene) -> Result<Self, FrameError> {
        let atlas = Atlas::new(4, 16, 4)?;
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut draws = Vec::new();
        let mut raster_quads = Vec::new();

        for &kind in &PASS_KINDS {
            let mut group: Vec<usize> = (0..scene.sections.len())
                .filter(|&i| scene.sections[i].kind == kind)
                .collect();
            group.sort_by(|&a, &b| {
                scene.sections[b]
                    .distance
                    .cmp(&scene.sections[a].distance)
                    .then(a.cmp(&b))
            });
            for index in group {
                let section = &scene.sections[index];
                let source = section.vertices();
                if source.is_empty() || !source.len().is_multiple_of(4) {
                    return Err(FrameError::BadQuad {
                        index,
                        vertices: source.len(),
                    });
                }
                let base_vertex = vertices.len() as u32;
                if base_vertex as usize + source.len() > MAX_INDEXED_VERTICES {
                    return Err(FrameError::TooManyVertices);
                }
                for vertex in source {
                    vertices.push(GlVertex::from_terrain(vertex, &atlas, UV_STEPS)?);
                }

                let first_index = indices.len() as u32;
                let mut local = 0_u16;
                while (local as usize) < source.len() {
                    let base = base_vertex as u16 + local;
                    indices.extend_from_slice(&[
                        base,
                        base + 1,
                        base + 2,
                        base,
                        base + 2,
                        base + 3,
                    ]);
                    local += 4;
                }
                draws.push(DrawCommand {
                    pass: pass_for(kind),
                    blend: section.blend(),
                    first_index,
                    index_count: indices.len() as u32 - first_index,
                    base_vertex,
                    layer: source[0].layer & !OPAQUE_LAYER,
                    distance: section.distance,
                });

                let [x0, y0, x1, y1] = section.screen_rect();
                let fog = section
                    .distance
                    .saturating_mul(255)
                    .checked_div(scene.fog_scale.max(1))
                    .unwrap_or(255)
                    .min(255) as u8;
                raster_quads.push(RasterQuad {
                    x0,
                    y0,
                    x1,
                    y1,
                    color: pack_color(source[0].color),
                    fog,
                    blend: section.blend(),
                    checker: section.checker,
                });
            }
        }

        let mut multi_draws = Vec::new();
        for &pass in &PASS_ORDER {
            let group: Vec<&DrawCommand> = draws.iter().filter(|d| d.pass == pass).collect();
            if group.is_empty() {
                continue;
            }
            multi_draws.push(MultiDraw {
                pass,
                blend: group[0].blend,
                counts: group.iter().map(|d| d.index_count as i32).collect(),
                offsets: group.iter().map(|d| d.first_index * 2).collect(),
                commands: group.len(),
            });
        }

        let plan = Self {
            atlas,
            vertices,
            indices,
            draws,
            multi_draws,
            raster_quads,
            hud: scene.hud.clone(),
            lines: scene.lines.clone(),
            sky_top: scene.sky_top,
            sky_bottom: scene.sky_bottom,
            fog_color: scene.fog_color,
        };
        plan.validate()?;
        Ok(plan)
    }

    /// One `glMultiDrawElementsEXT` per pass plus one overlay draw each.
    pub fn draw_call_count(&self) -> u32 {
        self.multi_draws.len() as u32 + self.hud.len() as u32 + self.lines.len() as u32
    }

    pub fn validate(&self) -> Result<(), FrameError> {
        if self.vertices.len() > MAX_INDEXED_VERTICES {
            return Err(FrameError::TooManyVertices);
        }
        if self.indices.len() > MAX_INDEXED_VERTICES {
            return Err(FrameError::TooManyIndices);
        }
        let samplers = self.atlas.sampler_count();
        if samplers > MAX_SAMPLERS {
            return Err(FrameError::TooManySamplers {
                count: samplers,
                limit: MAX_SAMPLERS,
            });
        }
        let draws = self.draw_call_count();
        if draws > MAX_DRAWS {
            return Err(FrameError::TooManyDraws {
                count: draws,
                limit: MAX_DRAWS,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_frame_groups_three_passes_into_three_multi_draws() {
        let plan = FramePlan::build(&Scene::reference()).unwrap();
        assert_eq!(plan.draws.len(), 3);
        assert_eq!(plan.multi_draws.len(), 3);
        assert_eq!(plan.vertices.len(), 12);
        assert_eq!(plan.indices.len(), 18);
        assert_eq!(plan.draw_call_count(), 5);
        assert!(plan.draw_call_count() <= MAX_DRAWS);
        assert_eq!(plan.validate(), Ok(()));
        assert_eq!(plan.atlas.sampler_count(), 4);
    }

    #[test]
    fn multi_draw_offsets_are_byte_offsets_into_the_index_buffer() {
        let plan = FramePlan::build(&Scene::reference()).unwrap();
        assert_eq!(plan.draws[0].first_index, 0);
        assert_eq!(plan.draws[1].first_index, 6);
        assert_eq!(plan.draws[2].first_index, 12);
        assert_eq!(plan.multi_draws[0].counts, vec![6]);
        assert_eq!(plan.multi_draws[0].offsets, vec![0]);
        assert_eq!(plan.multi_draws[1].offsets, vec![12]);
        assert_eq!(plan.multi_draws[2].offsets, vec![24]);
        assert_eq!(plan.multi_draws[0].pass, PassKind::Opaque);
        assert_eq!(plan.multi_draws[1].pass, PassKind::Cutout);
        assert_eq!(plan.multi_draws[2].pass, PassKind::Translucent);
    }

    #[test]
    fn raster_quads_keep_independent_rects_fog_and_blends() {
        let plan = FramePlan::build(&Scene::reference()).unwrap();
        assert_eq!(plan.raster_quads.len(), 3);
        assert_eq!(
            plan.raster_quads[0],
            RasterQuad {
                x0: 8,
                y0: 20,
                x1: 32,
                y1: 44,
                color: [120, 90, 60, 255],
                fog: 51,
                blend: Blend::Opaque,
                checker: false,
            }
        );
        assert_eq!(plan.raster_quads[1].fog, 76);
        assert!(plan.raster_quads[1].checker);
        assert_eq!(plan.raster_quads[1].blend, Blend::Cutout);
        assert_eq!(plan.raster_quads[2].fog, 102);
        assert_eq!(plan.raster_quads[2].blend, Blend::Alpha);
        assert_eq!(plan.raster_quads[2].color, [40, 90, 160, 128]);
    }

    #[test]
    fn malformed_quad_is_rejected() {
        let mut scene = Scene::reference();
        scene.sections[0].mesh.solid.truncate(3);
        assert_eq!(
            FramePlan::build(&scene),
            Err(FrameError::BadQuad {
                index: 0,
                vertices: 3
            })
        );
    }
}
