#version 110
attribute vec3 position;
varying vec3 v_dir;
void main() {
    v_dir = position;
    gl_Position = vec4(position, 1.0);
}
