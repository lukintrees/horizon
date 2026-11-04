// Terrain, fragment stage: the GLSL 1.20 port of terrain.wgsl's fs_main (the vanilla path).
//
// What the target GPU changes here:
//
//  * one `sampler2D` atlas, not eight `texture_2d_array` samplers: the mesher resolved the
//    layer into the UV and passes the layer along only so this stage can pick between the
//    two atlases the plan budgets for (2 x 2048^2: 16 MiB each on a 256 MiB card, against
//    67 MiB for one 4096^2).
//  * the lightmap is a 16x16 `sampler2D` read with `texture2D`: `texelFetch` is 1.30.
//  * cutout is `discard`, which 1.20 has and SM 3.0 does cheaply.
//  * no `textureGradARB` yet: mip selection comes from `texture2D`'s implicit derivatives.
//    Where it has to match the wgpu path exactly (the RGSS blend) a later pass adds the
//    `ARB_shader_texture_lod` variant; the target does have that extension.
//
// Measured by tools/gl21_glsl.py: 8 uniform components, 11 varying floats, 2 samplers of
// 16. The vertex stage's varyings must match this list name for name; the checker's
// cross-stage pass (--check) refuses a mismatch, which the target's linker would refuse too.

uniform sampler2D u_sprites;
uniform sampler2D u_lightmap;
uniform vec4 u_fog_color;
// x: fog start (blocks), y: fog end, z: environmental start, w: environmental end. The
// environmental pair is the Nether's 10..96 and wins when z < w.
uniform vec4 u_fog;

varying vec2 v_uv;
varying vec4 v_color;
varying vec2 v_light;
varying float v_cylindrical;
varying float v_layer;
varying float v_cutout;

// Vanilla's lightmap: sky light on x, block light on y, already through the gamma curve and
// the effects. The WGSL reads the same 16x16 texture with `texelFetch`; the half-texel shift
// is what `texture2D` needs to land on the same texel.
vec4 sample_lightmap(float sky, float block) {
    return texture2D(u_lightmap, vec2(sky, block) * (15.0 / 16.0) + (0.5 / 16.0));
}

void main() {
    // Two atlases live in one texture whose right half is the second atlas: the mesher writes
    // u in [0, 1) for the first and [1, 2) for the second, so the shader folds the upper half
    // back. One branch on a value that is uniform across a quad is cheap on this chip.
    vec2 uv = v_uv;
    if (v_layer >= 1.0) {
        uv = vec2(uv.x - 1.0, uv.y);
    }
    vec4 texel = texture2D(u_sprites, uv);

    // Cutout (leaves, grass, glass panes): vanilla's RenderType.cutout discards below 0.1,
    // and the flag comes from the mesher so an opaque section never pays for the branch.
    if (v_cutout > 0.5 && texel.a < 0.1) {
        discard;
    }

    vec4 light = sample_lightmap(v_light.x, v_light.y);
    vec3 color = texel.rgb * v_color.rgb * light.rgb;
    float alpha = texel.a * v_color.a;

    // Fog, as the WGSL does it: the environmental range wins when one is set, otherwise the
    // render distance. `max(..., 0.0001)` keeps a degenerate range from dividing by zero,
    // which on this driver paints the world black rather than reporting anything.
    float fog_start = u_fog.x;
    float fog_end = u_fog.y;
    if (u_fog.z < u_fog.w) {
        fog_start = u_fog.z;
        fog_end = u_fog.w;
    }
    float fog = clamp((v_cylindrical - fog_start) / max(fog_end - fog_start, 0.0001), 0.0, 1.0);

    color = mix(color, u_fog_color.rgb, fog);
    alpha = mix(alpha, u_fog_color.a, fog);

    gl_FragColor = vec4(color, alpha);
}
