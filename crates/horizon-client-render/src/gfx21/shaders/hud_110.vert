#version 110
attribute vec2 position;
attribute vec2 uv;
attribute vec4 color;
uniform vec2 screen;
varying vec2 v_uv;
varying vec4 v_color;
void main() {
    vec2 ndc = position / screen * 2.0 - 1.0;
    v_uv = uv;
    v_color = color;
    gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}
