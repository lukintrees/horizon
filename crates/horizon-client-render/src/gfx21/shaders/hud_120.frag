#version 120
uniform sampler2D atlas;
varying vec2 v_uv;
varying vec4 v_color;
void main() {
    gl_FragColor = texture2D(atlas, v_uv) * v_color;
}
