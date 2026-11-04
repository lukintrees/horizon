#version 110
attribute vec3 position;
attribute vec2 uv;
attribute vec4 color;
attribute vec2 light_uv;
uniform mat4 view_proj;
uniform vec3 origin;
varying vec2 v_uv;
varying vec4 v_color;
varying vec2 v_light_uv;
void main() {
    v_uv = uv;
    v_color = color;
    v_light_uv = light_uv;
    gl_Position = view_proj * vec4(position + origin, 1.0);
}
