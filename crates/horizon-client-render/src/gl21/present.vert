#version 120

// The GL 2.1 path's full-screen present: one quad of the window, sampling the frame Horizon drew at
// the resolution scale (the GL 2.1 equivalent of `world_post.wgsl`'s copy).
//
// GLSL 1.20: attribute/varying, texture2D, no layout qualifiers, no vertex ids. The quad's four
// corners come from a vertex buffer of [-1,-1]..[1,1] (gl21/renderer.rs), drawn with
// glDrawArrays(GL_TRIANGLE_STRIP).

attribute vec2 a_pos;
varying vec2 v_uv;

void main() {
    v_uv = a_pos * 0.5 + 0.5;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
