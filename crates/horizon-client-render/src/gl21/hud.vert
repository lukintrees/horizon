#version 120

// The GL 2.1 path's HUD vertex stage: the port of `hud.wgsl`'s `vs_main` to GLSL 1.20.
//
// GUI pixels to the window, exactly as `hud.wgsl` does it: the vertex is multiplied by the GUI scale
// (`u_scale`), then a window pixel becomes clip space with a y flip. `hud.aspect` decides the y
// direction: 1 keeps the flip the WGSL stage has (`1 - y / height * 2`), -1 is the same expression
// with the sign of the height negated, which is how the same source draws into a framebuffer whose
// origin is the bottom left (GL 2.1 without `GL_ARB_framebuffer_object`'s flip).
//
// `u_glint` carries the repeating glint texture's scale and time; the fragment stage uses it, the
// vertex stage passes the glint UV through untouched.

uniform vec4 u_hud; // x,y: window in pixels; z: GUI scale; w: the glint texture's time.
uniform vec2 u_aspect; // x: the y direction (1 or -1); y: unused.

attribute vec2 a_pos;
attribute vec2 a_uv;
attribute vec4 a_color;
attribute float a_depth;
attribute vec2 a_glint_uv;

varying vec2 v_uv;
varying vec4 v_color;
varying vec4 v_proj;
varying vec2 v_glint_uv;

void main() {
    vec2 pixels = a_pos * u_hud.z;
    // GLSL 1.20 has no `y / w * 2 - 1` on a swizzle, so the division is written out.
    float x = pixels.x / u_hud.x * 2.0 - 1.0;
    float y = 1.0 - pixels.y / u_hud.y * 2.0;
    gl_Position = vec4(x, y * u_aspect.x, a_depth, 1.0);
    v_uv = a_uv;
    v_color = a_color;
    // projection_from_position(gl_Position): the End portal's texProj0.
    v_proj = vec4(gl_Position.xy * 0.5 + 0.5, gl_Position.zw);
    v_glint_uv = a_glint_uv;
}
