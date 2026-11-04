// Terrain, vertex stage: the GLSL 1.20 port of horizon-client-render's terrain.wgsl.
//
// Differences from the WGSL, all forced by the target GPU (a Radeon X1650 on Catalyst 9.3):
//
//  * `#version 120`: `attribute`/`varying` instead of `@location`, no `uint`, no `in`/`out`,
//    no integer attributes at all. The mesher's `u16`/`u32` fields arrive as normalized
//    floats (a_color, a_light) or as one float the mesh sets per vertex (a_extra = which
//    atlas).
//  * sprite arrays are gone: the mesher resolves the layer into an atlas UV, so this stage
//    never samples and never needs the layer.
//  * the per-section origin is a uniform (`u_chunk_origin`), because 2.1 has no instancing
//    divisor here. It is subtracted in integer space before the float conversion, as the
//    WGSL does, so large world coordinates do not jitter in single precision.
//  * the four wgpu entry points (vanilla, flat, visbuffer, shadow) collapse to one `main`:
//    flat draws, the visibility buffer and shadow maps do not exist on this path
//    (docs/gl21-x1650-plan.ru.md §8.3).
//  * 512 uniform components: the WGSL's 936 become 48 here. Fog is fragment work in this
//    path, so those uniforms live in the fragment stage only; the arrays the WGSL carries
//    (dynamic lights, coloured lights, emissive layers, the sunrise fan) move to textures and
//    belong to passes that read them, not to terrain.
//
// Measured by tools/gl21_glsl.py: 35 uniform components, 10 varying floats, 5
// attributes, 0 samplers. Twelve times under the uniform budget, four times under varyings.

uniform mat4 u_view_proj;
uniform mat4 u_inverse_view;
// Section origin in blocks, relative to the camera (the WGSL's InstanceData.chunkOffset).
uniform vec3 u_chunk_origin;

attribute vec3 a_position;   // section-local blocks, 0..16
attribute vec2 a_texcoord;   // atlas UV
attribute vec4 a_color;      // tint * shade * AO, RGBA8 normalized
attribute vec2 a_light;      // sky, block, 0..1 normalized from the 0..15 levels
// x: which of the two atlases the vertex samples (0 or 1). Not a sprite layer: the mesher
// already turned the layer into a UV (texture.rs), so the shader has no array to index and
// only needs the atlas the UV range belongs to. A cutout is a property of the pass and lives
// in a uniform of the fragment stage, not here (gl/buffer.rs says what that saves).
attribute float a_extra;

varying vec2 v_uv;
varying vec4 v_color;
varying vec2 v_light;
varying float v_cylindrical;
varying float v_atlas;

void main() {
    // Integer-style subtraction before the float conversion: the origin is whole blocks, and
    // at a world coordinate of 30 million the difference is what keeps precision.
    vec3 world = floor(u_chunk_origin + 0.5) + a_position;
    vec3 camera_relative = world - u_chunk_origin;
    gl_Position = u_view_proj * vec4(camera_relative, 1.0);

    // Vanilla's cylindrical fog weight (FogRenderer): the horizontal distance from the eye.
    // The eye comes out of the inverse view matrix's translation, as in the WGSL; a 2.1
    // vertex shader has no `gl_CameraPosition`.
    vec3 eye = u_inverse_view[3].xyz;
    vec3 delta = world - eye;
    v_cylindrical = length(vec3(delta.x, 0.0, delta.z));

    v_uv = a_texcoord;
    v_color = a_color;
    v_light = a_light;
    v_atlas = a_extra;
}
