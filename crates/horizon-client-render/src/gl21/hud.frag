#version 120

// The GL 2.1 path's HUD fragment stage: `hud.wgsl`'s `fs_main`, one shader for every blend the
// batch's runs ask for (the blending itself is `glBlendFunc`, set from `gl21::hud::state`).
//
// Kept from the WGSL stage, because each changes the picture:
// - the glint branch (`uv.x >= 2`): the item's own texel at `uv - 2` masks a glint sample, and the
//   result is squared so that only its brighter half shows;
// - the 0.004 cut, below which a texel is not blended at all.
//
// Left out, and why: a resource pack's text shader (`hud.rs`'s `pack_text`) needs a separate program
// per pack and is off on this path (plan §5.4), and the End portal's procedural `portal_layer` uses a
// 16-colour table indexed in a loop, which GLSL 1.20 can do — it stays out until the End is ported,
// and the run's blend keeps the frame opaque meanwhile.

uniform sampler2D u_atlas;
uniform sampler2D u_glint;
// The vignette's bilinear copy of the atlas (`hud::State::linear`); `hud.frag` picks it by
// `u_linear`, which is the sampler object the WGSL path binds instead.
uniform sampler2D u_linear;
uniform float u_use_linear; // 1 when the run samples u_linear, 0 for u_atlas.

varying vec2 v_uv;
varying vec4 v_color;
varying vec4 v_proj;
varying vec2 v_glint_uv;

void main() {
    if (v_uv.x >= 2.0) {
        // The item's texel masks the glint, as `hud.wgsl`'s fragment stage does it.
        vec4 mask = texture2D(u_atlas, v_uv - vec2(2.0, 0.0));
        if (mask.a < 0.1) {
            discard;
        }
        // The strength is the vertex colour's alpha, as `hud.wgsl`'s `hud.size.w` is for its runs.
        vec4 glint = v_color.a * texture2D(u_glint, v_glint_uv);
        gl_FragColor = vec4(glint.rgb * glint.rgb, mask.a);
        return;
    }
    vec4 c;
    if (u_use_linear > 0.5) {
        c = texture2D(u_linear, v_uv) * v_color;
    } else {
        c = texture2D(u_atlas, v_uv) * v_color;
    }
    if (c.a < 0.004) {
        discard;
    }
    gl_FragColor = c;
}
