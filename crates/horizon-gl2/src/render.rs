//! The first slice of the GL 2.1 renderer (docs/plans/opengl21-radeon-x1650.md, phase 1, gate G1):
//! a clear and one textured quad, drawn into an offscreen target and read back.
//!
//! It uses OpenGL 2.0 calls and GLSL 1.20, which is what the Radeon X1650's driver offers. The atlas
//! is generated here because game assets are not part of the repository. The quad covers a 64x64
//! block of pixels with a 64x64 atlas, so every pixel samples exactly one texel and the readback
//! can be compared texel by texel.

use glow::HasContext;

use crate::report::Check;

/// Size of the colour target: a power of two, so it is also valid on GL 2.0 without NPOT.
pub const SIZE: i32 = 256;
/// The quad's lower-left corner in window pixels and its side length.
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

/// A rendered frame: rows from top to bottom, four bytes per pixel.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Frame {
    /// The pixel at window coordinates (x, y), with y counted from the bottom as GL does.
    pub fn pixel_bottom_up(&self, x: u32, y: u32) -> [u8; 4] {
        let row = y;
        let top_down = self.height - 1 - row;
        let at = (top_down * self.width + x) as usize * 4;
        [self.rgba[at], self.rgba[at + 1], self.rgba[at + 2], self.rgba[at + 3]]
    }
}

/// Draws the quad over the sky and reads it back. `on_screen` draws into the back buffer of the
/// current window instead of the offscreen target; the caller presents it afterwards.
pub fn render(gl: &glow::Context, on_screen: bool) -> Result<Frame, String> {
    // SAFETY: the caller has made a context current, and it outlives this call.
    unsafe {
        let program = build_program(gl)?;
        let atlas = atlas_bytes();

        let atlas_texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(atlas_texture));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            ATLAS_SIZE,
            ATLAS_SIZE,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(atlas.as_slice())),
        );

        if on_screen {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.read_buffer(glow::BACK);
        } else {
            let target = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(target));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                SIZE,
                SIZE,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            let framebuffer = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(target),
                0,
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                return Err(format!("framebuffer incomplete: 0x{status:04x}"));
            }
        }

        // The quad in clip space: the window rectangle [96, 160) on both axes, as NDC.
        let to_ndc = |px: i32| px as f32 / SIZE as f32 * 2.0 - 1.0;
        let (lo, hi) = (to_ndc(QUAD_ORIGIN), to_ndc(QUAD_ORIGIN + QUAD_SIZE));
        // x, y, u, v per vertex, as a triangle strip.
        let vertices: [f32; 16] = [lo, lo, 0.0, 0.0, hi, lo, 1.0, 0.0, lo, hi, 0.0, 1.0, hi, hi, 1.0, 1.0];
        let bytes: Vec<u8> = vertices.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let buffer = gl.create_buffer()?;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);

        gl.use_program(Some(program));
        let stride = 4 * 4;
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 2 * 4);
        let location = gl.get_uniform_location(program, "u_atlas");
        gl.uniform_1_i32(location.as_ref(), 0);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(atlas_texture));

        gl.viewport(0, 0, SIZE, SIZE);
        let sky = SKY.map(|c| f32::from(c) / 255.0);
        gl.clear_color(sky[0], sky[1], sky[2], sky[3]);
        gl.clear(glow::COLOR_BUFFER_BIT);
        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        gl.finish();

        let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
        gl.read_pixels(
            0,
            0,
            SIZE,
            SIZE,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(pixels.as_mut_slice())),
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        let error = gl.get_error();
        if error != glow::NO_ERROR {
            return Err(format!("glGetError after the frame: 0x{error:04x}"));
        }

        // GL reads the bottom row first; the file and the checks use the top row first.
        let row = (SIZE * 4) as usize;
        let mut rgba = Vec::with_capacity(pixels.len());
        for y in (0..SIZE as usize).rev() {
            rgba.extend_from_slice(&pixels[y * row..(y + 1) * row]);
        }
        Ok(Frame { width: SIZE as u32, height: SIZE as u32, rgba })
    }
}

/// The checks behind gate G1 on a rendered frame: the sky outside the quad, and the atlas texels
/// inside it at five points, each read back at its exact pixel.
pub fn checks(frame: &Frame) -> Vec<Check> {
    let last = frame.width - 1;
    let corners = [(0, 0), (last, 0), (0, last), (last, last)];
    let sky_bad: Vec<_> = corners
        .iter()
        .filter(|&&(x, y)| !close(frame.pixel_bottom_up(x, y), SKY))
        .map(|&(x, y)| format!("({x},{y})={:?}", frame.pixel_bottom_up(x, y)))
        .collect();
    let mut checks = vec![Check {
        name: "gate_g1_clear",
        required: true,
        ok: sky_bad.is_empty(),
        detail: if sky_bad.is_empty() {
            format!("the four corners equal the sky {SKY:?}")
        } else {
            format!("corners off the sky colour: {}", sky_bad.join(" "))
        },
    }];

    let samples = [(96, 96), (159, 159), (127, 100), (100, 150), (130, 130)];
    let mut bad = Vec::new();
    for &(x, y) in &samples {
        let expected = atlas_texel(x as i32 - QUAD_ORIGIN, y as i32 - QUAD_ORIGIN);
        let got = frame.pixel_bottom_up(x, y);
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

/// Compiles and links the GLSL 1.20 program. The error is the driver's log.
unsafe fn build_program(gl: &glow::Context) -> Result<glow::Program, String> {
    unsafe {
        let vertex = gl.create_shader(glow::VERTEX_SHADER)?;
        gl.shader_source(vertex, VERTEX_SHADER);
        gl.compile_shader(vertex);
        if !gl.get_shader_compile_status(vertex) {
            return Err(format!("vertex shader: {}", gl.get_shader_info_log(vertex)));
        }
        let fragment = gl.create_shader(glow::FRAGMENT_SHADER)?;
        gl.shader_source(fragment, FRAGMENT_SHADER);
        gl.compile_shader(fragment);
        if !gl.get_shader_compile_status(fragment) {
            return Err(format!("fragment shader: {}", gl.get_shader_info_log(fragment)));
        }
        let program = gl.create_program()?;
        gl.attach_shader(program, vertex);
        gl.attach_shader(program, fragment);
        gl.bind_attrib_location(program, 0, "a_pos");
        gl.bind_attrib_location(program, 1, "a_uv");
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(format!("link: {}", gl.get_program_info_log(program)));
        }
        Ok(program)
    }
}
