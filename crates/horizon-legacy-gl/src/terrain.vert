#version 110
// Prototype terrain pipeline: CPU supplies a sprite rectangle for each quad.
attribute vec3 position;
attribute vec2 sprite_uv;
attribute vec4 atlas_rect;
attribute vec4 vertex_color;
uniform mat4 view_projection;
uniform vec3 section_offset;
varying vec2 uv;
varying vec4 rect;
varying vec4 tint;
varying float spherical_distance;
varying float cylindrical_distance;
void main() {
    gl_Position = view_projection * vec4(position, 1.0);
    uv = sprite_uv;
    rect = atlas_rect;
    tint = vertex_color;
    vec3 relative_position = position + section_offset;
    spherical_distance = length(relative_position);
    cylindrical_distance = max(length(relative_position.xz), abs(relative_position.y));
}
