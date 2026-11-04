//! The sky, the weather and the lightning of the OpenGL 2 path, drawn from the data the wgpu path draws
//! (`Frame::stars`, `celestial`, `clouds`, `weather` and `lightning`) in GLSL 1.20:
//!
//! - the stars: `gfx::stars`' static buffer, additive, before the world (they are at infinity);
//! - the sun and the moon (`celestial`), additive, before the world, sampling the shared atlas;
//! - the clouds: the cloud mesh's faces, alpha blended and faded towards the cloud end, after the solid
//!   terrain, which hides them where it is nearer;
//! - the lightning: the bolts' quads, after the terrain, depth tested, as their pipeline draws them;
//! - the rain and the snow: `weather`'s quads, alpha tested and lit, after the translucent terrain.
//!
//! Entity vertices keep their wgpu meaning: the atlas UV, the overlay in `layer`, the light as sky and
//! block bytes, and the cloud palette entry in the light's low byte. The weather's animation is worked
//! out on the CPU, since GLSL 1.20 has no 64-bit integers for `entity.wgsl` to emulate it with.

use glam::{DVec3, Mat4};
use glow::HasContext;

use super::draw::Fog;
use super::shader;
use crate::clouds;
use crate::gfx::{lightning, stars};
use crate::mesher::Vertex as EntityVertex;
use crate::weather::{self, Time};

/// The most vertices of the celestial bodies a frame draws: `Game::celestial_geometry`'s cap.
const MAX_BODY_VERTICES: usize = 2048 * 4;

/// One vertex of the sky's quads as the shaders read it: the entity vertex with its colour and light
/// unpacked, and the weather's sprite rectangle in the shared atlas.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
    /// `(block, sky)` light, 16 per level, as the lightmap is sampled by.
    light: [f32; 2],
    /// The overlay (`layer` & 0xffff): 37 is the untextured dragon ray, which fades with the fog.
    layer: f32,
    /// The cloud palette entry, from the light's low byte.
    index: f32,
    /// The weather sprite's rectangle `(u0, v0, width, height)` in the shared atlas.
    rect: [f32; 4],
}

/// The vertex attributes shared by the body, cloud and weather programs, in the order of their locations.
const ATTRIBUTES: [&str; 7] = [
    "a_pos", "a_uv", "a_color", "a_light", "a_layer", "a_index", "a_rect",
];
const SIZES: [i32; 7] = [3, 2, 4, 2, 1, 1, 4];
const OFFSETS: [i32; 7] = [
    std::mem::offset_of!(Vertex, pos) as i32,
    std::mem::offset_of!(Vertex, uv) as i32,
    std::mem::offset_of!(Vertex, color) as i32,
    std::mem::offset_of!(Vertex, light) as i32,
    std::mem::offset_of!(Vertex, layer) as i32,
    std::mem::offset_of!(Vertex, index) as i32,
    std::mem::offset_of!(Vertex, rect) as i32,
];
const STRIDE: i32 = std::mem::size_of::<Vertex>() as i32;

const STAR_VERTEX: &str = r#"#version 120
attribute vec3 a_pos;
uniform mat4 u_clip_from_sky;
void main() {
    gl_Position = u_clip_from_sky * vec4(a_pos, 1.0);
}
"#;

// starBrightness is in the colour's every channel and multiplies it once more (fs_stars).
const STAR_FRAGMENT: &str = r#"#version 120
uniform float u_brightness;
void main() {
    gl_FragColor = vec4(vec3(u_brightness * u_brightness), 1.0);
}
"#;

const BODY_VERTEX: &str = r#"#version 120
attribute vec3 a_pos;
attribute vec2 a_uv;
attribute vec4 a_color;
attribute float a_layer;
uniform mat4 u_view_proj;
varying vec2 v_uv;
varying vec4 v_color;
varying float v_layer;
varying float v_spherical;
varying float v_cylindrical;
void main() {
    gl_Position = u_view_proj * vec4(a_pos, 1.0);
    v_uv = a_uv;
    v_color = a_color;
    v_layer = a_layer;
    v_spherical = length(a_pos);
    v_cylindrical = max(length(a_pos.xz), abs(a_pos.y));
}
"#;

