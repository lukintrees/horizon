#version 110
uniform sampler2D image;
uniform sampler2D lightmap;
uniform sampler2D foil;
uniform float glint_strength;
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
varying vec2 uv;
varying vec2 light_uv;
varying vec4 tint;
varying float distance_to_eye;
void main() {
    if (mask_only > 0.5) { gl_FragColor = vec4(0.0); return; }
    vec4 color = tint;
    if (entity_mode > 0.5 && !gl_FrontFacing) color = back_tint;
    vec4 pixel = texture2D(image, uv) * color;
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
    float amount = clamp((distance_to_eye - fog.a) / (fog_end - fog.a), 0.0, 1.0);
    gl_FragColor = vec4(mix(pixel.rgb, fog.rgb, amount), pixel.a);
}
