// Terrain, fragment stage: the GLSL 1.20 port of terrain.wgsl's fs_main (the vanilla path).
//
// What the target GPU changes here:
//
//  * one `sampler2D` atlas, not eight `texture_2d_array` samplers: the mesher resolved the
//    layer into a UV inside the sprite. The two atlases the plan budgets for (2 x 2048^2:
//    16 MiB each on a 256 MiB card, against 67 MiB for one 4096^2) sit side by side in one
//    4096x2048 texture, so choosing one is a `u` offset of half a texture and no branch.
//    `v_atlas` is that choice, 0 or 1.
//  * the lightmap is a 16x16 `sampler2D` read with `texture2D`: `texelFetch` is 1.30.
//  * cutout is `discard`, which 1.20 has and SM 3.0 does cheaply.
//  * no `textureGradARB` yet: mip selection comes from `texture2D`'s implicit derivatives.
//    Where it has to match the wgpu path exactly (the RGSS blend) a later pass adds the
//    `ARB_shader_texture_lod` variant; the target does have that extension.
//
// Measured by tools/gl21_glsl.py: 12 uniform components, 10 varying floats, 2 samplers of
// 16. The vertex stage's varyings must match this list name for name; the checker's
// cross-stage pass (--check) refuses a mismatch, which the target's linker would refuse too.

uniform sampler2D u_sprites;
uniform sampler2D u_lightmap;
uniform vec4 u_fog_color;
// x: fog start (blocks), y: fog end, z: environmental start, w: environmental end. The
// environmental pair is the Nether's 10..96 and wins when z < w.
uniform vec4 u_fog;
// x: cutout (1.0 when this pass discards below the mesher's alpha threshold, 0.0 for an
// opaque pass), y, z, w: reserved. A vec4 rather than a float because `glUniform1f` and a
// vec4's first component are both one call to set, and packing the pass flags together
// leaves the uniform budget for the ones that need it.
uniform vec4 u_pass;

varying vec2 v_uv;
varying vec4 v_color;
varying vec2 v_light;
varying float v_cylindrical;
varying float v_atlas;

// Vanilla's lightmap: sky light on x, block light on y, already through the gamma curve and
// the effects. The WGSL reads the same 16x16 texture with `texelFetch`; the half-texel shift
// is what `texture2D` needs to land on the same texel.
vec4 sample_lightmap(float sky, float block) {
    return texture2D(u_lightmap, vec2(sky, block) * (15.0 / 16.0) + (0.5 / 16.0));
}

void main() {
    // The two atlases are the left and right halves of one 4096x2048 texture, and `v_uv` is
    // inside a single atlas (0..1 both ways). So the choose is a half-width offset folded
    // into one scale: no branch, and nothing interpolates that should not.
    vec2 uv = vec2((v_uv.x + v_atlas) * 0.5, v_uv.y);
    vec4 texel = texture2D(u_sprites, uv);

    // Cutout (leaves, grass, glass panes): vanilla's RenderType.cutout discards below 0.1.
    // The flag is per pass (`u_pass.x`), not per vertex: meshing already splits cutout and
    // opaque sections into different draw runs, so the branch is constant for a whole run.
    if (u_pass.x > 0.5 && texel.a < 0.1) {
        discard;
    }

    vec4 light = sample_lightmap(v_light.x, v_light.y);
    vec3 color = texel.rgb * v_color.rgb * light.rgb;
    // The alpha comes from the lightmap, as in `terrain_vertex` (WGSL): `v_color.a` is not a
    // transparency. The mesher puts the HDR emission code in the colour word's top byte
    // (`mesher.rs`'s `EMISSION_BITS`), and the wgpu path reads it as `@location(5) emission`,
    // not as alpha. Reading it as alpha here would make a glowing face transparent.
    float alpha = texel.a * light.a;

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

    // `fog_color.a` is the fog's own opacity, as in the WGSL: it scales the blend rather than
    // becoming the world's alpha, and it is 0 when nothing is fogged.
    color = mix(color, u_fog_color.rgb, fog * u_fog_color.a);

    gl_FragColor = vec4(color, alpha);
}
