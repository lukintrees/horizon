#version 110
attribute vec3 position;
attribute vec2 texcoord;
attribute vec2 lightcoord;
attribute vec4 color;
attribute vec4 back_color;
attribute vec2 foilcoord;
varying vec2 foil_uv;
attribute vec4 light_color;
varying vec4 back_tint;
varying vec4 vertex_light;
uniform mat4 matrix;
uniform vec3 origin;
varying vec2 uv;
varying vec2 light_uv;
varying vec4 tint;
varying float distance_to_eye;
varying float cylindrical_distance;
void main() {
    vec3 relative = position + origin;
    gl_Position = matrix * vec4(relative, 1.0);
    uv = texcoord;
    light_uv = lightcoord;
    tint = color;
    back_tint = back_color;
    foil_uv = foilcoord;
    vertex_light = light_color;
    distance_to_eye = length(relative);
    cylindrical_distance = max(length(relative.xz), abs(relative.y));
}
