#version 110
attribute vec3 position;
attribute vec2 texcoord;
attribute vec2 lightcoord;
attribute vec4 color;
uniform mat4 matrix;
uniform vec3 origin;
varying vec2 uv;
varying vec2 light_uv;
varying vec4 tint;
varying float distance_to_eye;
void main() {
    vec3 relative = position + origin;
    gl_Position = matrix * vec4(relative, 1.0);
    uv = texcoord;
    light_uv = lightcoord;
    tint = color;
    distance_to_eye = length(relative);
}
