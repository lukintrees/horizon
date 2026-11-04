#version 110
// Experimental terrain path: positions are section-local; origin is camera-relative.
attribute vec3 a_position;
attribute vec2 a_uv;
attribute vec2 a_light_uv;
attribute vec4 a_color;
uniform mat4 u_view_projection;
uniform vec3 u_section_origin;
varying vec2 v_uv;
varying vec2 v_light_uv;
varying vec4 v_color;
void main() {
    gl_Position = u_view_projection * vec4(a_position + u_section_origin, 1.0);
    v_uv = a_uv;
    v_light_uv = a_light_uv;
    v_color = a_color;
}
