//! Gate G1 in a game window: the window is cleared to the sky colour and one textured quad of 64x64
//! pixels is drawn at [96, 160) on both axes, sampling a generated atlas texel for texel.
//!
//! It is the probe's frame (`horizon_gl2::render`) drawn at the window's own size, not in a fixed
//! 256x256 target. The shaders are GLSL 1.20, so the driver must run OpenGL 2.1 (`version`).

use glow::HasContext;
use horizon_gl2::report::Check;

/// Lower-left corner of the quad in window pixels, and its side. The whole quad is on screen only
/// when the window is at least `QUAD_ORIGIN + QUAD_SIZE` pixels on both sides.
pub const QUAD_ORIGIN: i32 = 96;
pub const QUAD_SIZE: i32 = 64;
pub const ATLAS_SIZE: i32 = 64;
/// The clear colour, outside the quad.
pub const SKY: [u8; 4] = [51, 102, 204, 255];
/// Tolerance per channel for the readback checks.
const TOLERANCE: u8 = 2;

const VERTEX_SHADER: &str = r#"#version 120
attribute vec2 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() {
    v_uv = a_uv;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
"#;

const FRAGMENT_SHADER: &str = r#"#version 120
uniform sampler2D u_atlas;
varying vec2 v_uv;
void main() {
    gl_FragColor = texture2D(u_atlas, v_uv);
}
"#;

/// The generated atlas: a red ramp along x, a green ramp along y, and a blue step for every 16x16
/// tile. A texel read from the wrong place shows up in the readback.
pub fn atlas_texel(x: i32, y: i32) -> [u8; 4] {
    [(x * 4) as u8, (y * 4) as u8, ((x / 16 + y / 16) * 16) as u8, 255]
}

/// A frame as read back: rows from top to bottom, four bytes per pixel.
#[derive(Debug, Clone)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Pixels {
    /// The pixel at window coordinates (x, y), with y counted from the bottom as GL does.
    pub fn at_bottom_up(&self, x: u32, y: u32) -> [u8; 4] {
        let top_down = self.height - 1 - y;
        let at = (top_down * self.width + x) as usize * 4;
        [self.rgba[at], self.rgba[at + 1], self.rgba[at + 2], self.rgba[at + 3]]
    }
}

/// The program, the atlas and the vertex buffer, made once per context.
pub struct Renderer {
    program: glow::Program,
    atlas: glow::Texture,
    buffer: glow::Buffer,
}

impl Renderer {
    /// Compiles and links the program and uploads the atlas. Needs a current OpenGL 2.1 context;
    /// the error is the driver's log when a shader does not compile.
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        // SAFETY: the caller keeps the context current while the renderer lives.
        unsafe {
            let program = link(gl, VERTEX_SHADER, FRAGMENT_SHADER, &[(0, "a_pos"), (1, "a_uv")])?;

            let atlas = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(atlas));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            let bytes = atlas_bytes();
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                ATLAS_SIZE,
                ATLAS_SIZE,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(bytes.as_slice())),
            );

            let buffer = gl.create_buffer()?;
            Ok(Self { program, atlas, buffer })
        }
    }

    /// Draws the frame into the back buffer of a `width` x `height` window, reads it back and
    /// returns it. The caller presents the buffer afterwards.
    pub fn draw(&self, gl: &glow::Context, width: u32, height: u32) -> Result<Pixels, String> {
        if width == 0 || height == 0 {
            return Err(format!("the window has no pixels to draw into ({width}x{height})"));
        }
        let (w, h) = (width as i32, height as i32);
        // SAFETY: the context is current on this thread, as `Renderer::new` requires.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.viewport(0, 0, w, h);
            let sky = SKY.map(|c| f32::from(c) / 255.0);
            gl.clear_color(sky[0], sky[1], sky[2], sky[3]);
            gl.clear(glow::COLOR_BUFFER_BIT);

            // The quad in clip space: the window rectangle [96, 160) on both axes, as NDC.
            let to_x = |px: i32| px as f32 / w as f32 * 2.0 - 1.0;
            let to_y = |px: i32| px as f32 / h as f32 * 2.0 - 1.0;
            let (x0, x1) = (to_x(QUAD_ORIGIN), to_x(QUAD_ORIGIN + QUAD_SIZE));
            let (y0, y1) = (to_y(QUAD_ORIGIN), to_y(QUAD_ORIGIN + QUAD_SIZE));
            // x, y, u, v per vertex, as a triangle strip.
            let vertices: [f32; 16] = [x0, y0, 0.0, 0.0, x1, y0, 1.0, 0.0, x0, y1, 0.0, 1.0, x1, y1, 1.0, 1.0];
            let bytes: Vec<u8> = vertices.iter().flat_map(|v| v.to_ne_bytes()).collect();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STREAM_DRAW);

            gl.use_program(Some(self.program));
            let stride = 4 * 4;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 2 * 4);
            let location = gl.get_uniform_location(self.program, "u_atlas");
            gl.uniform_1_i32(location.as_ref(), 0);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas));
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.disable_vertex_attrib_array(0);
            gl.disable_vertex_attrib_array(1);
            gl.finish();
        }
        read_back(gl, width, height)
    }
}

