#version 120

// The GL 2.1 path's present pass: the frame texture onto the window, GL_LINEAR and clamp-to-edge.
//
// GLSL 1.20: gl_FragColor, texture2D with explicit LOD 0 (the frame texture has no mips), no
// `texture()`. The blur (vanilla's menu background) samples the same texture through a second
// sampler object in the real renderer; GL 2.1 binds the filtering state with the texture, so that
// pass gets its own texture unit rather than a sampler uniform.

uniform sampler2D u_frame;
varying vec2 v_uv;

void main() {
    gl_FragColor = texture2D(u_frame, v_uv);
}
