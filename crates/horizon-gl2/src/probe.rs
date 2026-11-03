#![allow(unused_unsafe)]

//! The probe proper: what the driver reports, and whether a GLSL 1.10 triangle renders right
//! into a framebuffer whose size is not a power of two.
//!
//! Everything here sticks to OpenGL 2.0 core plus GLSL 1.10: no vertex array objects, no
//! texture arrays, no instancing, no uniform buffers, no integer maths in shaders. That is
//! the floor the Radeon X1650's driver gives (docs/plans/opengl21-radeon-x1650.md, 2.1).

use glow::HasContext;

use crate::egl;
use crate::report::{Check, Report, parse_version};

/// Framebuffer size: deliberately not powers of two.
const WIDTH: i32 = 100;
const HEIGHT: i32 = 60;
/// Texels the test samples: a 2x2 texture, green.
const TEXEL: [u8; 4] = [0, 255, 0, 255];
/// Vertex colour in the test: red 1.0, green 0.5, blue 0.0. Output = texel * colour.
const VERTEX_COLOUR: [f32; 3] = [1.0, 0.5, 0.0];
/// Expected pixel, and how far each channel may differ (rounding of 0.5 * 255).
const EXPECTED: [u8; 4] = [0, 128, 0, 255];
const TOLERANCE: u8 = 2;

const VERTEX_SHADER: &str = "#version 110
attribute vec2 a_pos;
attribute vec2 a_uv;
attribute vec3 a_color;
varying vec2 v_uv;
varying vec3 v_color;
void main() {
    v_uv = a_uv;
    v_color = a_color;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
";

const FRAGMENT_SHADER: &str = "#version 110
uniform sampler2D u_tex;
varying vec2 v_uv;
varying vec3 v_color;
void main() {
    gl_FragColor = texture2D(u_tex, v_uv) * vec4(v_color, 1.0);
}
";

/// GL limits a renderer must size its work by, with the names the report prints.
const LIMITS: [(&str, u32); 9] = [
    ("GL_MAX_TEXTURE_SIZE", glow::MAX_TEXTURE_SIZE),
    ("GL_MAX_RENDERBUFFER_SIZE", glow::MAX_RENDERBUFFER_SIZE),
    ("GL_MAX_DRAW_BUFFERS", glow::MAX_DRAW_BUFFERS),
    ("GL_MAX_VERTEX_ATTRIBS", glow::MAX_VERTEX_ATTRIBS),
    ("GL_MAX_VERTEX_UNIFORM_COMPONENTS", glow::MAX_VERTEX_UNIFORM_COMPONENTS),
    ("GL_MAX_FRAGMENT_UNIFORM_COMPONENTS", glow::MAX_FRAGMENT_UNIFORM_COMPONENTS),
    ("GL_MAX_VARYING_FLOATS", glow::MAX_VARYING_FLOATS),
    ("GL_MAX_TEXTURE_IMAGE_UNITS", glow::MAX_TEXTURE_IMAGE_UNITS),
    ("GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS", glow::MAX_VERTEX_TEXTURE_IMAGE_UNITS),
];

/// Creates the context, gathers what it reports and runs the render check.
pub fn run() -> Result<Report, String> {
    let context = egl::Context::new_surfaceless().map_err(|e| e.to_string())?;
    let gl = context.glow();
    let mut report = Report::default();
    // SAFETY: `gl` belongs to the current context, which `context` keeps alive until the end.
    unsafe {
        gather(&gl, &mut report);
        render_check(&gl, &mut report);
    }
    Ok(report)
}

/// Reads a string state. glow panics when the driver returns NULL (a query its context does not
/// support), so that becomes a placeholder: the probe still runs the render check and reports it.
unsafe fn string_parameter(gl: &glow::Context, pname: u32) -> String {
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        gl.get_parameter_string(pname)
    }));
    read.unwrap_or_else(|_| "(not reported)".to_string())
}

unsafe fn gather(gl: &glow::Context, report: &mut Report) {
    report.gl_version = unsafe { string_parameter(gl, glow::VERSION) };
    report.glsl_version = unsafe { string_parameter(gl, glow::SHADING_LANGUAGE_VERSION) };
    report.renderer = unsafe { string_parameter(gl, glow::RENDERER) };
    report.vendor = unsafe { string_parameter(gl, glow::VENDOR) };
    report.extensions = unsafe { string_parameter(gl, glow::EXTENSIONS) }
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    for (name, pname) in LIMITS {
        report.limits.push((name, unsafe { gl.get_parameter_i32(pname) }));
    }

    let gl_version = parse_version(&report.gl_version);
    let glsl_version = parse_version(&report.glsl_version);
    report.checks.push(version_check("gl_2_0", (2, 0), gl_version, true));
    report.checks.push(version_check("gl_2_1", (2, 1), gl_version, false));
    report.checks.push(version_check("glsl_1_10", (1, 10), glsl_version, true));
    report.checks.push(version_check("glsl_1_20", (1, 20), glsl_version, false));
}

fn version_check(name: &'static str, wanted: (u32, u32), found: Option<(u32, u32)>, required: bool) -> Check {
    let (ok, detail) = match found {
        Some(found) => (found >= wanted, format!("driver {}.{}, wanted {}.{}", found.0, found.1, wanted.0, wanted.1)),
        None => (false, "version string not understood".to_string()),
    };
    Check { name, required, ok, detail }
}

