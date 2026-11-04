#version 120

// Solid, cutout and translucent terrain, as GLSL 1.20: the port of `terrain.wgsl`'s fragment stage
// that GL 2.1 can compile (docs/opengl21-plan.md §5.1).
//
// One shader draws all three of wgpu's passes (`draws.rs` explains why they are one on GL 2.1), so
// the one thing that differs between them is a uniform: the alpha test's threshold, 0.5 for solid
// and cutout faces as in `solid_color`, 0.01 for see-through ones as in `fs_translucent`.
//
// Kept, because each changes the picture: the sprite times the vertex colour, that alpha test (and
// its bypass for a sprite whose layer says force-opaque, which is what the vertex colour's alpha
// carries here — `vertex.rs`), the lightmap at the same coordinates and in the same order as wgpu's
// `sample_lightmap` (block light across, sky light up), the fog over its two distances, and the
// fade-in toward the fog colour.
//
// Left out, and why: HDR's emissive mark in the frame's alpha (no HDR on this path); the emissive
// overlays (an extra sprite per vertex, which a draw per sprite cannot bind); the water mirror's cut
// plane; the RGSS and anisotropic filtering modes (`docs/opengl21-plan.md` §5.1: no
// `textureSampleGrad` before GLSL 1.30, so the hardware's own derivatives and mip selection are
// used); distant-terrain tiles' repeating UVs (the plan's §6 drops LOD first).
//
// Fog arithmetic is vanilla's `FogRenderer`, as `terrain.wgsl` keeps it: linear between the two
// distances of each pair, the farther of the two, scaled by the fog colour's alpha.

uniform sampler2D u_atlas;
uniform sampler2D u_lightmap;
// `GL_EXT_texture_lod_bias` is on the plan's required list (§9.2), but no GLSL 1.20 driver in the
// wild applies `GL_TEXTURE_LOD_BIAS` to `texture2D`: the bias is the sampler's state on the X1650,
// so the shader passes it to every sprite sample itself (`gfx.rs`'s `mip_bias`, one uniform).
uniform float u_bias;
uniform vec4 u_fog_color;
// environmental start/end, render-distance start/end.
uniform vec4 u_fog;
// `Pass::alpha_test`: the threshold below which a face is cut away, 0 for a pass that keeps all.
uniform float u_alpha_test;

varying vec2 v_uv;
varying vec4 v_color;
varying vec2 v_light;
varying float v_visibility;
varying float v_spherical;
varying float v_cylindrical;

// `sample_lightmap`: the lightmap covers 16x16 levels on a 256-texel texture, sampled at a level's
// centre, block light across and sky light up.
vec4 lightmap(vec2 levels) {
    vec2 uv = levels * 16.0 / 256.0 + 0.5 / 16.0;
    return texture2D(u_lightmap, clamp(uv, vec2(0.5 / 16.0), vec2(15.5 / 16.0)));
}

// 0 before `start`, 1 past `end`, linear between.
float linear_fog(float d, float start, float end) {
    if (d <= start) {
        return 0.0;
    }
    if (d >= end) {
        return 1.0;
    }
    return (d - start) / (end - start);
}

void main() {
    vec4 texel = texture2D(u_atlas, v_uv, u_bias);
    // `v_light` is the level pair (sky, block); the lightmap wants (block, sky).
    vec4 color = texel * vec4(v_color.rgb, 1.0) * lightmap(v_light.yx);
    // The fade-in mixes toward the fog colour, as the mesh's own visibility does on wgpu.
    color = mix(vec4(u_fog_color.rgb, color.a), color, v_visibility);
    // A force-opaque sprite — opaque leaves, quads over opaque texels — arrives with 1 in the vertex
    // colour's alpha and is never tested.
    if (color.a < u_alpha_test && v_color.a < 1.0) {
        discard;
    }
    float f = max(linear_fog(v_spherical, u_fog.x, u_fog.y),
                  linear_fog(v_cylindrical, u_fog.z, u_fog.w));
    gl_FragColor = vec4(mix(color.rgb, u_fog_color.rgb, f * u_fog_color.a), color.a);
}
