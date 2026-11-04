#version 120
varying vec3 v_dir;
uniform vec3 sky_top;
uniform vec3 sky_bottom;
void main() {
    float t = clamp(v_dir.y * 0.5 + 0.5, 0.0, 1.0);
    gl_FragColor = vec4(mix(sky_bottom, sky_top, t), 1.0);
}
