#version 110
uniform sampler2D image;
uniform sampler2D lightmap;
uniform float cutoff;
uniform vec4 fog;
uniform float fog_end;
varying vec2 uv;
varying vec2 light_uv;
varying vec4 tint;
varying float distance_to_eye;
void main() {
    vec4 pixel = texture2D(image, uv) * tint;
    if (pixel.a < cutoff) discard;
    pixel.rgb *= texture2D(lightmap, light_uv).rgb;
    float amount = clamp((distance_to_eye - fog.a) / (fog_end - fog.a), 0.0, 1.0);
    gl_FragColor = vec4(mix(pixel.rgb, fog.rgb, amount), pixel.a);
}