// celestial_color: the atlas texel times the colour, unlit; the untextured dragon ray (overlay 37) fades
// with the fog instead. Premultiplied for the sun and the moon (one, one); the bolts keep their alpha.
const BODY_FRAGMENT: &str = r#"#version 120
uniform sampler2D u_atlas;
uniform vec4 u_fog;
uniform float u_premultiplied;
varying vec2 v_uv;
varying vec4 v_color;
varying float v_layer;
varying float v_spherical;
varying float v_cylindrical;
float linear_fog(float d, float start, float end) {
    if (d <= start) { return 0.0; }
    if (d >= end) { return 1.0; }
    return (d - start) / (end - start);
}
void main() {
    vec4 c;
    if (abs(v_layer - 37.0) < 0.5) {
        float fog = max(linear_fog(v_spherical, u_fog.x, u_fog.y),
                        linear_fog(v_cylindrical, u_fog.z, u_fog.w));
        c = v_color * (1.0 - fog);
    } else {
        c = texture2D(u_atlas, v_uv) * v_color;
    }
    gl_FragColor = u_premultiplied > 0.5 ? vec4(c.rgb * c.a, 1.0) : c;
}
"#;

// vs_cloud: the cloud offset is added, and the colour is the palette entry the light's low byte names.
const CLOUD_VERTEX: &str = r#"#version 120
attribute vec3 a_pos;
attribute vec2 a_uv;
attribute float a_index;
uniform mat4 u_view_proj;
uniform vec4 u_offset;
uniform vec4 u_palette[4];
varying vec2 v_uv;
varying vec4 v_color;
varying float v_spherical;
void main() {
    vec3 pos = a_pos + u_offset.xyz;
    gl_Position = u_view_proj * vec4(pos, 1.0);
    v_uv = a_uv;
    if (a_index < 0.5) {
        v_color = u_palette[0];
    } else if (a_index < 1.5) {
        v_color = u_palette[1];
    } else if (a_index < 2.5) {
        v_color = u_palette[2];
    } else {
        v_color = u_palette[3];
    }
    v_spherical = length(pos);
}
"#;

// entity_color for overlay 4 (clouds.fsh): the alpha fades out towards the cloud fog end.
const CLOUD_FRAGMENT: &str = r#"#version 120
uniform sampler2D u_atlas;
uniform float u_cloud_end;
varying vec2 v_uv;
varying vec4 v_color;
varying float v_spherical;
float linear_fog(float d, float start, float end) {
    if (d <= start) { return 0.0; }
    if (d >= end) { return 1.0; }
    return (d - start) / (end - start);
}
void main() {
    vec4 c = texture2D(u_atlas, v_uv) * v_color;
    if (c.a < 0.1) { discard; }
    float fade = 1.0 - linear_fog(v_spherical, 0.0, u_cloud_end);
    gl_FragColor = vec4(c.rgb, c.a * fade);
}
"#;

// vs_weather: the phase is already in the UV. fs_weather: the sprite's texel, repeated within the sprite
// (nearest, no mipmaps), alpha tested, lit by the lightmap, then fogged.
const WEATHER_VERTEX: &str = r#"#version 120
attribute vec3 a_pos;
attribute vec2 a_uv;
attribute vec4 a_color;
attribute vec2 a_light;
attribute vec4 a_rect;
uniform mat4 u_view_proj;
varying vec2 v_uv;
varying vec4 v_color;
varying vec2 v_light;
varying vec4 v_rect;
varying float v_spherical;
varying float v_cylindrical;
void main() {
    gl_Position = u_view_proj * vec4(a_pos, 1.0);
    v_uv = a_uv;
    v_color = a_color;
    v_light = a_light;
    v_rect = a_rect;
    v_spherical = length(a_pos);
    v_cylindrical = max(length(a_pos.xz), abs(a_pos.y));
}
"#;

