//! The GL2 terrain (docs/plans/opengl21-radeon-x1650.md, phase 2): every section with faces gets one
//! vertex buffer; each frame the sections outside the view frustum are skipped, and each remaining one
//! is one draw call. GLSL 1.20, OpenGL 2.0 calls, no instancing, no uniform buffers, no texture arrays:
//! the atlas is one 64x64 texture, and the fog is computed per vertex.

use std::time::Instant;

use glow::HasContext;
use horizon_gl2::report::Check;

use crate::frame::{self, Pixels};
use crate::math::{self, Mat4, Vec3};
use crate::world::{self, World, FLOATS_PER_VERTEX};

/// The sky and the fog share a colour, so distant ground fades into the sky.
pub const SKY: [u8; 4] = [153, 204, 255, 255];
const TOLERANCE: u8 = 2;
/// Bytes between the starts of two vertices.
const STRIDE: i32 = (FLOATS_PER_VERTEX * 4) as i32;

const VERTEX_SHADER: &str = r#"#version 120
attribute vec3 a_pos;
attribute vec2 a_uv;
attribute float a_shade;
uniform mat4 u_view_proj;
uniform vec3 u_eye;
uniform float u_fog_start;
uniform float u_fog_end;
varying vec2 v_uv;
varying float v_shade;
varying float v_fog;
void main() {
    v_uv = a_uv;
    v_shade = a_shade;
    gl_Position = u_view_proj * vec4(a_pos, 1.0);
    float d = distance(a_pos, u_eye);
    v_fog = clamp((d - u_fog_start) / (u_fog_end - u_fog_start), 0.0, 1.0);
}
"#;

const FRAGMENT_SHADER: &str = r#"#version 120
uniform sampler2D u_atlas;
uniform vec3 u_fog_color;
varying vec2 v_uv;
varying float v_shade;
varying float v_fog;
void main() {
    vec3 lit = texture2D(u_atlas, v_uv).rgb * v_shade;
    gl_FragColor = vec4(mix(lit, u_fog_color, v_fog), 1.0);
}
"#;

/// Where the terrain is seen from, and how far the fog reaches.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    pub fovy: f32,
    pub near: f32,
    pub far: f32,
    pub fog_start: f32,
    pub fog_end: f32,
}

impl Camera {
    /// The view of the terrain check on frame `frame`: above the hills, looking down at the centre of the
    /// world from 62 blocks out, and circling it at 3 degrees a frame from the south-west corner. Frame 0
    /// is at about (20.2, 26, 20.2), so the culling changes from frame to frame, as it does when the player moves.
    pub fn orbit(frame: u32) -> Self {
        let centre = [64.0, 10.0, 64.0];
        let angle = (225.0 + 3.0 * frame as f32).to_radians();
        let radius = 62.0;
        Self {
            eye: [centre[0] + radius * angle.cos(), 26.0, centre[2] + radius * angle.sin()],
            target: centre,
            fovy: 60.0,
            near: 0.5,
            far: 300.0,
            fog_start: 80.0,
            fog_end: 160.0,
        }
    }
}

/// What one frame cost, in the units the plan budgets: sections and draw calls, not frames per second.
/// `frame_ms` is the CPU time of the frame, including the GPU's work up to the readback. llvmpipe is
/// a CPU rasteriser, so this number says nothing about the Radeon X1650.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub sections: u32,
    pub empty: u32,
    pub culled: u32,
    pub drawn: u32,
    pub draw_calls: u32,
    pub triangles: u64,
    /// Bytes of vertex data on the GPU for the whole terrain, not per frame.
    pub buffer_bytes: u64,
    pub frame_ms: f64,
}

struct Section {
    min: Vec3,
    max: Vec3,
    buffer: glow::Buffer,
    vertices: i32,
}

pub struct Terrain {
    program: glow::Program,
    atlas: glow::Texture,
    sections: Vec<Section>,
    total: u32,
    empty: u32,
    buffer_bytes: u64,
}

