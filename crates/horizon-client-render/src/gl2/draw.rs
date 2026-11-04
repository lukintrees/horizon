//! The OpenGL 2 terrain draw: the solid path of terrain.wgsl in GLSL 1.20 (the sprite's texel, the
//! mesher's tint, the lightmap by sky and block light, the fog, the alpha cut-out), the atlas pages as
//! 2D textures, and one vertex buffer per section. CI runs no GL 2 context, so this module is only
//! compiled there; the data it draws is checked by the tests of `atlas` and `batch`.

use glow::HasContext;

use super::atlas::{Atlas, PAGE};
use super::batch::{Batch, Run, Vertex};
use super::shader;

/// Vertex attributes, in the order of their locations. Matches `batch::Vertex`'s field layout.
const ATTRIBUTES: [&str; 7] = ["a_pos", "a_uv", "a_origin", "a_sprite", "a_light", "a_tint", "a_opaque"];
const SIZES: [i32; 7] = [3, 2, 2, 1, 2, 3, 1];
const OFFSETS: [i32; 7] = [0, 12, 20, 28, 32, 40, 52];
const STRIDE: i32 = std::mem::size_of::<Vertex>() as i32;

const VERTEX_SHADER: &str = r#"#version 120
attribute vec3 a_pos;
attribute vec2 a_uv;
attribute vec2 a_origin;
attribute float a_sprite;
attribute vec2 a_light;
attribute vec3 a_tint;
attribute float a_opaque;
uniform mat4 u_view_proj;
uniform vec3 u_section_origin;
varying vec2 v_uv;
varying vec2 v_origin;
varying float v_sprite;
varying vec2 v_light;
varying vec3 v_tint;
varying float v_opaque;
varying float v_spherical;
varying float v_cylindrical;
void main() {
    vec3 pos = a_pos + u_section_origin;
    gl_Position = u_view_proj * vec4(pos, 1.0);
    v_uv = a_uv;
    v_origin = a_origin;
    v_sprite = a_sprite;
    v_light = a_light;
    v_tint = a_tint;
    v_opaque = a_opaque;
    v_spherical = length(pos);
    v_cylindrical = length(pos.xz);
}
"#;

const FRAGMENT_SHADER: &str = r#"#version 120
uniform sampler2D u_page;
uniform sampler2D u_lightmap;
uniform float u_page_size;
uniform float u_blend;
uniform vec3 u_fog_color;
uniform vec4 u_fog;
varying vec2 v_uv;
varying vec2 v_origin;
varying float v_sprite;
varying vec2 v_light;
varying vec3 v_tint;
varying float v_opaque;
varying float v_spherical;
varying float v_cylindrical;
// fog: linear_fog in terrain.wgsl. The fog is the larger of the spherical and the cylindrical term.
float linear_fog(float d, float start, float end) {
    if (d <= start) { return 0.0; }
    if (d >= end) { return 1.0; }
    return (d - start) / (end - start);
}
void main() {
    // The texel of the sprite at this point: the sprite repeats every whole unit of uv.
    vec2 cell = floor(fract(v_uv) * v_sprite);
    vec4 texel = texture2D(u_page, (v_origin + cell + 0.5) / u_page_size);
    // The solid pass cuts out the texels under half alpha; the translucent pass (u_blend 1) keeps them
    // and blends with the texel's alpha.
    if (u_blend < 0.5 && texel.a < 0.5 && v_opaque < 0.5) {
        discard;
    }
    // sample_lightmap in terrain.wgsl: x is block light, y is sky light, 16 texels per level.
    vec2 light = clamp(vec2(v_light.y, v_light.x) * 16.0 / 256.0 + 0.5 / 16.0,
                       vec2(0.5 / 16.0), vec2(15.5 / 16.0));
    vec3 lit = texel.rgb * v_tint * texture2D(u_lightmap, light).rgb;
    float fogged = max(linear_fog(v_spherical, u_fog.x, u_fog.y),
                       linear_fog(v_cylindrical, u_fog.z, u_fog.w));
    float alpha = u_blend > 0.5 ? texel.a : 1.0;
    gl_FragColor = vec4(mix(lit, u_fog_color, fogged), alpha);
}
"#;

/// The fog of a frame, as terrain.wgsl's `Globals`: `range` is `[spherical start, spherical end,
/// cylindrical start, cylindrical end]` in blocks from the eye, and the colour the fog turns into.
#[derive(Clone, Copy, Debug)]
pub struct Fog {
    pub range: [f32; 4],
    pub color: [f32; 3],
}

/// A section's vertices on the GPU, with the runs that draw them.
pub struct Section {
    buffer: glow::Buffer,
    runs: Vec<Run>,
}

/// The terrain's program, its atlas pages and the lightmap, made once per context.
pub struct Terrain {
    program: glow::Program,
    pages: Vec<glow::Texture>,
    lightmap: glow::Texture,
}

impl Terrain {
    /// Builds the program and uploads the atlas pages and the 16x16 lightmap (`lightmap_pixels` in
    /// the renderer: RGBA rows of sky level by block level). Needs a current context.
    pub fn new(gl: &glow::Context, atlas: &Atlas, lightmap_pixels: &[u8; 16 * 16 * 4]) -> Result<Self, String> {
        let program = shader::program(gl, VERTEX_SHADER, FRAGMENT_SHADER, &ATTRIBUTES)?;
        let mut pages = Vec::with_capacity(atlas.pages.len());
        for page in &atlas.pages {
            pages.push(upload_page(gl, page)?);
        }
        let lightmap = upload_lightmap(gl, lightmap_pixels)?;
        Ok(Self { program, pages, lightmap })
    }

