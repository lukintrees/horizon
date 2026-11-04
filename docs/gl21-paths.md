# The poorest renderer paths, and how to run them on any machine

Phase 0 of [the OpenGL 2.1 plan](opengl21-plan.md). `HORIZON_GL_PROFILE=x1650` makes the *existing*
wgpu renderer take every path it already has for a weak device, on any machine, without changing what
a frame shows. Nothing here runs on the Radeon X1650 itself: that card needs the separate renderer the
plan describes, because wgpu's GL backend needs OpenGL 3.3 core. What this buys is that the degrading
branches — the CPU sprite animation, the CPU occlusion path, the plain vertex layout, the per-section
draws — run in CI and on a developer's machine, so they are not first tried on a player's 2006 driver.

Code: `crates/horizon-client-render/src/gl21/` (`profile.rs` is the card, `mod.rs` the switches).

## The switches

| Variable | Effect |
| --- | --- |
| `HORIZON_GL_PROFILE=x1650` | The card's limits where a limit is asked of the driver, and every poorest path below. Also names the profile in the log, with whether it could run this renderer (it cannot). Aliases: `r500`, `gl21`. |
| `HORIZON_GL21_PATHS=1` | Every poorest path below, with the driver's real limits kept for the atlas and pipelines. |

Both are read where they are used (at `Renderer::new`), not at start-up, and `HORIZON_GL_PROFILE` with a
name nobody knows selects nothing and logs a warning: a typo in a job that then measured the full path
would be worse than a failure.

## What is turned off

| Feature | How it is detected today | Why the poorest path |
| --- | --- | --- |
| Sprite animation on the GPU | `gfx/animate.rs`, `SpriteAnimator::compute`: compute writing a storage texture | no compute before GL 4.3; the CPU path rewrites the same frames (`sprites.rs`) |
| Occlusion culling | `gfx/cull.rs`, `Cull::supported`: Vulkan with early depth tests, or Metal | the test is a compute pass over storage buffers |
| Flat section draws | `gfx/flat.rs`, `supported`: vertex storage and compute | the draws are copied into one index list by compute |
| Translucent batching | `gfx.rs`, `translucent_batchable`: indirect multi-draw, not on OpenGL | the sorted pages need the GPU's own counts and a storage arena |
| GPU culling | `gfx/gpu_culling.rs`, `supported` | compute, a count the GPU wrote, and eight storage buffers |
| Improved transparency (OIT) | `gfx/oit.rs`, `supported`: `FLOAT32_BLENDABLE` | float render targets drawn by storage-heavy passes |
| Visibility buffer | `gfx/visbuffer.rs`, `wanted`: `HORIZON_VISBUFFER=1` and storage buffers | it runs only inside flat frames, with 16+ storage buffers |
| Compact quad layout | `gfx.rs`, `packed::pack_quads` (already off without flat draws) | its unpacking is GLSL 1.30 integer arithmetic (`>>`, `&`) |
| Sprite arrays | `gfx.rs`, `layer_split`: the adapter's `max_texture_array_layers` | a 2006 card has none; one array of `SPRITE_SPLIT` layers is the most its texture size names, and GL 2.1 will read one flat atlas per array |

The last row is a check rather than a picture: the game keeps drawing from its texture arrays, and what
changes is which split the check allows. The first eight change which pass runs; the plan's renderer is
what turns them from "off" into "drawn another way".

## Running it

```sh
# The poorest paths with the driver's own limits (works on any GPU, including llvmpipe):
HORIZON_GL_PROFILE=x1650 cargo run --profile fast -p horizon-client

# The same paths on a headless software GPU, for CI or a quick check:
MESA_GL_VERSION_OVERRIDE=2.1 MESA_GLSL_VERSION_OVERRIDE=120 LIBGL_ALWAYS_SOFTWARE=1 \
  HORIZON_GL_PROFILE=x1650 target/fast/horizon-client

# A render capture replayed through them, no game and no world:
HORIZON_GL_PROFILE=x1650 cargo run --release -p horizon-client-render --example scene_replay -- /tmp/scene
```

The log says which profile was forced and whether it could run this renderer, or that it needs the
separate one.

## The shaders of the renderer to come

`crates/horizon-client-render/src/gl21/*.vert|*.frag|*.glsl` are GLSL 1.20 and held to it by
`tools/gl21_shaders.py`, which CI runs with the other tool tests (`tests/test_gl21_shaders.py`): no
`in`/`out`, no `layout`, no `texture()`, no bit arithmetic, no texture arrays, no `gl_VertexID`, no
vertex texture fetch. Writing a shader that needs a newer OpenGL fails a test instead of a driver. The
present pass (`present.vert`/`present.frag`) is the first one, the GL 2.1 copy of `world_post.wgsl`'s
blit; `terrain.vert`/`terrain.frag` are the port of `terrain.wgsl`'s terrain stages.

## What of the renderer exists

Not a renderer yet: no window, no context, nothing drawn. What is here is the part that can be
checked without a GPU or a driver, and CI does check it (`cargo test -p horizon-client-render`):

| File | What it decides |
| --- | --- |
| `gl21/atlas.rs` | Sprites in one square power-of-two texture instead of a texture array; a sprite is a rectangle (`u_sprite`). No padding yet, so the top mips cross the seam between neighbours. |
| `gl21/vertex.rs` | A vertex in what GLSL 1.20 can read (28 bytes, four attributes), and a section's vertex list cut into one run per sprite — one draw call per sprite instead of a layer attribute. |
| `gl21/draws.rs` | The frame's calls and their order: sky, then solid faces, then see-through ones farthest first, then the present quad; and how many calls that is. |
| `gl21/terrain.vert|frag` | The port of `terrain.wgsl`: sprite × vertex colour × lightmap, one alpha test whose threshold is a uniform, fog over both distances, a section's fade-in. |

What the GL path does not draw yet, and so what a player would notice: no HDR, no emissive
overlays, no water mirror, no distant-terrain LOD, no RGSS/anisotropic modes, no dynamic or coloured
lights. The plan's §5.4 lists them with the reason for each.