const WEATHER_FRAGMENT: &str = r#"#version 120
uniform sampler2D u_atlas;
uniform sampler2D u_lightmap;
uniform vec2 u_atlas_size;
uniform vec4 u_fog;
uniform vec3 u_fog_color;
varying vec2 v_uv;
varying vec4 v_color;
varying vec2 v_light;
varying vec4 v_rect;
varying float v_spherical;
varying float v_cylindrical;
float linear_fog(float d, float start, float end) {
    if (d <= start) { return 0.0; }
    if (d >= end) { return 1.0; }
    return (d - start) / (end - start);
}
void main() {
    vec2 origin = floor(v_rect.xy * u_atlas_size + 0.5);
    vec2 extent = max(floor(v_rect.zw * u_atlas_size + 0.5), vec2(1.0));
    vec2 local = min(floor(fract(v_uv) * extent), extent - vec2(1.0));
    vec4 c = texture2D(u_atlas, (origin + local + 0.5) / u_atlas_size) * v_color;
    if (c.a < 0.1) { discard; }
    vec2 light = clamp(v_light / 256.0 + vec2(0.5 / 16.0), vec2(0.5 / 16.0), vec2(15.5 / 16.0));
    c *= texture2D(u_lightmap, light);
    float fogged = max(linear_fog(v_spherical, u_fog.x, u_fog.y),
                       linear_fog(v_cylindrical, u_fog.z, u_fog.w));
    gl_FragColor = vec4(mix(c.rgb, u_fog_color, fogged), c.a);
}
"#;

/// What the sky of a frame draws, as the game gives it to the renderer.
pub struct SkyScene<'a> {
    /// The sky's rotation and the star brightness, when the stars show.
    pub stars: Option<(Mat4, f32)>,
    /// The sun's and the moon's quads, in the sky's space.
    pub celestial: &'a [EntityVertex],
    pub clouds: Option<&'a clouds::Input<'a>>,
    /// The distance at which the clouds fade out.
    pub cloud_end: f32,
    pub weather: &'a [weather::Vertex],
    pub weather_time: Time,
    pub lightning: Option<&'a lightning::Input<'a>>,
}

#[cfg(test)]
impl SkyScene<'_> {
    /// A frame with nothing in its sky.
    pub fn nothing<'a>() -> SkyScene<'a> {
        SkyScene {
            stars: None,
            celestial: &[],
            clouds: None,
            cloud_end: 2048.0,
            weather: &[],
            weather_time: Time::new(0, 0.0),
            lightning: None,
        }
    }
}

/// The sky's programs and buffers, made once per context. The stars never change, so they are built
/// into one buffer; the other quads are uploaded for each draw.
pub struct Sky {
    star_program: glow::Program,
    star_buffer: glow::Buffer,
    star_count: i32,
    body_program: glow::Program,
    cloud_program: glow::Program,
    weather_program: glow::Program,
    dynamic: glow::Buffer,
    cloud_buffer: glow::Buffer,
    cloud_count: usize,
    /// The shared atlas, which the HUD also samples; its texels are the sky's sprites.
    atlas: glow::Texture,
    atlas_size: [f32; 2],
}

