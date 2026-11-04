//! The HUD for OpenGL 2: the GUI's vertices (`HudBatch`) drawn over the frame, alpha-blended from one
//! RGBA atlas. Runs take the blend the GUI set for them; the blends OpenGL 2 cannot draw yet (vignette,
//! end portal, CIT glint) fall back to alpha, and a glint layer (`uv.x >= 2`) samples its item without
//! the glint.

use std::ops::Range;

use glow::HasContext;

use crate::font::Patch;
use crate::hud::{Blend, HudBatch, HudVertex};

use super::shader;

const VERTEX_SHADER: &str = r#"#version 120
uniform vec3 u_size;
attribute vec2 a_pos;
attribute vec2 a_uv;
attribute vec4 a_color;
varying vec2 v_uv;
varying vec4 v_color;
void main() {
    // hud.wgsl: `pixels = pos * scale`, then the surface's pixels map to clip space with y down.
    vec2 pixels = a_pos * u_size.z;
    gl_Position = vec4(pixels.x / u_size.x * 2.0 - 1.0, 1.0 - pixels.y / u_size.y * 2.0, 0.0, 1.0);
    v_uv = a_uv;
    v_color = a_color;
}
"#;

const FRAGMENT_SHADER: &str = r#"#version 120
uniform sampler2D u_atlas;
varying vec2 v_uv;
varying vec4 v_color;
void main() {
    vec2 uv = v_uv;
    if (uv.x >= 2.0) {
        uv.x -= 2.0;
    }
    gl_FragColor = texture2D(u_atlas, uv) * v_color;
}
"#;

const ATTRIBUTES: [&str; 3] = ["a_pos", "a_uv", "a_color"];

/// The HUD's atlas on the GPU, its program and one vertex buffer for the batch.
pub struct Hud {
    program: glow::Program,
    atlas: glow::Texture,
    vertices: glow::Buffer,
}

impl Hud {
    /// Uploads the atlas (`width` by `height` RGBA8 texels) and builds the program. Needs a current
    /// context.
    pub fn new(gl: &glow::Context, pixels: &[u8], width: u32, height: u32) -> Result<Self, String> {
        if pixels.len() != (width as usize) * (height as usize) * 4 {
            return Err(format!(
                "the HUD atlas holds {} bytes, not {width}x{height} RGBA",
                pixels.len()
            ));
        }
        let program = shader::program(gl, VERTEX_SHADER, FRAGMENT_SHADER, &ATTRIBUTES)?;
        // SAFETY: the context is current on this thread, as the caller requires.
        unsafe {
            let atlas = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(atlas));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(pixels)),
            );
            let vertices = gl.create_buffer()?;
            Ok(Self {
                program,
                atlas,
                vertices,
            })
        }
    }

    /// Writes a patch of the atlas (a glyph, or a sprite the GUI changed).
    pub fn patch(&self, gl: &glow::Context, patch: &Patch) {
        if patch.rgba.len() != (patch.width as usize) * (patch.height as usize) * 4 {
            return;
        }
        // SAFETY: the context is current on this thread and the atlas was made on it.
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas));
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                patch.x as i32,
                patch.y as i32,
                patch.width as i32,
                patch.height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(patch.rgba.as_slice())),
            );
        }
    }

    /// Draws the batch over a surface of `surface` pixels, run by run in the order of its blends.
    /// Depth testing must be off.
    pub fn draw(&self, gl: &glow::Context, batch: &HudBatch, surface: [u32; 2]) {
        if batch.vertices.is_empty() {
            return;
        }
        let stride = std::mem::size_of::<HudVertex>() as i32;
        let bytes: &[u8] = bytemuck::cast_slice(&batch.vertices);
        // SAFETY: the context is current on this thread; every handle was made on it.
        unsafe {
            gl.use_program(Some(self.program));
            gl.uniform_3_f32(
                gl.get_uniform_location(self.program, "u_size").as_ref(),
                surface[0] as f32,
                surface[1] as f32,
                batch.scale,
            );
            gl.uniform_1_i32(gl.get_uniform_location(self.program, "u_atlas").as_ref(), 0);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vertices));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 8);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 4, glow::UNSIGNED_BYTE, true, stride, 16);
            gl.enable(glow::BLEND);
            let len = batch.vertices.len() as u32;
            for (range, blend) in runs(len, &batch.blends) {
                let (source, destination) = factors(blend);
                gl.blend_func(source, destination);
                gl.draw_arrays(
                    glow::TRIANGLES,
                    range.start as i32,
                    (range.end - range.start) as i32,
                );
            }
            gl.disable(glow::BLEND);
            for index in 0..ATTRIBUTES.len() {
                gl.disable_vertex_attrib_array(index as u32);
            }
        }
    }

    /// Frees the program, the atlas and the buffer. The context must be current.
    pub fn free(&self, gl: &glow::Context) {
        // SAFETY: every handle was made on this context by `new`.
        unsafe {
            gl.delete_program(self.program);
            gl.delete_texture(self.atlas);
            gl.delete_buffer(self.vertices);
        }
    }
}

/// The vertex ranges to draw, each with its blend: the GUI's blends as given, and alpha for the
/// vertices none of them covers. Ranges past the batch are cut.
pub fn runs(len: u32, blends: &[(Range<u32>, Blend)]) -> Vec<(Range<u32>, Blend)> {
    let mut out = Vec::new();
    let mut at = 0;
    for (range, blend) in blends {
        let start = range.start.clamp(at, len);
        let end = range.end.clamp(start, len);
        if start > at {
            out.push((at..start, Blend::Alpha));
        }
        if end > start {
            out.push((start..end, *blend));
        }
        at = end;
    }
    if at < len {
        out.push((at..len, Blend::Alpha));
    }
    out
}

/// The blend factors of a run. Blends with no OpenGL 2 equivalent yet are alpha.
fn factors(blend: Blend) -> (u32, u32) {
    match blend {
        Blend::Invert => (glow::ONE_MINUS_DST_COLOR, glow::ZERO),
        Blend::Additive => (glow::SRC_ALPHA, glow::ONE),
        Blend::Vignette => (glow::ZERO, glow::ONE_MINUS_SRC_COLOR),
        Blend::Alpha | Blend::Depth(_) | Blend::EndPortal | Blend::CitGlint(_) => {
            (glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_are_alpha_and_the_blends_keep_their_order() {
        let runs = runs(10, &[(2..4, Blend::Additive), (6..7, Blend::Invert)]);
        assert_eq!(
            runs,
            vec![
                (0..2, Blend::Alpha),
                (2..4, Blend::Additive),
                (4..6, Blend::Alpha),
                (6..7, Blend::Invert),
                (7..10, Blend::Alpha),
            ]
        );
    }

    #[test]
    fn no_blends_is_one_alpha_run() {
        assert_eq!(runs(5, &[]), vec![(0..5, Blend::Alpha)]);
    }

    #[test]
    fn an_empty_batch_has_no_runs() {
        assert!(runs(0, &[]).is_empty());
    }

    #[test]
    fn a_blend_past_the_end_is_cut() {
        assert_eq!(
            runs(3, &[(2..9, Blend::Additive)]),
            vec![(0..2, Blend::Alpha), (2..3, Blend::Additive)]
        );
    }
}
