#version 120

// Solid terrain, cutout (alpha tested) and translucent faces, as GLSL 1.20.
//
// The port of `terrain.wgsl`'s vertex stage, minus what the GL 2.1 path leaves out (see
// `docs/opengl21-plan.md` §5.4): no sprite from an array (the sprite's atlas rectangle comes in
// `u_sprite`, one draw per sprite), no instance origin (a uniform per section), no fixed-point UV
// step or fade-in in the instance (`u_origin`, `u_fade`), no dynamic or coloured lights, no emissive
// overlays, no HDR.
//
// What is kept, because it changes what a block face looks like:
// - the position offset by the section's origin (camera-relative, as the game sends it);
// - the sprite-local UV scaled into the sprite's atlas rectangle (without the scale, every block
//   would show the sprite's top left texel);
// - the smooth light per vertex, which the fragment stage turns into the lightmap sample at the
//   same coordinates `terrain.wgsl` uses (`light / 16`, and 16 steps of 255 is not 256: the level is
//   `a_light / 255 * 16`);
// - the section's fade-in as a uniform, mixed toward the fog colour there.

uniform mat4 u_view_proj;
uniform vec3 u_origin;
uniform vec4 u_sprite;
uniform float u_fade;

attribute vec3 a_pos;
attribute vec2 a_uv;
attribute vec2 a_light;
attribute vec4 a_color;

varying vec2 v_uv;
varying vec4 v_color;
varying vec2 v_light;
varying float v_visibility;
varying float v_spherical;
varying float v_cylindrical;

void main() {
    // Emissive and cutout reads the same as on wgpu; the vertex colour is in gamma space.
    vec3 pos = a_pos + u_origin;

    v_uv = u_sprite.xy + a_uv * (u_sprite.zw - u_sprite.xy);
    v_color = a_color;
    // `a_light` is the mesh's light byte (`level * 16`) over 255, and `terrain.wgsl` reads the level
    // itself (`light.x / 16`): back to the level, sky in x and block in y.
    v_light = a_light * (255.0 / 16.0);
    v_visibility = u_fade;
    // Fog's two distances, as terrain.wgsl measures them (`spherical`, `cylindrical`).
    v_spherical = length(pos);
    v_cylindrical = max(length(pos.xz), abs(pos.y));

    gl_Position = u_view_proj * vec4(pos, 1.0);
}