impl Sky {
    /// Builds the programs and the star buffer. `atlas` is the shared atlas of `atlas_size` texels, as
    /// the HUD made it. Needs a current context.
    pub fn new(
        gl: &glow::Context,
        atlas: glow::Texture,
        atlas_size: [u32; 2],
    ) -> Result<Self, String> {
        let star_program = shader::program(gl, STAR_VERTEX, STAR_FRAGMENT, &["a_pos"])?;
        let body_program = shader::program(gl, BODY_VERTEX, BODY_FRAGMENT, &ATTRIBUTES)?;
        let cloud_program = shader::program(gl, CLOUD_VERTEX, CLOUD_FRAGMENT, &ATTRIBUTES)?;
        let weather_program = shader::program(gl, WEATHER_VERTEX, WEATHER_FRAGMENT, &ATTRIBUTES)?;
        let star_vertices = stars::star_triangles();
        // SAFETY: the context is current on this thread, as the caller requires.
        unsafe {
            let star_buffer = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(star_buffer));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&star_vertices),
                glow::STATIC_DRAW,
            );
            Ok(Self {
                star_program,
                star_buffer,
                star_count: star_vertices.len() as i32,
                body_program,
                cloud_program,
                weather_program,
                dynamic: gl.create_buffer()?,
                cloud_buffer: gl.create_buffer()?,
                cloud_count: 0,
                atlas,
                atlas_size: [atlas_size[0] as f32, atlas_size[1] as f32],
            })
        }
    }

    /// Frees the programs and buffers. The context must be current.
    pub fn free(&self, gl: &glow::Context) {
        // SAFETY: every handle was made on this context by `new`.
        unsafe {
            gl.delete_program(self.star_program);
            gl.delete_program(self.body_program);
            gl.delete_program(self.cloud_program);
            gl.delete_program(self.weather_program);
            gl.delete_buffer(self.star_buffer);
            gl.delete_buffer(self.dynamic);
            gl.delete_buffer(self.cloud_buffer);
        }
    }

    /// Draws the stars, additively, with `clip` (the frame's clip matrix turned by the sky's rotation) and
    /// their brightness. Before the world: the stars are at infinity.
    pub fn stars(&self, gl: &glow::Context, clip: Mat4, brightness: f32) {
        let brightness = brightness.clamp(0.0, 1.0);
        state(gl, Some((glow::ONE, glow::ONE)), false, false);
        // SAFETY: the context is current and the programs and buffers were made on it.
        unsafe {
            gl.use_program(Some(self.star_program));
            gl.uniform_matrix_4_f32_slice(
                location(gl, self.star_program, "u_clip_from_sky").as_ref(),
                false,
                &clip.to_cols_array(),
            );
            gl.uniform_1_f32(
                location(gl, self.star_program, "u_brightness").as_ref(),
                brightness,
            );
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.star_buffer));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);
            gl.draw_arrays(glow::TRIANGLES, 0, self.star_count);
            gl.disable_vertex_attrib_array(0);
        }
        state(gl, None, true, true);
    }

    /// Draws the sun and the moon, or with `lightning` the bolts, from their quads: the atlas texel times
    /// the vertex colour, unlit. The bodies go additively before the world; the bolts after it, depth
    /// tested, as their pipeline draws them.
    pub fn bodies(
        &self,
        gl: &glow::Context,
        clip: Mat4,
        vertices: &[EntityVertex],
        fog: &Fog,
        lightning: bool,
    ) {
        let count = vertices.len().min(MAX_BODY_VERTICES);
        let converted: Vec<Vertex> = vertices[..count].iter().map(entity_vertex).collect();
        let triangles = as_triangles(&converted);
        if triangles.is_empty() {
            return;
        }
        upload(gl, self.dynamic, &triangles);
        let program = self.body_program;
        // SAFETY: the context is current and the program was made on it.
        unsafe {
            gl.use_program(Some(program));
            gl.uniform_matrix_4_f32_slice(
                location(gl, program, "u_view_proj").as_ref(),
                false,
                &clip.to_cols_array(),
            );
            gl.uniform_1_f32(
                location(gl, program, "u_premultiplied").as_ref(),
                if lightning { 0.0 } else { 1.0 },
            );
        }
        set_fog(gl, program, fog);
        set_samplers(gl, program);
        bind_atlas(gl, self.atlas);
        // The bolts blend with their alpha, as the additive entity pipeline does; the bodies
        // add their premultiplied colour.
        let blend = if lightning {
            (glow::SRC_ALPHA, glow::ONE)
        } else {
            (glow::ONE, glow::ONE)
        };
        state(gl, Some(blend), lightning, false);
        draw_vertices(gl, self.dynamic, triangles.len());
        state(gl, None, true, true);
    }

    /// Draws the clouds from the cloud mesh's quads (`vertices`, re-uploaded when `rebuilt`) with the
    /// cloud uniform's offset and palette. Alpha blended over the solid terrain, which hides them where it
    /// is nearer, and faded out towards `cloud_end`.
    pub fn clouds(
        &mut self,
        gl: &glow::Context,
        clip: Mat4,
        vertices: &[EntityVertex],
        uniform: &clouds::Uniform,
        cloud_end: f32,
        rebuilt: bool,
    ) {
        if rebuilt {
            let converted: Vec<Vertex> = vertices.iter().map(entity_vertex).collect();
            let triangles = as_triangles(&converted);
            upload(gl, self.cloud_buffer, &triangles);
            self.cloud_count = triangles.len();
        }
        if self.cloud_count == 0 {
            return;
        }
        let program = self.cloud_program;
        let palette: Vec<f32> = uniform.colors.iter().flatten().copied().collect();
        // SAFETY: the context is current and the program was made on it.
        unsafe {
            gl.use_program(Some(program));
            gl.uniform_matrix_4_f32_slice(
                location(gl, program, "u_view_proj").as_ref(),
                false,
                &clip.to_cols_array(),
            );
            let offset = uniform.offset;
            gl.uniform_4_f32(
                location(gl, program, "u_offset").as_ref(),
                offset[0],
                offset[1],
                offset[2],
                offset[3],
            );
            gl.uniform_4_f32_slice(location(gl, program, "u_palette").as_ref(), &palette);
            gl.uniform_1_f32(location(gl, program, "u_cloud_end").as_ref(), cloud_end);
        }
        set_samplers(gl, program);
        bind_atlas(gl, self.atlas);
        state(
            gl,
            Some((glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA)),
            true,
            false,
        );
        draw_vertices(gl, self.cloud_buffer, self.cloud_count);
        state(gl, None, true, true);
    }

    /// Draws the rain and the snow from `vertices`, with each vertex's animation worked out here. Alpha
    /// tested and lit by the lightmap, faded into the fog, after the translucent terrain.
    pub fn weather(
        &self,
        gl: &glow::Context,
        clip: Mat4,
        lightmap: glow::Texture,
        vertices: &[weather::Vertex],
        time: Time,
        fog: &Fog,
    ) {
        let converted: Vec<Vertex> = vertices
            .iter()
            .map(|vertex| weather_vertex(vertex, time.values))
            .collect();
        let triangles = as_triangles(&converted);
        if triangles.is_empty() {
            return;
        }
        upload(gl, self.dynamic, &triangles);
        let program = self.weather_program;
        // SAFETY: the context is current and the program was made on it.
        unsafe {
            gl.use_program(Some(program));
            gl.uniform_matrix_4_f32_slice(
                location(gl, program, "u_view_proj").as_ref(),
                false,
                &clip.to_cols_array(),
            );
            gl.uniform_2_f32(
                location(gl, program, "u_atlas_size").as_ref(),
                self.atlas_size[0],
                self.atlas_size[1],
            );
        }
        set_fog(gl, program, fog);
        set_samplers(gl, program);
        // SAFETY: the context is current; the lightmap is the terrain's, on unit 1.
        unsafe {
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(lightmap));
        }
        bind_atlas(gl, self.atlas);
        state(
            gl,
            Some((glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA)),
            true,
            false,
        );
        draw_vertices(gl, self.dynamic, triangles.len());
        state(gl, None, true, true);
    }
}