/// The checks behind gate G1 on a frame: the sky at the four corners, and the atlas texels inside
/// the quad at five points, each read back at its exact pixel. A window too small for the quad
/// fails the texel check rather than panicking.
pub fn checks(pixels: &Pixels) -> Vec<Check> {
    let (w, h) = (pixels.width, pixels.height);
    let mut checks = Vec::new();

    if w == 0 || h == 0 {
        checks.push(failed("gate_g1_clear", "the frame has no pixels".to_string()));
        return checks;
    }

    let quad_start = QUAD_ORIGIN as u32;
    let quad_end = (QUAD_ORIGIN + QUAD_SIZE) as u32;
    let in_quad = |x: u32, y: u32| (quad_start..quad_end).contains(&x) && (quad_start..quad_end).contains(&y);

    // The corners that the quad does not cover: in a window of 160x160 or less the far corner is inside it.
    let (last_x, last_y) = (w - 1, h - 1);
    let corners = [(0, 0), (last_x, 0), (0, last_y), (last_x, last_y)];
    let sky_bad: Vec<String> = corners
        .iter()
        .filter(|&&(x, y)| !in_quad(x, y) && !close(pixels.at_bottom_up(x, y), SKY))
        .map(|&(x, y)| format!("({x},{y})={:?}", pixels.at_bottom_up(x, y)))
        .collect();
    checks.push(Check {
        name: "gate_g1_clear",
        required: true,
        ok: sky_bad.is_empty(),
        detail: if sky_bad.is_empty() {
            format!("the corners of the {w}x{h} window outside the quad equal the sky {SKY:?}")
        } else {
            format!("corners off the sky colour: {}", sky_bad.join(" "))
        },
    });

    if w < quad_end || h < quad_end {
        checks.push(failed(
            "gate_g1_quad_texels",
            format!("the window is {w}x{h}; the quad needs {quad_end}x{quad_end} pixels"),
        ));
        return checks;
    }

    let samples = [(96, 96), (159, 159), (127, 100), (100, 150), (130, 130)];
    let mut bad = Vec::new();
    for &(x, y) in &samples {
        let expected = atlas_texel(x - QUAD_ORIGIN, y - QUAD_ORIGIN);
        let got = pixels.at_bottom_up(x as u32, y as u32);
        if !close(got, expected) {
            bad.push(format!("({x},{y}): got {got:?}, atlas {expected:?}"));
        }
    }
    checks.push(Check {
        name: "gate_g1_quad_texels",
        required: true,
        ok: bad.is_empty(),
        detail: if bad.is_empty() {
            format!("{} sampled pixels equal their atlas texels within {TOLERANCE}", samples.len())
        } else {
            bad.join("; ")
        },
    });
    checks
}

fn failed(name: &'static str, detail: String) -> Check {
    Check { name, required: true, ok: false, detail }
}

fn close(got: [u8; 4], want: [u8; 4]) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= TOLERANCE)
}

fn atlas_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity((ATLAS_SIZE * ATLAS_SIZE * 4) as usize);
    for y in 0..ATLAS_SIZE {
        for x in 0..ATLAS_SIZE {
            bytes.extend_from_slice(&atlas_texel(x, y));
        }
    }
    bytes
}

