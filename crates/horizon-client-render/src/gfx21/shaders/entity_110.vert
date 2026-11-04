#version 110
attribute vec3 position;
attribute vec2 uv;
attribute vec4 color;
uniform mat4 model_view_proj;
varying vec2 v_uv;
varying vec4 v_color;
void main() {
    v_uv = uv;
    v_color = color;
    gl_Position = model_view_proj * vec4(position, 1.0);
}