/// The bolts' quads at their places relative to the camera: `LightningBoltRenderer`'s offsets, added
/// to the quads of `gfx::lightning::geometry`.
pub fn lightning_vertices(input: &lightning::Input<'_>, camera: DVec3) -> Vec<EntityVertex> {
    let mut out = Vec::new();
    for bolt in input.bolts {
        let [x, y, z] = lightning::offset(bolt, camera);
        for mut vertex in lightning::geometry(bolt.seed, input.uv) {
            vertex.pos = [vertex.pos[0] + x, vertex.pos[1] + y, vertex.pos[2] + z];
            out.push(vertex);
        }
    }
    out
}

/// The blend, the depth test and the depth writes of the next draws. The caller sets the program.
/// `state(.., None, true, true)` is the default the world expects after the sky.
fn state(gl: &glow::Context, blend: Option<(u32, u32)>, depth_test: bool, depth_write: bool) {
    // SAFETY: the context is current on this thread, as every draw of the sky requires.
    unsafe {
        match blend {
            Some((source, destination)) => {
                gl.enable(glow::BLEND);
                gl.blend_func(source, destination);
            }
            None => gl.disable(glow::BLEND),
        }
        if depth_test {
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::GEQUAL);
        } else {
            gl.disable(glow::DEPTH_TEST);
        }
        gl.depth_mask(depth_write);
    }
}

