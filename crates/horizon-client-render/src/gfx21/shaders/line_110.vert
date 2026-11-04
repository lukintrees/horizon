#version 110
attribute vec3 position;
attribute vec4 color;
uniform mat4 view_proj;
varying vec4 v_color;
void main() {
    v_color = color;
    gl_Position = view_proj * vec4(position, 1.0);
}
