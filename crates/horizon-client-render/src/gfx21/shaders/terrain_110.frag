#version 110
uniform sampler2D atlas;
uniform sampler2D lightmap;
uniform vec4 fog_color;
uniform float fog_amount;
varying vec2 v_uv;
varying vec4 v_color;
varying vec2 v_light_uv;
void main() {
    vec4 texel = texture2D(atlas, v_uv);
    vec4 light = texture2D(lightmap, v_light_uv);
    vec4 color = texel * v_color * light;
    gl_FragColor = vec4(mix(color.rgb, fog_color.rgb, fog_amount * fog_color.a), color.a);
}
