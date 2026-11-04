# `gfx21::Renderer` contract (generated)

This is the public surface `game.rs` and `game/*.rs` read from `gfx::Renderer`.
It was extracted from the current commit with:

```sh
grep -rhoE "\brenderer\.[A-Za-z_][A-Za-z0-9_]*" crates/horizon-client/src/game.rs crates/horizon-client/src/game/*.rs
grep -rhoE "\bRenderer::[A-Za-z_][A-Za-z0-9_]*" crates/horizon-client/src/game.rs crates/horizon-client/src/game/*.rs
```

PR 8 replaces `Renderer` with a `#[cfg]`-selected type alias. Every name below
must exist on `gfx21::Renderer`; fields can be plain data and methods can be
no-ops that return `None`/`0`/`false`, but none may be missing. `Option` methods
are not part of the renderer contract and are listed only for completeness.

## `renderer.<name>`

| Name | Uses | Kind | Legacy stub |
|---|---:|---|---|
| `add_mod_signs` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `and_then` | 1 | Option method | n/a (not a renderer member) |
| `as_mut` | 24 | Option method | n/a (not a renderer member) |
| `as_ref` | 40 | Option method | n/a (not a renderer member) |
| `boot` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `cit` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `clear_lods` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `clear_sections` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `config` | 14 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `define_pack_macros` | 2 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `device` | 2 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `drawn_sections` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `drop_shader_packs` | 2 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `gamma` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `hud` | 2 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `info` | 2 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `is_none` | 1 | Option method | n/a (not a renderer member) |
| `is_some` | 4 | Option method | n/a (not a renderer member) |
| `is_some_and` | 1 | Option method | n/a (not a renderer member) |
| `light_color` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `lod_detail_revision` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `paced_elsewhere` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `pack_inputs` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `queue` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `queue_lods` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `recreate_surface` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `release_surface` | 3 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `remove_columns` | 2 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `render` | 8 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `resize` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `resolve_with_rule` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `screenshot` | 5 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `screenshot_result` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `set_lines_shader` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `set_pass_timing` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `set_present_mode` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `set_separate_ao` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `set_shader_pack` | 3 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `set_terrain_loading` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `shader_pack_dimension` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `start_occlusion` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `take` | 2 | Option method | n/a (not a renderer member) |
| `title_artwork` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `upload` | 2 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `upload_lods` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `wait_in_flight` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |
| `world` | 1 | field or method | no-op / neutral value unless listed in docs/design/legacy-gl21.md |

## `Renderer::<name>` (associated functions)

| Name | Uses | Legacy stub |
|---|---:|---|
| `device_lost` | 2 | must exist; `new` is the PR 8 constructor, the rest may return defaults |
| `new` | 3 | must exist; `new` is the PR 8 constructor, the rest may return defaults |
| `pack_shadow_casters` | 1 | must exist; `new` is the PR 8 constructor, the rest may return defaults |
| `take_dropped_sections` | 1 | must exist; `new` is the PR 8 constructor, the rest may return defaults |

## Additional accesses through a local `r` bound from `self.renderer`

These appear as `r.<name>` inside `if let Some(r) = self.renderer.as_...()` blocks.
They are not exhaustively enumerated here (the same short name is used for many
unrelated locals in `game.rs`), but PR 8 must cover at least:

`config`, `gpu_times`, `last_wait`, `last_encode`, `drawn_sections`,
`screenshot_result`, `capture_result`, `display_changed`, `wants_water_plane`,
`visible_block_entities`, `static_block_entity_frame_stats`, `skinned_models`,
`camera_frustum`, `culled_sections`, `native_shadow`, `pack_entity_shadows`,
`resize`, `clear_sections`, `set_flat_draws`, `set_gpu_culling`, `set_hdr`,
`set_present_mode`, `set_world_post`, `set_texture_filtering`, `set_mipmap_levels`,
`section_sides`, `section_draws`, `section_vertex_bytes`, `sprite_split`,
`shader_pass_times`, `own_pass_times`, `gpu_culling`, `gpu_frame`, `hdr_on`,
`hdr_line`, `frame_generation_share`, `skins_entities`, `skins_tagged`.

## Not in scope for PR 2

`wgpu`-typed fields (`device`, `queue`, `hud`, `info`, `config`) cannot be
reproduced in the legacy backend. PR 8 splits `build_renderer` and
`loading_renderer` in `game.rs` with `#[cfg]` so those accesses disappear from
the legacy build instead of being faked.

## PR 2b / PR 3 stage 1 (additive)

PR 2b adds the `legacy-gl` Cargo feature to `horizon-client` and
`horizon-client-render`, makes `wgpu`/`naga`/`pollster` optional behind the
default `modern-gl` feature, and adds a crate-root alias
`pub type Renderer = gfx21::Renderer;` under `#[cfg(feature = "legacy-gl")]`.
`game.rs` still imports `gfx::{Frame, Renderer}`; the `#[cfg]` split of those
imports is PR 8. `gfx21::Frame` and `gfx21::Renderer` are zero-sized markers,
so no half-implemented renderer can be used by accident.

PR 3 stage 1 adds `context.rs` (validated `Report` → `ShaderDialect`/`FboKind`),
`state.rs` (`GlCommand`/`GlState` cache, GL2 enums only) and `smoke.rs`
(headless `gl21_clear` triangle command stream). These modules make no GL calls
and have no external crate dependencies. All `renderer.<name>` and
`Renderer::<name>` accesses above remain the PR 8 checklist.

## PR 3 stage 2 / PR 4 stage 1 (real context, GLSL 110/120)

`examples/gl21_clear.rs` is gated behind `required-features = ["legacy-gl"]`.
It creates an OpenGL 2.1 (or 2.0 under the CI override) compatibility context
through `glutin`/`glutin-winit`, loads GL 2.1 entry points with `glow`, prints
`GL_VERSION`, `GL_RENDERER`, `GL_VENDOR` and `GL_SHADING_LANGUAGE_VERSION`,
selects `ShaderDialect::Glsl120` or `Glsl110`, links one smoke-triangle
program, draws it and checks `glGetError() == GL_NO_ERROR`. The optional
`glow`/`glutin`/`glutin-winit`/`raw-window-handle` dependencies and their
notices are recorded in `THIRD_PARTY_NOTICES.md`; no upstream source was copied.

`program.rs` + `shaders/` ship the same triangle as `#version 120` and
`#version 110` pairs. `validate()` rejects a mismatched `#version`, GL3+
tokens (`in`/`out`/`layout`/`uint`/`texelFetch`/`gl_VertexID`/...), missing
`attribute`/`gl_Position`/`gl_FragColor` and missing `void main`; `texture2D`
stays allowed. F0 has no GLSL program (`smoke_program` returns `None`). The
atlas, mesh converter, sections, passes and HUD still follow in PR 5–7, and the
full `#[cfg]` split of `gfx`/`game.rs` remains PR 8.