    /// Uploads a section's batch into a new vertex buffer.
    pub fn upload(&self, gl: &glow::Context, batch: &Batch) -> Result<Section, String> {
        // SAFETY: the context is current on this thread, as `new` requires.
        unsafe {
            let buffer = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck::cast_slice(&batch.vertices), glow::STATIC_DRAW);
            Ok(Section { buffer, runs: batch.runs.clone() })
        }
    }

    /// Frees a section's buffer.
    pub fn free_section(&self, gl: &glow::Context, section: Section) {
        // SAFETY: the buffer was made on this context by `upload`.
        unsafe { gl.delete_buffer(section.buffer) };
    }

    /// The lightmap texture, which the weather samples too.
    pub fn lightmap(&self) -> glow::Texture {
        self.lightmap
    }

    /// Replaces the lightmap's texels (RGBA, 16 by 16).
    pub fn set_lightmap(&self, gl: &glow::Context, pixels: &[u8; 16 * 16 * 4]) {
        // SAFETY: the context is current on this thread and the lightmap was made on it.
        unsafe {
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.lightmap));
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                16,
                16,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(pixels.as_slice())),
            );
            gl.active_texture(glow::TEXTURE0);
        }
    }

    /// Starts a frame: the program, the camera's clip matrix (column-major), the fog, the page and
    /// lightmap units. Then `draw` each section.
    pub fn begin(&self, gl: &glow::Context, view_proj: &[f32; 16], fog: Fog) {
        // SAFETY: the context is current on this thread and every handle was made on it.
        unsafe {
            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(self.program, "u_view_proj").as_ref(), false, view_proj);
            gl.uniform_4_f32(
                gl.get_uniform_location(self.program, "u_fog").as_ref(),
                fog.range[0],
                fog.range[1],
                fog.range[2],
                fog.range[3],
            );
            gl.uniform_3_f32(gl.get_uniform_location(self.program, "u_fog_color").as_ref(), fog.color[0], fog.color[1], fog.color[2]);
            gl.uniform_1_f32(gl.get_uniform_location(self.program, "u_page_size").as_ref(), PAGE as f32);
            gl.uniform_1_f32(gl.get_uniform_location(self.program, "u_blend").as_ref(), 0.0);
            gl.uniform_1_i32(gl.get_uniform_location(self.program, "u_page").as_ref(), 0);
            gl.uniform_1_i32(gl.get_uniform_location(self.program, "u_lightmap").as_ref(), 1);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.lightmap));
            gl.active_texture(glow::TEXTURE0);
        }
    }

    /// Switches between the solid pass (`false`) and the translucent pass (`true`): the cut-out is
    /// off and the alpha of the texel is kept. The caller sets the blend state.
    pub fn set_translucent(&self, gl: &glow::Context, on: bool) {
        // SAFETY: the context is current on this thread and the program was made on it.
        unsafe {
            gl.use_program(Some(self.program));
            gl.uniform_1_f32(gl.get_uniform_location(self.program, "u_blend").as_ref(), if on { 1.0 } else { 0.0 });
        }
    }

    /// Draws one section, its origin camera-relative in blocks, one draw per page run.
    pub fn draw(&self, gl: &glow::Context, section: &Section, origin: [f32; 3]) {
        // SAFETY: the context is current and `begin` set the program; the buffer is the section's.
        unsafe {
            gl.uniform_3_f32(gl.get_uniform_location(self.program, "u_section_origin").as_ref(), origin[0], origin[1], origin[2]);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(section.buffer));
            for (index, (&size, &offset)) in SIZES.iter().zip(OFFSETS.iter()).enumerate() {
                gl.enable_vertex_attrib_array(index as u32);
                gl.vertex_attrib_pointer_f32(index as u32, size, glow::FLOAT, false, STRIDE, offset);
            }
            for run in &section.runs {
                let Some(&page) = self.pages.get(run.page as usize) else {
                    continue;
                };
                gl.bind_texture(glow::TEXTURE_2D, Some(page));
                gl.draw_arrays(glow::TRIANGLES, run.first as i32, run.count as i32);
            }
            for index in 0..ATTRIBUTES.len() {
                gl.disable_vertex_attrib_array(index as u32);
            }
        }
    }

    /// Frees the program, the pages and the lightmap. The context must be current.
    pub fn free(&self, gl: &glow::Context) {
        // SAFETY: every handle was made on this context by `new`.
        unsafe {
            gl.delete_program(self.program);
            for &page in &self.pages {
                gl.delete_texture(page);
            }
            gl.delete_texture(self.lightmap);
        }
    }
}

fn upload_page(gl: &glow::Context, texels: &[u8]) -> Result<glow::Texture, String> {
    // SAFETY: the context is current on this thread, as `Terrain::new` requires.
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
            PAGE as i32,
            PAGE as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(texels)),
        );
        Ok(texture)
    }
}

fn upload_lightmap(gl: &glow::Context, texels: &[u8; 16 * 16 * 4]) -> Result<glow::Texture, String> {
    // SAFETY: the context is current on this thread, as `Terrain::new` requires.
    unsafe {
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            16,
            16,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(texels.as_slice())),
        );
        Ok(texture)
    }
}
