#version 110
uniform sampler2D atlas;
uniform float alpha_cutoff;
uniform vec4 fog_color;
uniform vec4 fog_ranges;
uniform float fog_enabled;
varying vec2 uv;
varying vec4 rect;
varying vec4 tint;
varying float spherical_distance;
varying float cylindrical_distance;
float linear_fog(float d, float start, float end) {
    if (d <= start) return 0.0;
    if (d >= end) return 1.0;
    return (d - start) / (end - start);
}
void main() {
    // Match terrain.wgsl's tile_uv, including the upper edge at integer UVs.
    vec2 tile = uv - max(ceil(uv) - vec2(1.0), vec2(0.0));
    vec4 color = texture2D(atlas, rect.xy + tile * rect.zw) * tint;
    if (color.a < alpha_cutoff) discard;
    if (fog_enabled > 0.5) {
        float fog = max(linear_fog(spherical_distance, fog_ranges.x, fog_ranges.y),
                        linear_fog(cylindrical_distance, fog_ranges.z, fog_ranges.w));
        color.rgb = mix(color.rgb, fog_color.rgb, fog * fog_color.a);
    }
    gl_FragColor = color;
}
