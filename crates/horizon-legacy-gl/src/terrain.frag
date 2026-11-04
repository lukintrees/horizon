#version 110
uniform sampler2D atlas;
uniform float alpha_cutoff;
uniform vec4 fog_color;
uniform vec4 fog_ranges;
uniform float fog_enabled;
uniform float entity_mode;
uniform float screen_uv_mode;
uniform float half_texel;
uniform vec4 entity_overlay;
varying vec3 entity_light;
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
    vec2 sample_uv = rect.xy + tile * rect.zw;
    if (screen_uv_mode > 1.5) {
        sample_uv = clamp(rect.xy + uv * rect.zw, rect.xy + vec2(half_texel),
                          rect.xy + rect.zw - vec2(half_texel));
    } else if (screen_uv_mode > 0.5) {
        sample_uv = rect.xy + fract(uv) * rect.zw;
    }
    vec4 color = texture2D(atlas, sample_uv) * tint;
    if (color.a < alpha_cutoff) discard;
    if (entity_mode > 0.5) {
        color.rgb = mix(entity_overlay.rgb, color.rgb, entity_overlay.a) * entity_light;
    }
    if (fog_enabled > 0.5) {
        float fog = max(linear_fog(spherical_distance, fog_ranges.x, fog_ranges.y),
                        linear_fog(cylindrical_distance, fog_ranges.z, fog_ranges.w));
        color.rgb = mix(color.rgb, fog_color.rgb, fog * fog_color.a);
    }
    gl_FragColor = color;
}