/// The location of a uniform, or `None` when the program does not use it.
fn location(
    gl: &glow::Context,
    program: glow::Program,
    name: &str,
) -> Option<glow::UniformLocation> {
    // SAFETY: the context is current and `program` was linked on it.
    unsafe { gl.get_uniform_location(program, name) }
}

/// The fog of a program: its range and the colour it turns into.
fn set_fog(gl: &glow::Context, program: glow::Program, fog: &Fog) {
    // SAFETY: the context is current and `program` is in use.
    unsafe {
        gl.uniform_4_f32(
            location(gl, program, "u_fog").as_ref(),
            fog.range[0],
            fog.range[1],
            fog.range[2],
            fog.range[3],
        );
        gl.uniform_3_f32(
            location(gl, program, "u_fog_color").as_ref(),
            fog.color[0],
            fog.color[1],
            fog.color[2],
        );
    }
}

/// The texture units of a program: the atlas on unit 0, the lightmap on unit 1.
fn set_samplers(gl: &glow::Context, program: glow::Program) {
    // SAFETY: the context is current and `program` is in use.
    unsafe {
        gl.uniform_1_i32(location(gl, program, "u_atlas").as_ref(), 0);
        gl.uniform_1_i32(location(gl, program, "u_lightmap").as_ref(), 1);
    }
}

fn bind_atlas(gl: &glow::Context, atlas: glow::Texture) {
    // SAFETY: the context is current and the atlas was made on it.
    unsafe {
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(atlas));
    }
}

/// Uploads `vertices` into `buffer` for the draw that follows.
fn upload(gl: &glow::Context, buffer: glow::Buffer, vertices: &[Vertex]) {
    // SAFETY: the context is current and `buffer` was made on it.
    unsafe {
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            bytemuck::cast_slice(vertices),
            glow::STREAM_DRAW,
        );
    }
}

/// Draws `count` vertices of `buffer` as triangles, with every attribute bound and unbound around it.
fn draw_vertices(gl: &glow::Context, buffer: glow::Buffer, count: usize) {
    // SAFETY: the context is current, the program is in use and `buffer` holds `count` vertices.
    unsafe {
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        for (index, (&size, &offset)) in SIZES.iter().zip(OFFSETS.iter()).enumerate() {
            gl.enable_vertex_attrib_array(index as u32);
            gl.vertex_attrib_pointer_f32(index as u32, size, glow::FLOAT, false, STRIDE, offset);
        }
        gl.draw_arrays(glow::TRIANGLES, 0, count as i32);
        for index in 0..ATTRIBUTES.len() {
            gl.disable_vertex_attrib_array(index as u32);
        }
    }
}

/// The entity vertex as the sky's shaders read it.
fn entity_vertex(vertex: &EntityVertex) -> Vertex {
    Vertex {
        pos: vertex.pos,
        uv: vertex.uv,
        color: unpack_color(vertex.color),
        light: light_levels(vertex.light),
        layer: (vertex.layer & 0xffff) as f32,
        index: (vertex.light & 0xff) as f32,
        rect: [0.0; 4],
    }
}

/// A weather quad's vertex at this moment: its UV moved by the phase of its animation.
fn weather_vertex(vertex: &weather::Vertex, time: [f32; 4]) -> Vertex {
    let phase = weather_phase(vertex.animation, vertex.bases, vertex.snow != 0, time);
    Vertex {
        pos: vertex.pos,
        uv: [vertex.uv[0] + phase[0], vertex.uv[1] + phase[1]],
        color: unpack_color(vertex.color),
        light: light_levels(vertex.light),
        layer: 0.0,
        index: 0.0,
        rect: vertex.rect,
    }
}

