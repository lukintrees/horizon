#version 120
uniform sampler2D atlas;
uniform float alpha_cutoff;
varying vec2 uv;
varying vec4 rect;
varying vec4 tint;
void main() {
    // Match terrain.wgsl's tile_uv, including the upper edge at integer UVs.
    vec2 tile = uv - max(ceil(uv) - vec2(1.0), vec2(0.0));
    vec4 color = texture2D(atlas, rect.xy + tile * rect.zw) * tint;
    if (color.a < alpha_cutoff) discard;
    gl_FragColor = color;
}
