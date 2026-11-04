#version 110
// Experimental level-zero atlas path. Cutout threshold is set by the caller;
// use zero for opaque or translucent geometry, and blending for translucent.
uniform sampler2D u_atlas;
uniform sampler2D u_lightmap;
uniform float u_alpha_cutoff;
varying vec2 v_uv;
varying vec2 v_light_uv;
varying vec4 v_color;
void main() {
    vec4 texel = texture2D(u_atlas, v_uv);
    if (texel.a < u_alpha_cutoff) discard;
    gl_FragColor = texel * v_color * texture2D(u_lightmap, v_light_uv);
}
