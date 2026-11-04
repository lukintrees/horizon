#version 110
uniform sampler2D image;
uniform sampler2D lightmap;
uniform sampler2D foil;
uniform float glint_strength;
uniform float gui_glint;
uniform float screen_mode;
varying vec2 foil_uv;
uniform float entity_mode;
uniform float mask_only;
uniform vec4 overlay;
varying vec4 back_tint;
varying vec4 vertex_light;
uniform float cutoff;
uniform float force_opaque;
uniform vec4 fog;
uniform float fog_end;
uniform vec4 game_fog;
varying vec2 uv;
varying vec2 light_uv;
varying vec4 tint;
varying float distance_to_eye;
varying float cylindrical_distance;
float linear_fog(float distance, float start, float end) {
    // Branch order matches terrain.wgsl, including start == end and MAX pairs.
    if (distance <= start) return 0.0;
    if (distance >= end) return 1.0;
    return (distance - start) / (end - start);
}
void main() {
    if (mask_only > 0.5) { gl_FragColor = vec4(0.0); return; }
    vec4 color = tint;
    if (entity_mode > 0.5 && !gl_FrontFacing) color = back_tint;
    vec4 sample_color = texture2D(image, uv);
    if (screen_mode > 0.5) {
        vec4 result = sample_color * tint;
        if (screen_mode > 1.5) result.a = 1.0;
        else if (result.a <= 0.0) discard;
        gl_FragColor = result;
        return;
    }
    if (gui_glint > 0.5) {
        // HUD glint is a separate, alpha-masked layer, NOT an entity composite.
        // Ignore vertex tint, lightmap and foil alpha, including strength zero.
        if (sample_color.a < 0.1) discard;
        vec3 g = glint_strength * texture2D(foil, foil_uv).rgb;
        gl_FragColor = vec4(g * g, sample_color.a);
        return;
    }
    vec4 pixel = sample_color * color;
    pixel.a = mix(pixel.a, 1.0, force_opaque);
    if (pixel.a <= 0.0 || pixel.a < cutoff) discard;
    if (entity_mode > 0.5) {
        pixel.rgb = mix(overlay.rgb, pixel.rgb, overlay.a);
        pixel *= vertex_light;
        if (glint_strength > 0.0) {
            vec3 g = glint_strength * texture2D(foil, foil_uv).rgb;
            pixel.rgb += g * g;
            pixel.a = max(pixel.a, glint_strength);
        }
    } else pixel.rgb *= texture2D(lightmap, light_uv).rgb;
    float amount = linear_fog(distance_to_eye, fog.a, fog_end);
    if (game_fog.w > 0.5) {
        amount = max(amount, linear_fog(cylindrical_distance, game_fog.x, game_fog.y)) * game_fog.z;
    }
    gl_FragColor = vec4(mix(pixel.rgb, fog.rgb, amount), pixel.a);
}