impl Terrain {
    /// Meshes every section of `world` and uploads the non-empty ones. Needs a current context.
    pub fn new(gl: &glow::Context, world: &World) -> Result<Self, String> {
        let program = frame::link(gl, VERTEX_SHADER, FRAGMENT_SHADER, &[(0, "a_pos"), (1, "a_uv"), (2, "a_shade")])?;
        let atlas = upload_atlas(gl)?;
        let mut sections = Vec::new();
        let (mut total, mut empty, mut buffer_bytes) = (0, 0, 0u64);
        let sections_xz = world.size_xz() / world::SECTION;
        for sy in 0..world.size_y() / world::SECTION {
            for sz in 0..sections_xz {
                for sx in 0..sections_xz {
                    total += 1;
                    let mesh = world::build_section(world, sx, sy, sz);
                    if mesh.faces == 0 {
                        empty += 1;
                        continue;
                    }
                    let bytes: Vec<u8> = mesh.vertices.iter().flat_map(|v| v.to_ne_bytes()).collect();
                    buffer_bytes += bytes.len() as u64;
                    // SAFETY: the context is current, as the caller guarantees.
                    let buffer = unsafe {
                        let buffer = gl.create_buffer()?;
                        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
                        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);
                        buffer
                    };
                    let min = [sx * world::SECTION, sy * world::SECTION, sz * world::SECTION].map(|v| v as f32);
                    let max = min.map(|v| v + world::SECTION as f32);
                    sections.push(Section { min, max, buffer, vertices: mesh.vertex_count() as i32 });
                }
            }
        }
        Ok(Self { program, atlas, sections, total, empty, buffer_bytes })
    }

    /// Draws the terrain from `camera` into a `width` x `height` window and reads the frame back.
    pub fn draw(&self, gl: &glow::Context, width: u32, height: u32, camera: &Camera) -> Result<(Pixels, Stats), String> {
        let started = Instant::now();
        let aspect = width as f32 / height as f32;
        let view = math::look_at(camera.eye, camera.target, [0.0, 1.0, 0.0]);
        let proj = math::perspective(camera.fovy, aspect, camera.near, camera.far);
        let view_proj: Mat4 = math::mul(&proj, &view);
        let planes = math::frustum_planes(&view_proj);

        let mut stats = Stats { sections: self.total, empty: self.empty, buffer_bytes: self.buffer_bytes, ..Stats::default() };
        // SAFETY: the context is current on this thread; every handle was made on it.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.viewport(0, 0, width as i32, height as i32);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LESS);
            let sky = SKY.map(|c| f32::from(c) / 255.0);
            gl.clear_color(sky[0], sky[1], sky[2], sky[3]);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(self.program, "u_view_proj").as_ref(), false, &view_proj);
            gl.uniform_3_f32(gl.get_uniform_location(self.program, "u_eye").as_ref(), camera.eye[0], camera.eye[1], camera.eye[2]);
            gl.uniform_1_f32(gl.get_uniform_location(self.program, "u_fog_start").as_ref(), camera.fog_start);
            gl.uniform_1_f32(gl.get_uniform_location(self.program, "u_fog_end").as_ref(), camera.fog_end);
            gl.uniform_3_f32(
                gl.get_uniform_location(self.program, "u_fog_color").as_ref(),
                sky[0],
                sky[1],
                sky[2],
            );
            gl.uniform_1_i32(gl.get_uniform_location(self.program, "u_atlas").as_ref(), 0);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas));

            gl.enable_vertex_attrib_array(0);
            gl.enable_vertex_attrib_array(1);
            gl.enable_vertex_attrib_array(2);
            for section in &self.sections {
                if !math::aabb_visible(&planes, section.min, section.max) {
                    stats.culled += 1;
                    continue;
                }
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(section.buffer));
                gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, STRIDE, 0);
                gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, STRIDE, 12);
                gl.vertex_attrib_pointer_f32(2, 1, glow::FLOAT, false, STRIDE, 20);
                gl.draw_arrays(glow::TRIANGLES, 0, section.vertices);
                stats.drawn += 1;
                stats.draw_calls += 1;
                stats.triangles += (section.vertices / 3) as u64;
            }
            gl.disable_vertex_attrib_array(0);
            gl.disable_vertex_attrib_array(1);
            gl.disable_vertex_attrib_array(2);
            gl.finish();
        }
        let pixels = frame::read_back(gl, width, height)?;
        stats.frame_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok((pixels, stats))
    }
}

/// The checks of the terrain frame: the terrain is drawn and culled, the top of the frame is sky, the
/// centre is ground, and the ground covers part of the frame.
pub fn checks(pixels: &Pixels, stats: &Stats) -> Vec<Check> {
    let mut checks = Vec::new();

    let drawn_ok = stats.drawn > 0 && stats.draw_calls == stats.drawn;
    checks.push(Check {
        name: "terrain_sections_drawn",
        required: true,
        ok: drawn_ok,
        detail: format!("{} sections drawn in {} draw calls of {} with faces", stats.drawn, stats.draw_calls, stats.sections - stats.empty),
    });

    let culled_ok = stats.culled > 0 && stats.drawn + stats.culled + stats.empty == stats.sections;
    checks.push(Check {
        name: "terrain_culling",
        required: true,
        ok: culled_ok,
        detail: format!("{} sections outside the frustum, {} empty, of {}", stats.culled, stats.empty, stats.sections),
    });

    let (w, h) = (pixels.width, pixels.height);
    let top_is_sky = (0..w).step_by(8).all(|x| close(pixels.at_bottom_up(x, h - 1), SKY) && close(pixels.at_bottom_up(x, h - 2), SKY));
    checks.push(Check {
        name: "terrain_sky_top",
        required: true,
        ok: top_is_sky,
        detail: "the top two rows of the frame equal the sky".to_string(),
    });

    let centre = pixels.at_bottom_up(w / 2, h / 2);
    checks.push(Check {
        name: "terrain_centre_is_ground",
        required: true,
        ok: !close(centre, SKY),
        detail: format!("the centre pixel is {centre:?}, the sky is {SKY:?}"),
    });

    let total = (w * h) as usize;
    let sky_pixels = pixels.rgba.chunks_exact(4).filter(|p| close([p[0], p[1], p[2], p[3]], SKY)).count();
    let ground_share = 1.0 - sky_pixels as f64 / total as f64;
    checks.push(Check {
        name: "terrain_coverage",
        required: true,
        ok: (0.3..=0.95).contains(&ground_share),
        detail: format!("ground covers {:.1}% of the frame (expected 30% to 95%)", ground_share * 100.0),
    });
    checks
}

