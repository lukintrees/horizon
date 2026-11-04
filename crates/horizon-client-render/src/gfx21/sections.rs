//! PR 6: section meshes, draw kinds and the deterministic reference scene.
//!
//! Sections keep the CPU-side `TerrainVertex` quads; `passes.rs` turns them
//! into GL2 vertices, indices and `glMultiDrawElementsEXT` groups. The small
//! reference scene is also the input of the golden-frame CPU rasterizer.

use super::mesh::{TerrainVertex, UV_STEPS};

/// Orthographic pixels per world unit in the deterministic reference frame.
pub const PROJECTION_SCALE: i32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SectionKind {
    Solid,
    Cutout,
    Translucent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Blend {
    Opaque,
    Cutout,
    Alpha,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SectionMesh {
    pub solid: Vec<TerrainVertex>,
    pub cutout: Vec<TerrainVertex>,
    pub translucent: Vec<TerrainVertex>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub origin: [i32; 3],
    pub distance: u32,
    pub kind: SectionKind,
    pub checker: bool,
    pub mesh: SectionMesh,
}

impl Section {
    pub fn vertices(&self) -> &[TerrainVertex] {
        match self.kind {
            SectionKind::Solid => &self.mesh.solid,
            SectionKind::Cutout => &self.mesh.cutout,
            SectionKind::Translucent => &self.mesh.translucent,
        }
    }

    pub const fn blend(&self) -> Blend {
        match self.kind {
            SectionKind::Solid => Blend::Opaque,
            SectionKind::Cutout => Blend::Cutout,
            SectionKind::Translucent => Blend::Alpha,
        }
    }

    /// Screen-space rectangle `[x0, y0, x1, y1]` with an exclusive far edge.
    pub fn screen_rect(&self) -> [i32; 4] {
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for vertex in self.vertices() {
            let x = (self.origin[0] + vertex.pos[0] as i32) * PROJECTION_SCALE;
            let y = (self.origin[1] + vertex.pos[1] as i32) * PROJECTION_SCALE;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        [min_x, min_y, max_x, max_y]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub color: [u8; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub sections: Vec<Section>,
    pub hud: Vec<OverlayRect>,
    pub lines: Vec<OverlayRect>,
    pub sky_top: [u8; 3],
    pub sky_bottom: [u8; 3],
    pub fog_color: [u8; 3],
    pub fog_scale: u32,
}

/// `TerrainVertex.color` stores little-endian RGBA8 in a u32.
pub fn color_u32(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16) | (u32::from(a) << 24)
}

fn quad_vertices(layer: u16, color: u32, light: u16) -> [TerrainVertex; 4] {
    let pos = [
        [0.0, 0.0, 0.0],
        [6.0, 0.0, 0.0],
        [6.0, 6.0, 0.0],
        [0.0, 6.0, 0.0],
    ];
    let uv = [
        [0, 0],
        [UV_STEPS as u16, 0],
        [UV_STEPS as u16, UV_STEPS as u16],
        [0, UV_STEPS as u16],
    ];
    [
        TerrainVertex {
            pos: pos[0],
            uv: uv[0],
            layer,
            light,
            color,
        },
        TerrainVertex {
            pos: pos[1],
            uv: uv[1],
            layer,
            light,
            color,
        },
        TerrainVertex {
            pos: pos[2],
            uv: uv[2],
            layer,
            light,
            color,
        },
        TerrainVertex {
            pos: pos[3],
            uv: uv[3],
            layer,
            light,
            color,
        },
    ]
}

impl Scene {
    /// Three quads: opaque ground, cutout foliage and translucent water, plus
    /// one HUD rectangle and one line. All values are hard-coded so the Python
    /// golden-frame tool can reproduce the frame independently.
    pub fn reference() -> Self {
        let light = 0x0A_14;
        let brown = color_u32(120, 90, 60, 255);
        let tan = color_u32(180, 160, 120, 255);
        let blue = color_u32(40, 90, 160, 128);
        Self {
            sections: vec![
                Section {
                    origin: [2, 5, 0],
                    distance: 20,
                    kind: SectionKind::Solid,
                    checker: false,
                    mesh: SectionMesh {
                        solid: quad_vertices(0, brown, light).to_vec(),
                        ..SectionMesh::default()
                    },
                },
                Section {
                    origin: [5, 4, 0],
                    distance: 30,
                    kind: SectionKind::Cutout,
                    checker: true,
                    mesh: SectionMesh {
                        cutout: quad_vertices(1, tan, light).to_vec(),
                        ..SectionMesh::default()
                    },
                },
                Section {
                    origin: [10, 6, 0],
                    distance: 40,
                    kind: SectionKind::Translucent,
                    checker: false,
                    mesh: SectionMesh {
                        translucent: quad_vertices(2, blue, light).to_vec(),
                        ..SectionMesh::default()
                    },
                },
            ],
            hud: vec![OverlayRect {
                x0: 4,
                y0: 4,
                x1: 20,
                y1: 12,
                color: [220, 220, 220, 255],
            }],
            lines: vec![OverlayRect {
                x0: 4,
                y0: 56,
                x1: 76,
                y1: 57,
                color: [240, 200, 80, 255],
            }],
            sky_top: [24, 36, 56],
            sky_bottom: [72, 96, 120],
            fog_color: [90, 110, 130],
            fog_scale: 100,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_scene_has_three_sections_and_two_overlays() {
        let scene = Scene::reference();
        assert_eq!(scene.sections.len(), 3);
        assert_eq!(scene.hud.len(), 1);
        assert_eq!(scene.lines.len(), 1);
        assert_eq!(scene.sky_top, [24, 36, 56]);
        assert_eq!(scene.sky_bottom, [72, 96, 120]);
        assert_eq!(scene.fog_color, [90, 110, 130]);
        assert_eq!(scene.fog_scale, 100);
    }

    #[test]
    fn screen_rects_are_origin_plus_quad_times_scale() {
        let scene = Scene::reference();
        assert_eq!(scene.sections[0].screen_rect(), [8, 20, 32, 44]);
        assert_eq!(scene.sections[1].screen_rect(), [20, 16, 44, 40]);
        assert_eq!(scene.sections[2].screen_rect(), [40, 24, 64, 48]);
    }

    #[test]
    fn kinds_map_to_blend_modes_and_quad_counts() {
        let scene = Scene::reference();
        assert_eq!(scene.sections[0].blend(), Blend::Opaque);
        assert_eq!(scene.sections[1].blend(), Blend::Cutout);
        assert_eq!(scene.sections[2].blend(), Blend::Alpha);
        for section in &scene.sections {
            assert_eq!(section.vertices().len(), 4);
        }
        assert!(scene.sections[1].checker);
        assert!(!scene.sections[0].checker);
        assert!(!scene.sections[2].checker);
    }

    #[test]
    fn color_u32_is_little_endian_rgba8() {
        assert_eq!(color_u32(120, 90, 60, 255), 0xFF_3C_5A_78);
        assert_eq!(color_u32(40, 90, 160, 128), 0x80_A0_5A_28);
    }
}