/// The phase of a weather vertex, `weather_phase` in `entity.wgsl`. Rain scrolls down its column by the
/// ticks; snow drifts by its random base, which is added as a double, and falls.
pub fn weather_phase(animation: [f32; 4], bases: [u32; 4], snow: bool, time: [f32; 4]) -> [f32; 2] {
    if !snow {
        let ticks = time[0] + animation[1];
        let t = ticks + time[2];
        let step = -t / 32.0;
        let offset = step * animation[0];
        return [0.0, offset % 32.0];
    }
    let t = time[0] + time[2];
    let u_time = t * 0.01;
    let du = u_time * animation[2];
    let v_time = t * animation[3];
    let dv = v_time * 0.001;
    let u = double_plus_float([bases[0], bases[1]], du);
    let v = double_plus_float([bases[2], bases[3]], dv);
    let fall_time = time[1] + time[2];
    let fall = -fall_time / 512.0;
    [u, fall + v]
}

/// A random base (a double, low word first) plus a float, rounded to a float once: vanilla first adds
/// them as doubles, then casts.
fn double_plus_float(base: [u32; 2], delta: f32) -> f32 {
    let bits = u64::from(base[0]) | (u64::from(base[1]) << 32);
    (f64::from_bits(bits) + f64::from(delta)) as f32
}

/// A packed RGBA8 colour, its low byte red, as the vertex attribute reads it.
fn unpack_color(packed: u32) -> [f32; 4] {
    packed.to_le_bytes().map(|byte| f32::from(byte) / 255.0)
}

/// The block and sky light of a packed light word: sky in bits 0..8, block in bits 8..16, 16 per level,
/// as `(block, sky)`, the order the lightmap is sampled in.
fn light_levels(packed: u32) -> [f32; 2] {
    [((packed >> 8) & 0xff) as f32, (packed & 0xff) as f32]
}

/// Each quad (four vertices in order around it) as two triangles.
fn as_triangles<T: Copy>(quads: &[T]) -> Vec<T> {
    let mut out = Vec::with_capacity(quads.len() / 4 * 6);
    for quad in quads.chunks_exact(4) {
        out.extend_from_slice(&[quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rain_falls_by_the_ticks_and_the_speed() {
        // 64 ticks at speed 1: the step is -2 of the 32-block column.
        let phase = weather_phase([1.0, 0.0, 0.0, 0.0], [0; 4], false, [64.0, 0.0, 0.0, 0.0]);
        assert_eq!(phase, [0.0, -2.0]);
    }

    #[test]
    fn the_snow_drifts_by_its_speed_and_falls_by_its_time() {
        // The bases are zero, so the drift is the delta: 100 ticks at speed 1 move u by one sprite.
        let phase = weather_phase([0.0, 0.0, 1.0, 0.0], [0; 4], true, [100.0, 0.0, 0.0, 0.0]);
        assert!((phase[0] - 1.0).abs() < 1e-6, "{phase:?}");
        assert_eq!(phase[1], 0.0);
    }

    #[test]
    fn a_base_is_added_as_a_double_and_rounded_once() {
        let one = 1.0f64.to_bits();
        let base = [one as u32, (one >> 32) as u32];
        assert_eq!(double_plus_float(base, 0.5), 1.5);
        assert_eq!(double_plus_float([0, 0], -0.0), 0.0);
    }

    #[test]
    fn colours_unpack_their_red_from_the_low_byte() {
        assert_eq!(
            unpack_color(0x8040_20FF),
            [1.0, 32.0 / 255.0, 64.0 / 255.0, 128.0 / 255.0],
        );
    }

    #[test]
    fn light_is_block_then_sky() {
        // Sky in the low byte (48), block in the high byte (15): the lightmap's (block, sky).
        assert_eq!(light_levels(0x0F30), [15.0, 48.0]);
    }

    #[test]
    fn each_quad_becomes_two_triangles_in_its_order() {
        let quads = [1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(as_triangles(&quads), [1, 2, 3, 1, 3, 4, 5, 6, 7, 5, 7, 8]);
    }

    #[test]
    fn the_stars_are_whole_quads() {
        let count = stars::star_triangles().len();
        assert_eq!(count % 6, 0, "two triangles per star");
        assert!((500 * 6..=1500 * 6).contains(&count), "{count} vertices");
    }
}