/// Compiles and links a program from GLSL sources, binding each attribute name to its location
/// first. The error is the driver's log.
pub fn link(
    gl: &glow::Context,
    vertex_source: &str,
    fragment_source: &str,
    attributes: &[(u32, &str)],
) -> Result<glow::Program, String> {
    // SAFETY: the caller has a context current on this thread.
    unsafe {
        let vertex = gl.create_shader(glow::VERTEX_SHADER)?;
        gl.shader_source(vertex, vertex_source);
        gl.compile_shader(vertex);
        if !gl.get_shader_compile_status(vertex) {
            return Err(format!("vertex shader: {}", gl.get_shader_info_log(vertex)));
        }
        let fragment = gl.create_shader(glow::FRAGMENT_SHADER)?;
        gl.shader_source(fragment, fragment_source);
        gl.compile_shader(fragment);
        if !gl.get_shader_compile_status(fragment) {
            return Err(format!("fragment shader: {}", gl.get_shader_info_log(fragment)));
        }
        let program = gl.create_program()?;
        gl.attach_shader(program, vertex);
        gl.attach_shader(program, fragment);
        for &(index, name) in attributes {
            gl.bind_attrib_location(program, index, name);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(format!("link: {}", gl.get_program_info_log(program)));
        }
        // The shaders are inside the program now; the objects can go.
        gl.detach_shader(program, vertex);
        gl.detach_shader(program, fragment);
        gl.delete_shader(vertex);
        gl.delete_shader(fragment);
        Ok(program)
    }
}

/// Reads the back buffer of the current window back as rows from the top, and checks GL's error flag.
pub fn read_back(gl: &glow::Context, width: u32, height: u32) -> Result<Pixels, String> {
    let (w, h) = (width as i32, height as i32);
    // SAFETY: the context is current on this thread.
    unsafe {
        gl.read_buffer(glow::BACK);
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        gl.read_pixels(
            0,
            0,
            w,
            h,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(pixels.as_mut_slice())),
        );
        let error = gl.get_error();
        if error != glow::NO_ERROR {
            return Err(format!("glGetError after the frame: 0x{error:04x}"));
        }
        // GL reads the bottom row first; the checks use the top row first.
        let row = (w * 4) as usize;
        let mut rgba = Vec::with_capacity(pixels.len());
        for y in (0..h as usize).rev() {
            rgba.extend_from_slice(&pixels[y * row..(y + 1) * row]);
        }
        Ok(Pixels { width, height, rgba })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame drawn from the spec alone, in bottom-up coordinates: sky everywhere, the atlas
    /// texel inside the quad.
    fn spec_frame(width: u32, height: u32) -> Pixels {
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let inside = (QUAD_ORIGIN..QUAD_ORIGIN + QUAD_SIZE).contains(&x)
                    && (QUAD_ORIGIN..QUAD_ORIGIN + QUAD_SIZE).contains(&y);
                let texel = if inside {
                    atlas_texel(x - QUAD_ORIGIN, y - QUAD_ORIGIN)
                } else {
                    SKY
                };
                let top_down = height as i32 - 1 - y;
                let at = ((top_down * width as i32 + x) * 4) as usize;
                rgba[at..at + 4].copy_from_slice(&texel);
            }
        }
        Pixels { width, height, rgba }
    }

    fn all_ok(checks: &[Check]) -> bool {
        checks.iter().all(|c| c.ok)
    }

    #[test]
    fn the_spec_frame_passes_at_any_size_that_fits_the_quad() {
        assert!(all_ok(&checks(&spec_frame(160, 160))));
        assert!(all_ok(&checks(&spec_frame(1280, 720))));
    }

    #[test]
    fn a_wrong_texel_inside_the_quad_fails_the_texel_check() {
        let mut frame = spec_frame(320, 240);
        let at = |x: u32, y: u32| ((frame.height - 1 - y) * frame.width + x) as usize * 4;
        let i = at(127, 100);
        frame.rgba[i] = frame.rgba[i].wrapping_add(40);
        let result = checks(&frame);
        assert!(result.iter().find(|c| c.name == "gate_g1_clear").unwrap().ok);
        assert!(!result.iter().find(|c| c.name == "gate_g1_quad_texels").unwrap().ok);
    }

    #[test]
    fn a_window_smaller_than_the_quad_fails_without_panicking() {
        let result = checks(&spec_frame(100, 100));
        let texels = result.iter().find(|c| c.name == "gate_g1_quad_texels").unwrap();
        assert!(!texels.ok);
        assert!(texels.detail.contains("160x160"));
    }

    #[test]
    fn a_sky_pixel_in_the_corner_fails_the_clear_check() {
        let mut frame = spec_frame(200, 200);
        frame.rgba[0] = 0;
        let result = checks(&frame);
        assert!(!result.iter().find(|c| c.name == "gate_g1_clear").unwrap().ok);
    }
}