unsafe fn render_check(gl: &glow::Context, report: &mut Report) {
    let program = match unsafe { build_program(gl) } {
        Ok(program) => {
            report.checks.push(Check {
                name: "shaders_glsl_110",
                required: true,
                ok: true,
                detail: "vertex and fragment shaders compile and link".into(),
            });
            program
        }
        Err(log) => {
            report.checks.push(Check { name: "shaders_glsl_110", required: true, ok: false, detail: log });
            return;
        }
    };

    let outcome = unsafe { draw_triangle(gl, program) };
    report.checks.push(Check {
        name: "framebuffer_npot_complete",
        required: true,
        ok: outcome.framebuffer_complete,
        detail: format!("{WIDTH}x{HEIGHT} RGBA8 colour target"),
    });
    report.first_pixel = outcome.first_pixel;
    report.pixels_checked = outcome.pixels_checked;
    report.pixels_mismatched = outcome.pixels_mismatched;
    report.checks.push(Check {
        name: "textured_triangle",
        required: true,
        ok: outcome.pixels_checked > 0 && outcome.pixels_mismatched == 0,
        detail: format!(
            "{} of {} pixels equal {:?} within {}; first pixel {:?}",
            outcome.pixels_checked - outcome.pixels_mismatched,
            outcome.pixels_checked,
            EXPECTED,
            TOLERANCE,
            outcome.first_pixel
        ),
    });
    let error = unsafe { gl.get_error() };
    report.checks.push(Check {
        name: "no_gl_error",
        required: true,
        ok: error == glow::NO_ERROR,
        detail: format!("glGetError after the render: 0x{error:04x}"),
    });
}

/// Compiles and links the probe's program. The error is the driver's log.
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
        // Attribute locations are fixed before linking, as a GL 2.x renderer must do.
        gl.bind_attrib_location(program, 0, "a_pos");
        gl.bind_attrib_location(program, 1, "a_uv");
        gl.bind_attrib_location(program, 2, "a_color");
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(format!("link: {}", gl.get_program_info_log(program)));
        }
        Ok(program)
    }
}

struct Outcome {
    framebuffer_complete: bool,
    first_pixel: [u8; 4],
    pixels_checked: u32,
    pixels_mismatched: u32,
}

/// Draws one full-screen triangle sampling a 2x2 texture into the NPOT framebuffer and reads
/// every pixel back. Only GL 2.0 calls, vertex arrays as client-side attribute pointers.
unsafe fn draw_triangle(gl: &glow::Context, program: glow::Program) -> Outcome {
    let mut outcome = Outcome {
        framebuffer_complete: false,
        first_pixel: [0; 4],
        pixels_checked: 0,
        pixels_mismatched: 0,
    };
    unsafe {
        // Texture: 2x2, green.
        let texture = gl.create_texture().expect("texture");
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        let texels: Vec<u8> = TEXEL.iter().copied().cycle().take(2 * 2 * 4).collect();
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            2,
            2,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(texels.as_slice())),
        );

        // Colour target: WIDTH x HEIGHT, not powers of two.
        let target = gl.create_texture().expect("target texture");
        gl.bind_texture(glow::TEXTURE_2D, Some(target));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            WIDTH,
            HEIGHT,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        let framebuffer = gl.create_framebuffer().expect("framebuffer");
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(target),
            0,
        );
        outcome.framebuffer_complete =
            gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;

        // One triangle covering the target: x, y, u, v, r, g, b per vertex.
        // Clip space (-1,-1), (3,-1), (-1,3) maps uv to (0,0), (2,0), (0,2).
        let vertices: [f32; 21] = [
            -1.0, -1.0, 0.0, 0.0, VERTEX_COLOUR[0], VERTEX_COLOUR[1], VERTEX_COLOUR[2], //
            3.0, -1.0, 2.0, 0.0, VERTEX_COLOUR[0], VERTEX_COLOUR[1], VERTEX_COLOUR[2], //
            -1.0, 3.0, 0.0, 2.0, VERTEX_COLOUR[0], VERTEX_COLOUR[1], VERTEX_COLOUR[2],
        ];
        let bytes: Vec<u8> = vertices.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let buffer = gl.create_buffer().expect("buffer");
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);

        gl.use_program(Some(program));
        let stride = 7 * 4;
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 2 * 4);
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_f32(2, 3, glow::FLOAT, false, stride, 4 * 4);
        let location = gl.get_uniform_location(program, "u_tex");
        gl.uniform_1_i32(location.as_ref(), 0);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));

        gl.viewport(0, 0, WIDTH, HEIGHT);
        gl.clear_color(0.0, 0.0, 0.0, 0.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.finish();

        let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
        gl.read_pixels(
            0,
            0,
            WIDTH,
            HEIGHT,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(pixels.as_mut_slice())),
        );
        outcome.first_pixel = [pixels[0], pixels[1], pixels[2], pixels[3]];
        for pixel in pixels.chunks_exact(4) {
            outcome.pixels_checked += 1;
            let close = pixel
                .iter()
                .zip(EXPECTED)
                .all(|(got, want)| got.abs_diff(want) <= TOLERANCE);
            if !close {
                outcome.pixels_mismatched += 1;
            }
        }

        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        gl.use_program(None);
        gl.delete_framebuffer(framebuffer);
        gl.delete_texture(target);
        gl.delete_texture(texture);
        gl.delete_buffer(buffer);
    }
    outcome
}
