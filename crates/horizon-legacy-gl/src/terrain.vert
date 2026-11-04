#version 120
// Prototype terrain pipeline: CPU supplies a sprite rectangle for each quad.
attribute vec3 position;
attribute vec2 sprite_uv;
attribute vec4 atlas_rect;
attribute vec4 vertex_color;
uniform mat4 view_projection;
varying vec2 uv;
varying vec4 rect;
varying vec4 tint;
void main() {
    gl_Position = view_projection * vec4(position, 1.0);
    uv = sprite_uv;
    rect = atlas_rect;
    tint = vertex_color;
}