fn close(got: [u8; 4], want: [u8; 4]) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= TOLERANCE)
}

/// The atlas tile colours, in `world::Tile` order.
const GRASS: [u8; 3] = [96, 156, 56];
const DIRT: [u8; 3] = [134, 96, 67];
const STONE: [u8; 3] = [124, 124, 124];

/// A 64x64 atlas of four 16x16 tiles: grass top, grass side (a green band over dirt), dirt, stone.
/// Each texel has a deterministic speckle so the faces are not flat.
fn atlas_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(64 * 64 * 4);
    for y in 0..64u32 {
        for x in 0..64u32 {
            let tile = (y / 16) * 4 + x / 16;
            let ly = y % 16;
            let base = match tile {
                0 => GRASS,
                1 if ly < 4 => GRASS,
                1 => DIRT,
                2 => DIRT,
                _ => STONE,
            };
            let speckle = noise(x, y);
            for channel in base {
                bytes.push((i32::from(channel) + speckle).clamp(0, 255) as u8);
            }
            bytes.push(255);
        }
    }
    bytes
}

fn noise(x: u32, y: u32) -> i32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    ((h >> 8) % 17) as i32 - 8
}

fn upload_atlas(gl: &glow::Context) -> Result<glow::Texture, String> {
    let bytes = atlas_bytes();
    // SAFETY: the context is current on this thread.
    unsafe {
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            64,
            64,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(bytes.as_slice())),
        );
        Ok(texture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_with(width: u32, height: u32, fill: impl Fn(u32, u32) -> [u8; 4]) -> Pixels {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&fill(x, height - 1 - row));
            }
        }
        Pixels { width, height, rgba }
    }

    fn stats(sections: u32, empty: u32, culled: u32, drawn: u32) -> Stats {
        Stats { sections, empty, culled, drawn, draw_calls: drawn, ..Stats::default() }
    }

    fn passed(checks: &[Check]) -> Vec<&'static str> {
        checks.iter().filter(|c| c.ok).map(|c| c.name).collect()
    }

    #[test]
    fn a_frame_with_sky_on_top_and_ground_below_passes_every_check() {
        // Ground from the bottom up to 70% of the height, sky above.
        let pixels = frame_with(64, 64, |_, y| if y < 45 { [120, 90, 60, 255] } else { SKY });
        let result = checks(&pixels, &stats(128, 20, 40, 68));
        assert_eq!(passed(&result).len(), result.len(), "{result:?}");
    }

    #[test]
    fn a_frame_of_only_sky_fails_the_ground_checks() {
        let pixels = frame_with(64, 64, |_, _| SKY);
        let result = checks(&pixels, &stats(128, 20, 40, 68));
        let failed: Vec<_> = result.iter().filter(|c| !c.ok).map(|c| c.name).collect();
        assert_eq!(failed, ["terrain_centre_is_ground", "terrain_coverage"]);
    }

    #[test]
    fn a_frame_drawn_without_any_section_fails_the_drawn_check() {
        let pixels = frame_with(64, 64, |_, y| if y < 45 { [120, 90, 60, 255] } else { SKY });
        let result = checks(&pixels, &stats(128, 20, 108, 0));
        assert!(!result.iter().find(|c| c.name == "terrain_sections_drawn").unwrap().ok);
    }

    #[test]
    fn the_orbit_starts_at_the_first_view_and_keeps_its_height_and_distance() {
        // 225 degrees puts the eye at 62 * cos(45 degrees) short of the centre on both horizontal axes.
        let start = Camera::orbit(0);
        let expected = 64.0 - 62.0 * std::f32::consts::FRAC_1_SQRT_2;
        assert!((start.eye[0] - expected).abs() < 1e-3 && (start.eye[2] - expected).abs() < 1e-3, "{:?}", start.eye);
        for frame in [1, 17, 59] {
            let c = Camera::orbit(frame);
            assert_eq!(c.eye[1], 26.0);
            let horizontal = ((c.eye[0] - 64.0).powi(2) + (c.eye[2] - 64.0).powi(2)).sqrt();
            assert!((horizontal - 62.0).abs() < 1e-3, "frame {frame}: {horizontal}");
        }
    }

    #[test]
    fn the_atlas_has_four_by_four_tiles_of_sixteen_texels() {
        let bytes = atlas_bytes();
        assert_eq!(bytes.len(), 64 * 64 * 4);
        // The tile to the right of the grass top is the grass side: its top band is green, its lower part dirt.
        let texel = |x: usize, y: usize| &bytes[(y * 64 + x) * 4..(y * 64 + x) * 4 + 3];
        assert!(texel(16 + 2, 1)[1] > texel(16 + 2, 1)[0], "green band on the side");
        assert!(texel(16 + 2, 10)[0] > texel(16 + 2, 10)[1], "dirt below it");
    }
}
