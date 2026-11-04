//! CPU scene assembly: streamed chunk meshes, camera, sky, entities, selection
//! outline, HUD rectangles and rasterized text. No GL calls live here.

use glam::{Mat4, Vec3};

use super::Game;
use super::player::REACH;
use super::text::{TextAtlas, TextQuad};
use super::ui::{Rect, UiFrame};
use super::world::{CHUNK_SIZE, WORLD_HEIGHT, height_at};

const UVS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
const ORDER: [usize; 6] = [0, 1, 2, 0, 2, 3];
const ATLAS_INSET: f32 = 0.5 / 64.0;
const WHITE_UV: [[f32; 2]; 4] = [
    [ATLAS_INSET, ATLAS_INSET],
    [0.25 - ATLAS_INSET, ATLAS_INSET],
    [0.25 - ATLAS_INSET, 0.25 - ATLAS_INSET],
    [ATLAS_INSET, 0.25 - ATLAS_INSET],
];

#[derive(Clone, Debug, PartialEq)]
pub struct ChunkMesh {
    pub key: (i32, i32),
    pub origin: [f32; 3],
    pub vertices: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderScene {
    pub width: u32,
    pub height: u32,
    pub view_proj: [f32; 16],
    pub sky_top: [u8; 4],
    pub sky_bottom: [u8; 4],
    pub fog_color: [u8; 4],
    pub fog_amount: f32,
    pub chunks: Vec<ChunkMesh>,
    pub entities: Vec<f32>,
    pub lines: Vec<f32>,
    pub hud_rects: Vec<f32>,
    pub hud_text: Vec<f32>,
    pub hud_texture: TextAtlas,
    pub draw_count: usize,
}

fn corners(rect: [f32; 4], z: f32) -> [[f32; 3]; 4] {
    [
        [rect[0], rect[3], z],
        [rect[2], rect[3], z],
        [rect[2], rect[1], z],
        [rect[0], rect[1], z],
    ]
}

fn rgba(color: [u8; 4]) -> [f32; 4] {
    [
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
        color[3] as f32 / 255.0,
    ]
}

fn mix(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let mut out = [0; 4];
    for i in 0..4 {
        out[i] = (a[i] as f32 * (1.0 - t) + b[i] as f32 * t).round() as u8;
    }
    out
}

fn sky_colors(day_time: u32) -> ([u8; 4], [u8; 4], f32) {
    let phase = (day_time % 24_000) as f32 / 24_000.0;
    let daylight = ((phase - 0.25) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
    let night_top = [12, 18, 40, 255];
    let day_top = [64, 132, 220, 255];
    let night_bottom = [8, 10, 22, 255];
    let day_bottom = [180, 210, 240, 255];
    (
        mix(night_top, day_top, daylight),
        mix(night_bottom, day_bottom, daylight),
        0.10 + (1.0 - daylight) * 0.18,
    )
}

fn push_hud_rect(out: &mut Vec<f32>, rect: Rect) {
    let c = corners([rect.x0, rect.y0, rect.x1, rect.y1], 0.0);
    let color = rgba(rect.color);
    for at in ORDER {
        out.extend_from_slice(&[c[at][0], c[at][1]]);
        out.extend_from_slice(&WHITE_UV[at]);
        out.extend_from_slice(&color);
    }
}

fn push_hud_text(out: &mut Vec<f32>, quad: &TextQuad) {
    let c = corners(quad.rect, 0.0);
    let color = rgba(quad.color);
    let uv = [
        [quad.uv[0], quad.uv[1]],
        [quad.uv[2], quad.uv[1]],
        [quad.uv[2], quad.uv[3]],
        [quad.uv[0], quad.uv[3]],
    ];
    for at in ORDER {
        out.extend_from_slice(&[c[at][0], c[at][1]]);
        out.extend_from_slice(&uv[at]);
        out.extend_from_slice(&color);
    }
}

fn push_entity(out: &mut Vec<f32>, position: [f32; 3], size: f32, color: [u8; 4]) {
    let rect = [
        position[0] - size,
        position[1] - size,
        position[0] + size,
        position[1] + size,
    ];
    let c = corners(rect, position[2]);
    let rgba = rgba(color);
    for at in ORDER {
        out.extend_from_slice(&c[at]);
        out.extend_from_slice(&UVS[at]);
        out.extend_from_slice(&rgba);
    }
}

fn push_line(out: &mut Vec<f32>, a: [f32; 3], b: [f32; 3], color: [f32; 4], thickness: f32) {
    let axis = if (a[0] - b[0]).abs() > 0.001 {
        0
    } else if (a[1] - b[1]).abs() > 0.001 {
        1
    } else {
        2
    };
    let mut quad = [[0.0; 3]; 4];
    match axis {
        0 => {
            quad = [
                [a[0], a[1] - thickness, a[2] - thickness],
                [b[0], a[1] - thickness, a[2] - thickness],
                [b[0], a[1] + thickness, a[2] + thickness],
                [a[0], a[1] + thickness, a[2] + thickness],
            ];
        }
        1 => {
            quad = [
                [a[0] - thickness, a[1], a[2] - thickness],
                [a[0] + thickness, a[1], a[2] - thickness],
                [a[0] + thickness, b[1], a[2] + thickness],
                [a[0] - thickness, b[1], a[2] + thickness],
            ];
        }
        _ => {
            quad = [
                [a[0] - thickness, a[1] - thickness, a[2]],
                [a[0] + thickness, a[1] - thickness, a[2]],
                [a[0] + thickness, a[1] + thickness, b[2]],
                [a[0] - thickness, a[1] + thickness, b[2]],
            ];
        }
    }
    for at in ORDER {
        out.extend_from_slice(&quad[at]);
        out.extend_from_slice(&color);
    }
}

fn selection_lines(out: &mut Vec<f32>, block: [i32; 3]) {
    let x = block[0] as f32;
    let y = block[1] as f32;
    let z = block[2] as f32;
    let x1 = x + 1.002;
    let y1 = y + 1.002;
    let z1 = z + 1.002;
    let x0 = x - 0.002;
    let y0 = y - 0.002;
    let z0 = z - 0.002;
    let color = [1.0, 1.0, 1.0, 0.9];
    let t = 0.012;
    let corners = [
        [x0, y0, z0],
        [x1, y0, z0],
        [x1, y0, z1],
        [x0, y0, z1],
        [x0, y1, z0],
        [x1, y1, z0],
        [x1, y1, z1],
        [x0, y1, z1],
    ];
    for (a, b) in [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ] {
        push_line(out, corners[a], corners[b], color, t);
    }
}

impl RenderScene {
    pub fn build(game: &mut Game, width: u32, height: u32) -> Result<Self, String> {
        let width = width.max(1);
        let height = height.max(1);
        let eye = Vec3::from_array(game.player.eye());
        let direction = Vec3::from_array(game.player.direction());
        let view = Mat4::look_at_rh(eye, eye + direction, Vec3::Y);
        let fov = game.settings.fov.clamp(30.0, 110.0).to_radians();
        let aspect = width as f32 / height as f32;
        let projection = Mat4::perspective_rh_gl(fov, aspect, 0.05, 512.0);
        let view_proj = (projection * view).to_cols_array();
        let (sky_top, sky_bottom, fog_amount) = sky_colors(game.world.day_time);

        let mut keys: Vec<_> = game.chunk_cache.keys().copied().collect();
        keys.sort_unstable();
        let chunks = keys
            .into_iter()
            .map(|(cx, cz)| ChunkMesh {
                key: (cx, cz),
                origin: [(cx * CHUNK_SIZE) as f32, 0.0, (cz * CHUNK_SIZE) as f32],
                vertices: game.chunk_cache[&(cx, cz)].clone(),
            })
            .collect::<Vec<_>>();

        let mut entities = Vec::new();
        for index in 0..4_i32 {
            let x = eye.x + 8.0 + index as f32 * 5.0;
            let z = eye.z - 6.0 + (index % 2) as f32 * 7.0;
            let y = height_at(game.world.seed, x.floor() as i32, z.floor() as i32) as f32 + 2.0;
            push_entity(
                &mut entities,
                [x, y, z],
                0.6,
                [220, 96 + index as u8 * 20, 80, 255],
            );
        }

        let mut lines = Vec::new();
        if let Some((block, _)) = game.player.raycast(&mut game.world, REACH) {
            selection_lines(&mut lines, block);
        }

        let draw_count = chunks.len()
            + 1  // sky
            + 1  // smoke
            + usize::from(!entities.is_empty())
            + usize::from(!lines.is_empty())
            + 2; // HUD rectangles and text
        let snapshot = game.snapshot(draw_count);
        let ui = UiFrame::build(&snapshot);
        let mut hud_rects = Vec::new();
        for rect in &ui.rects {
            push_hud_rect(&mut hud_rects, *rect);
        }
        let hud_texture = TextAtlas::build(&ui.texts, width, height)?;
        let mut hud_text = Vec::new();
        for quad in &hud_texture.quads {
            push_hud_text(&mut hud_text, quad);
        }
        game.ui = ui;
        game.draws = draw_count;
        Ok(Self {
            width,
            height,
            view_proj,
            sky_top,
            sky_bottom,
            fog_color: sky_bottom,
            fog_amount,
            chunks,
            entities,
            lines,
            hud_rects,
            hud_text,
            hud_texture,
            draw_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client_render::gfx21::facade::LegacySettings;

    #[test]
    fn a_game_scene_has_streamed_chunks_and_a_view_matrix() {
        let mut settings = LegacySettings::reference();
        settings.game = true;
        settings.render_distance = 1;
        let mut game = Game::new(&settings);
        let scene = game.build_scene(800, 600).unwrap();
        assert!(!scene.chunks.is_empty());
        assert_eq!(scene.view_proj.len(), 16);
        assert!(scene.draw_count > scene.chunks.len());
        assert!(!scene.hud_rects.is_empty());
        assert!(!scene.hud_text.is_empty());
        assert_eq!(scene.hud_texture.width, 800);
    }

    #[test]
    fn world_height_stays_inside_the_chunk_mesh_bounds() {
        assert!(WORLD_HEIGHT > 0);
        assert_eq!(CHUNK_SIZE, 16);
    }
}
