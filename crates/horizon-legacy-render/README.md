# Legacy SDR renderer

Reusable GL2 renderer, **not yet an integrated or playable game backend**.
This workspace contains `horizon-legacy-render` (native GL/contexts) and
`horizon-render-api` (safe CPU-only upload/draw contract). It is excluded from the
large root workspace so its tests build offline with **no registry dependencies**.
The game render crate depends on the CPU API and exposes a `render_bridge` from
its actual `TerrainVertex` / `SectionMesh`; it still uses wgpu for gameplay.

## Implemented

- Safe `RenderBackend` interface: create/update/remove RGBA8 textures, indexed
  triangle meshes, ordered frame submission, bounded readback.
- GL2-only ABI, GLSL110 programs, ordinary uniforms, 7 vertex attributes, 3 texture
  units (ordinary terrain/GUI vertices still use 4 attributes). No VAO, UBO, SSBO, instancing, arrays, compute, FBO or GL3 requirement.
- Opaque/cutout/translucent/overlay passes, depth test/write control, standard
  alpha blending, pixel scissor, SDR lightmap and basic spherical fog.
- POT nearest-filtered 2D atlas pages with extruded borders (level zero only).
  CPU atlas primitives are reused from this repository's draft PR #1, not a new
  parallel packing implementation. No external project source was copied.
- Terrain conversion preserves page/pass changes and triangle order, splits
  before u16 indices wrap, removes mesher material alpha flags from coverage,
  decodes lightmap coordinates, and enforces a CPU output-vertex budget.
- f64 camera subtraction before float conversion, explicit wgpu-to-GL projection
  conversion, input/ID/rectangle/index validation **before** frame clear/draw.
- Unique typed IDs, rejection of stale/foreign IDs, byte accounting and removal.
  Default budgets: 32 MiB textures + 32 MiB meshes; 4096 texture / 65536 mesh object
  caps. Accounting estimates payload bytes, not actual driver VRAM/VA usage.
- Thread-confined owned native EGL/OSMesa/WGL contexts, rebound on every safe
  renderer operation; multiple contexts on one thread cannot draw into each other.
  Renderer borrows its context; resource deletion occurs before context teardown.

## Run and test

```sh
cargo test --offline --manifest-path crates/horizon-legacy-render/Cargo.toml --workspace
cargo clippy --offline --manifest-path crates/horizon-legacy-render/Cargo.toml \
  --workspace --all-targets -- -D warnings
cargo clippy --offline --manifest-path crates/horizon-legacy-render/Cargo.toml \
  --workspace --all-targets --target i686-pc-windows-msvc -- -D warnings
python3 tools/legacy_gl_smoke.py --context osmesa
```

The probe uses this exact renderer through its safe API in addition to the
independent triangle probe. In each GL2.0, GL2.1 and unrestricted llvmpipe process,
25 backend checks execute real uploads/draws/readbacks: texture quadrants, texture
updates and bounds, depth, cutout holes, ordered alpha blend with depth writes off,
overlay/scissor reset, lightmap, spherical/dual-distance fog, reversed game depth, large coordinates, validation/budgets,
context isolation, resource reclamation, and the shared terrain-atlas conversion
contract, resident-section replacement/rollback, eviction, lightmap updates,
cleanup after partial construction, GUI compositing/depth/glyph updates, and
paged GUI streaming with linear gutters, depth continuity and one-page eviction,
entity material state and independently sampled animated glint. The runner requires all 25 outcomes; a triangle-only report cannot pass.
Report schema 2: `target/legacy-gl-probe/report.json`, ignored by Git.

The terrain end-to-end fixture uses the real CPU contract and renderer with
original synthetic pixels, **not stand-in game/mesher types**. The tests in `horizon-client-render::{render_bridge,gui_bridge}` target actual
game types. They now compile/run in the branch-only GitHub workflow, including
a real `HudAtlas::boot`/`HudBatch` GL2 readback (no Mojang resources).

## Verified / blocked (2026-11-04)

Verified here: stable Rust 1.88.0, 38 workspace unit tests plus 2 probe unit tests;
Linux and Windows-x86 compile-only Clippy; real OSMesa Mesa 23.1.4 / llvmpipe
LLVM 17.0.6, all three modes, 25 backend checks per mode. Python regression suite:
642 tests, 1 skipped. Locally Windows is compile-checked; own Actions also links the ordinary x86
probe (see below), but no Win7 execution is claimed. Existing
[tool README](../../tools/legacy-gl-probe/README.md) records bootstrap provenance
and the GLSL120 reported-version workaround (shader source remains GLSL110).

The local game check remains blocked. GitHub vendor mirrors supply 287 of the
488 exact registry packages in the root lock; 201 are still missing. With the
explicit directory-source configuration, Cargo now stops at `cpal`, not `log`.
No lock checksums were changed to accept mismatched/modified mirror packages.
The root client-render package also requires Rust 1.90; the independent GL2
bootstrap's installed/tested Rust 1.88 is not a full-game toolchain.
Prepared game resources are also absent. Root workspace
`cargo metadata --offline --no-deps` succeeds, but this is not a Rust type check.
No generated data, downloads or runtime libraries are checked into the repository.

## Not done — do not describe this as the completed port

The game's `gfx::Renderer` is still concrete and wgpu-owned. Device/queue accesses,
settings/capability policy, creation/surface recovery and game frame construction
are not switched to `RenderBackend`. There is no legacy gameplay executable,
playable title/world/inventory path, GUI portal/CIT shaders, all special
entity materials, gameplay CPU-pose/frame wiring, resource animation/reload
integration, streaming upload scheduler,
complete game event-loop wiring, mipmapped/repeating sprite UVs, or final culling.
Repeating/tiled UVs are rejected explicitly. Fragment-sampled lightmaps and basic
fog are not claimed to match every vanilla lighting/fog case. Packs/mods and
world data are not silently stripped to make a demo appear playable.

Win7-specific std/toolchain, PE32 imports/CRT/ISA, input/audio/TLS, full-game
memory stress and the real Radeon X1650 ICD remain unverified. Ordinary
`i686-pc-windows-msvc` checks are not Win7 certification. No FPS claim is made.
The required final result is still the [acceptance plan](../../docs/design/opengl2-win7-x86.md),
not this subsystem milestone.


## Resident sections and reproducible dependency mirrors

`horizon_render_api::scene::WorldScene` owns atlas/lightmap uploads and bounded
resident sections while borrowing its backend. It stages replacement meshes
before replacing the old section, retains failed cleanup IDs for retry, and
reclaims resources on unload/drop. Callers supply visible solid/translucent order;
it does not implement visibility, asynchronous scheduling or a global quad sort.
The game bridge has `upload_section` for real `SectionMesh` data with one combined
vertex ceiling. The actual game bridge has now been compiled/tested by our branch-only Actions.

`backend_resident_sections` tests the scene against real GL uploads/readbacks,
including a replacement that fails halfway at the byte budget. The old mesh
remains visible, staging allocations are reclaimed, lightmap changes affect pixels,
a missing section does not clear the framebuffer, and constructor failure/drop
return resource counters to zero. Synthetic pixels are not game resources.

For a pinned, opt-in Cargo directory source using allowed GitHub hosts:

```sh
python3 tools/cargo_github_vendor.py
# Exit 2 means an incomplete inventory, not success. Read its report.json.
cargo --config .local/cargo-github-vendor/offline.toml check --offline --locked -p horizon-client-render
```

The tool checks name/version and package-checksum metadata against `Cargo.lock`,
then every listed file SHA256. It installs only listed files atomically and records
source repository/commit/path. It checks consistency with pinned vendor manifests,
**not independent authenticity of the original compressed .crate archive**.
It does not change Cargo.lock or normal Cargo configuration. Missing binary import
libraries, changed files and checksum-free Chromium vendor trees are not accepted.
Downloads and reports remain ignored under `.local/`; no third-party sources or
Mojang resources are added to this change. Eight offline Python tests cover
installation and rejection, including changed hashes, missing files, symlinks,
traversal and injected unlisted build scripts.


## Terrain/HUD integration (branch-only Actions)

`horizon-client-render --features legacy-gl` exports
`legacy_gfx::TerrainHudRenderer`: an owning native-context/scene coordinator taking
real SectionMesh, HudAtlas, HudBatch and font::Patch data. It supports section
uploads/unloads, changing lightmaps, HUD atlas replacement, bounded glyph uploads,
GUI scaling, top-left scissors, invert/additive/vignette blending and isolated
LEQUAL depth previews. `Clear::Load` and `Clear::Depth` preserve world colour across
views. Vignette uses a linear atlas sample; other draws restore nearest filtering.
The paged coordinator streams runs in original triangle order and releases each
transient mesh after drawing. It preflights tracked transient mesh capacity before
clearing the world; driver/load failures during streaming can still leave a
partial frame. There is no full-frame GPU rollback guarantee.

`Renderer::owned` and the owning `WorldScene<B>` remove self-referential lifetime
requirements for storing this coordinator in a Game. Borrowed backends still work
through `RenderBackend for &mut B`. Every native operation remains thread-confined.

This is an explicit terrain/HUD subset, **not the normal game renderer selection**.
Portal, CIT glint and pack-text shaders are not silently drawn as plain
atlas quads. Large HUD atlases now use bounded POT pages rather than a single
oversized texture; the upstream full-image CPU atlas construction still needs
Win7 address-space qualification.
Ordinary entity/item/particle and hand paths are now integrated (below); sky,
remaining special entity materials and gameplay input/event-loop wiring remain work.

Our own `.github/workflows/legacy-gl-branch.yml` triggers only on
`arena/a7e96628-horizon`. The first two runs passed both independent GL2 readbacks
and actual game bridge compilation/tests; the second additionally passed the
real boot-atlas/HUD GL2 test. Run 41773789614 also passed the owning coordinator, combined real mesh/HUD
readback, glyph patches, atlas reload and game-bridge Clippy. These are not tests on Win7/X1650.


Earlier full-client validation: [41774182610](https://github.com/lukintrees/horizon/actions/runs/41774182610)
(all three jobs passed, code commit `85623ba`):

- `cargo check --locked -p horizon-client --no-default-features --features horizon-client-render/legacy-gl`;
- real game bridge CPU tests, three opt-in actual-game GL2 pixel tests, and
  `horizon-client-render --features legacy-gl` library/test Clippy;
- independent OSMesa GL2 matrix and modern-Windows i686 probe compilation/linking.

The x86 probe artifact is explicitly named `probe-modern-windows-x86-not-win7-qualified`.
A Cargo check does not link/run the game, and the ordinary i686 target is not the
Win7-specific std/import/CRT qualification. The `legacy-gl` feature currently
adds a coordinator alongside wgpu; it does not yet select it at Game startup or
remove the modern dependency graph. No playable Win7 game is claimed.


## Paged HUD backing and residency

`horizon-render-api::paged_gui` clips real GUI triangles into 512x512 POT pages
(510x510 interior with one-texel replicated gutters). It interpolates colour,
depth, UVs and light UVs in double precision, preserves triangle order and a single
depth clear per original group, and bounds output vertices and visited cells.
Pages are **not sorted**: transparent overlaps keep their original draw order.

`TerrainHudRenderer` uses independent LZ4 CPU blocks and an LRU GPU cache. Defaults:
at most 8 resident pages (8 MiB tracked RGBA payload), 128 MiB compressed backing,
4096 virtual pages, and 8 MiB incoming+decoded dirty-page patch working storage.
Actual residency is reduced to the byte/object capacity left after terrain,
lightmap and white textures. These are tracked payload limits, not measured driver
VRAM; original atlas construction and full-process peaks remain separate concerns.
Glyph updates visit only dirty subrectangles, with a separate write-work cap
to prevent repeated tiny edge edits amplifying into unbounded tile scans.
Each dirty page is decoded once per patch batch; neighbouring gutters are patched
in the same transaction. Validation/compression-budget failures retain old CPU
backing. Reload prepares a replacement backing before retiring the old GPU cache;
GPU page uploads are lazy. Cleanup/retry handles remain owned by the backend.

Own runs [41776018494](https://github.com/lukintrees/horizon/actions/runs/41776018494)
and [41776564666](https://github.com/lukintrees/horizon/actions/runs/41776564666)
passed all three jobs. The latter verifies code `d5dbd2b`: full-client Cargo check,
game lib/tests Clippy, CPU bridges/backing, **five actual-game GL2 fixtures**, the
16-check llvmpipe matrix and the ordinary Windows x86 probe build/link. A synthetic
16385x2 `HudAtlas` is drawn with only one 512x512 GPU page despite requesting eight;
pixel readbacks check both sides of a glyph-patched seam. An exhausted transient
mesh budget fails before erasing the previous frame. Independent readbacks also
check alpha order, linear seam continuity, depth across evictions, dirty reload
and complete resource release. No Win7/X1650 gameplay or FPS result is inferred.

Final patch-work verification:
[41777293951](https://github.com/lukintrees/horizon/actions/runs/41777293951)
passed all three jobs at `0aa4a05`. The CPU backing suite additionally compares
all 90 rectangles of a 5x3 source against complete page rebuilds and checks atomic
rejection of repeated tiny edge writes exceeding the work budget. Full-client
check, game Clippy, all five actual-game GL2 fixtures, independent runtime matrix
and ordinary Windows x86 probe linking passed again.


## Classic entities, particles and hand

`entity_bridge` consumes the **actual `mesher::Vertex`** stream. Neutral
`entity::runs` expands quad indices, decodes RGBA/SNORM normals, computes cardinal
lighting (including Nether and independently shaded back faces), and bilinearly
samples the CPU lightmap per vertex. There is **no vertex texture fetch**: GL2
can expose zero vertex texture units. Hurt/TNT/white flashes are mixed in GLSL
**after texture/tint and before lighting**, preserving the game's operation order.
Supported overlays: 0, 1, 5, 7, 8, 13, 14, 15, 16..31, 32, 33, 35, 36; HDR tier/OIT tags are ignored only as
they are in classic SDR. Glint flags require configured images (see below). Other flags and portal/crumbling/energy/dissolve
materials are rejected, not flattened into a normal textured quad.

Entity meshes have a checked 84-byte layout (base vertex, back colour, light
colour, independent glint UV); ordinary terrain meshes stay 44 bytes.
The foil-aware GUI uses a separate compact 52-byte layout (below). Typed mesh/pass mismatches
and malformed overlays are rejected before clears. Seven attributes and GLSL110
varyings fit desktop GL2 minima. Back-face culling, LEQUAL depth and alpha blending
are selected per material. Ordinary entities and translucent items write depth;
translucent particles do not. CPU clipping interpolates the extra lighting data
and checks 3D area, so camera-rotated constant-X walls are not discarded.

`TerrainHudRenderer::render_geometry` uses the same bounded atlas cache for:
solid/cutout terrain -> entity/item bodies -> translucent terrain -> particles ->
hand (separate GL projection, depth clear, no world fog) -> HUD. Inputs are already
camera-relative and CPU-posed. Order is stable within each phase; caller-side
visibility/sorting and GPU skin-frame fallback are not implemented here. Combined
entity+hand input/output vertices and transient payloads are bounded before the
world clear. CPU lightmap copies update only after successful texture updates.
Failures during streamed GPU work still do not provide full-frame rollback.

Final own validation: [41779419733](https://github.com/lukintrees/horizon/actions/runs/41779419733),
code `7dd78a8`, **all three jobs passed**: full-client check, game CPU bridges,
Clippy, **six actual-game GPU fixtures**, independent 3x17 llvmpipe matrix and
modern Windows x86 probe build/link. A combined real-type fixture puts terrain,
entities, particles, hand and HUD through a single resident 4x4 atlas page and
checks their alpha/depth ordering, lightmap updates, unsupported-material failure
isolation and transient cleanup. Independent readbacks additionally verify both
face colours/culling, hurt overlay order, interpolated light across page seams,
3D UV clipping, world-border-relative positions and typed-buffer rejection.
Local verification: 24 standalone Rust tests, host/i686 Clippy, 3x17 Mesa outcomes,
642 Python tests (1 skipped). This remains **not a playable Win7 executable**:
normal startup is still wgpu, full entity material/sky coverage and platform
integration/Win7 qualification remain unfinished.


## Shadows, depth-only boat masks and world text

Classic material kinds now distinguish bodies, biased text, shadows, translucent
items, water masks, particles and explicit see-through text. The coordinator uses
the game's classic material order (stable within each kind), not an atlas-page
sort: bodies -> biased text -> shadows -> items -> water masks -> translucent
terrain -> particles -> see-through text -> hand -> HUD.

Shadows accept faint coverage below the normal entity cutout, cull back faces and
never write depth. Polygon offset pulls bodies/text/shadows towards the camera;
its sign is mapped to conventional GL depth. These offsets still need real ATI
qualification, not an assumed equivalence across every depth-buffer format.
Boat masks bypass texture/alpha sampling, write depth without RGBA writes, and
need no atlas page upload. Mask UVs are canonicalised even when the unused source
UV is nonfinite. Polygon-offset text is back-face culled and writes depth;
see-through text bypasses depth test/writes while retaining world projection,
lighting and fog. World-only mask/shadow/text-pipeline materials are rejected in
the hand view. Item hand quads use the ordinary hand pipeline's bias.

Colour masks, culling, polygon offset, depth and vertex layouts are explicitly
restored between draws; clears restore colour writes even after a depth-only
frame. Independent readbacks check both transitions within one mixed draw list
and across views. Entity/hand/name-tag expansion consumes one shared remaining
vertex budget before clearing. Unpaged entity and GUI scratch is released before
GPU submission rather than retained by shadowed local variables.

Own runs [41781489098](https://github.com/lukintrees/horizon/actions/runs/41781489098)
(`4679b61`) and [41782002701](https://github.com/lukintrees/horizon/actions/runs/41782002701)
(`d648057`) passed all three jobs. The latter includes **seven actual-game GPU
fixtures**, CPU bridges, game Clippy/full-client check, **3x18 backend checks**
and ordinary Windows x86 probe build/link. The real-type fixture intentionally
submits mask/item/shadow out of order, then checks a faint shadow beneath an item,
water excluded from half the image, biased/cullable world text, name tags behind
terrain, hand composition and combined expansion-budget rejection without clearing
the previous frame. Local totals: **26 standalone Rust tests**, host/i686 Clippy,
3x18 Mesa outcomes, 642 Python tests (1 skipped). Remaining startup/sky/special
materials/platform/Win7 qualification limits above still apply.


Final verification of this continuation:
[41782569337](https://github.com/lukintrees/horizon/actions/runs/41782569337)
at `5fa39c0` passed all three jobs, including the scratch-lifetime cleanup and all
seven actual-game GPU fixtures. The branch workflow now excludes Markdown from
its crate-wide code globs, avoiding unnecessary builds for README-only updates.


## Entity, item, armour and hand glint

`entity::runs_with` accepts codes 7/8/33/36 with explicit `Options::glint`.
Code 8 retains hurt overlay; 33 retains item depth policy; 36 selects armour
texture and transforms model UV by scale .16, rotation 10 degrees and the game's
110000/30000-tick offsets (common period 330000). Ordinary item/entity glint
coordinates come from the actual vertex stream; atlas paging never remaps them.
No per-vertex `fract` is applied. The third sampler uses repeat + linear, with
sampler state reset when that texture later becomes an ordinary atlas image.
Nonconstant glint images may not alias atlas/lightmap samplers.

Composition follows the existing `entity.wgsl` SDR contract: cutout on
texture*tint, overlay, light, squared `(strength * foil.rgb)` contribution,
`max(alpha, strength)`, then fog. Foil image alpha is irrelevant. Strength keeps
the exact validated finite float in [0,1], not a quantised approximation.
GLSL110/GL2 minima still suffice; no vertex texture fetch or GL3 features.

`TerrainHudRenderer::set_glint_textures` stages both owned images before swap,
recomputes the bounded atlas quota and preserves old assets on bad pixels,
allocation failure or insufficient space for one page. Retired assets are
collected before next-frame capacity validation/upload. `Geometry::glint`
supplies strength and clock for both world and independent hand view. The full
game resource-loading/frame adapter is **not** yet wired to this opt-in method.

The new required `backend_entity_glint` outcome tests repeated/bilinear samples,
60 independent expected samples across clipped atlas pages, armour clocks and
image selection, squared half/zero strength, hurt/light/fog order, base cutout,
alpha restoration in darkness, sampler alias rejection/reset and final cleanup.
Local Rust 1.88 tests: **27**; OSMesa llvmpipe matrix: **3x19**. The added eighth
actual-game GPU fixture also covers hand, staged reload rollback, repeated reloads
and an exact 1164-byte texture / 516-byte transient mesh budget. Own run
[41785733646](https://github.com/lukintrees/horizon/actions/runs/41785733646)
at `1ba6133` passed all three jobs: eight actual-game GPU fixtures, full-client
check/game Clippy, independent matrix and modern Windows x86 probe link. Full
Python regression suite: **642 tests, 1 skipped**. No Win7 execution is claimed.
CIT glint and portal/crumbling/energy/dissolve remain unsupported; vanilla GUI
glint is covered by the next continuation.


## Vanilla inventory/GUI foil, separate from entity composition

The real `HudVertex` marker (`uv.x >= 2`) is decoded by
`gui_bridge::glint_runs` before atlas clipping. The coordinator uses typed
`gui::GuiVertex` (52 bytes: ordinary 44-byte base + independent foil UV), not the
84-byte world-lighting layout. `Pass::Gui` keeps the original alpha/additive,
invert/vignette or depth/scissor pipeline state. Native upload/draw validation
rejects Basic/Entity/Gui layout mismatches before clearing. Ordinary legacy
`gui::runs` / `gui::render` remain compatible and explicitly reject foil data.

Foil has the **GUI** shader contract from `hud.wgsl::fs_main`, not entity
`glint_over`: sample the original item's alpha mask, discard below .1, output
`(HudBatch::glint_alpha * foil.rgb)^2` with **mask alpha**. Ignore vertex tint,
lightmap and foil alpha. Additive is ONE/ONE, not SRC_ALPHA/ONE; half-alpha masks
therefore still add the full RGB. Alpha/depth pipelines retain normal source-alpha
blending. Neither model depth clears nor GL shader mode may be reset merely
because a batch changes from ordinary to foil vertices or crosses a u16/page
boundary. A pending clear survives an entirely degenerate material prefix until
the first visible triangle; it never leaks into a later non-depth overlay.

`paged_gui::split_gui` interpolates foil UV independently across page seams and
`Cache::render_gui` streams the resulting runs through the same bounded page
cache as terrain entities/hand/HUD. Existing staged texture reload also supplies
the GUI item image; the HUD strength is independent of world `Geometry::glint`.
Bad strengths/coordinates, mixed triangle markers, missing assets, unsupported
CIT/portal blends and input/expanded geometry-budget errors are rejected during
CPU preparation before the world colour clear. Driver/load errors during streaming
still do not have a full-frame rollback guarantee.

Required outcome `backend_gui_glint` tests a 60-pixel reference gradient across
three pages with a one-page cache, alpha cutoff/half masks, squared half/zero
strength, both depth/material orders, degenerate prefixes, isolated model scissors,
normal-overlay state reset, typed buffer/alias rejection before clear and cleanup.
Local tests: **31 standalone Rust**, host Clippy, **3x20 Mesa outcomes** and
**642 Python tests, 1 skipped**. The ninth actual-game GPU fixture uses the real
HudBatch, masks, dynamic patches, atlas + foil reload, invalid-input preservation
and an exact **1164-byte texture / 324-byte GUI mesh** budget. Own run
[41791059747](https://github.com/lukintrees/horizon/actions/runs/41791059747)
at `6c87c53` passed all three jobs: nine actual-game GPU fixtures, full-client
check/game Clippy, independent GL2 matrix and modern Windows x86 probe link.

Vanilla GUI foil is implemented in the opt-in coordinator; CIT remains rejected.
Normal Game startup/frame/asset scheduling is still wgpu. No playable Win7 build,
X1650 hardware run, new FPS figure or full resource-pack shader support is claimed.


## Resizable drawable and WGL window events

`Context::resize` replaces drawable storage, **not** the GL context. OSMesa keeps
its owned CPU backing in a thread-confined cell; GL work is finished before old
storage can be freed. EGL stages a replacement pbuffer with the retained config;
a failed surface retirement is retained and blocks another resize until cleanup,
bounding deferred native surfaces. WGL resizes the existing client area, leaving
its DC/pixel format/context alone. `size()` reports actual WGL client dimensions,
including zero while minimized; render/present reject suspended or oversized
surfaces. Requested dimensions outside 1..=4096 are rejected before native changes.
Same-size resize is a no-op. Colour/depth are unspecified after a successful size
change: the caller must redraw and rebuild projection/HUD for the new dimensions.
Framebuffer/pbuffer storage is **not** included in texture/mesh payload counters;
resize may temporarily retain old and staged surface storage.

`TerrainHudRenderer` forwards resize, poll-events, localized title and present
operations. Its resident section meshes, HUD cache, lightmap and foil IDs survive
resize; no reupload/recreation is used to disguise broken resource ownership.
`backend_surface_lifecycle` checks repeated non-square/odd/1x1 sizes, unchanged
resource counters, live texture updates/draws, readback bounds, invalid/same-size
preservation and final reclamation. The tenth actual-game GPU fixture redraws
real terrain plus a rebuilt foil-aware HudBatch across five sizes with exact
**1164 texture / 512 mesh bytes** (188 resident + 324 transient).

WGL now owns a private callback with a stable `Box<State>` and a unique class per
window. It buffers resize/focus, physical VK/scan/extended key data and repeat
counts, decoded Unicode text (UTF-16 surrogate pairs/WM_UNICHAR), signed client
pointer positions, five buttons, horizontal/vertical wheel and capture loss.
Adjacent absolute motion/resize events coalesce; at most 256 events and 256 queued
messages per poll are processed. Overflow is explicit (`lost_events`): the game
must release held input on loss/focus/capture loss. Close remains persistent even
on overflow. WM_CLOSE/Alt+F4 request closure **without destroying a renderer's
live drawable**; the owner tears down GL resources/context before its window.
Callback userdata is detached before destruction even on native error paths.
Queued messages are filtered by HWND; WM_QUIT is preserved for the outer loop.
Native modal moving/sizing can still block inside DispatchMessage; the count bound
is not a hard latency guarantee. Default shared arrow cursor and caller-supplied
Unicode title need no new windowing dependencies.

New CPU tests cover event overflow/coalescing, signed values, physical keys,
Unicode, focus, button/capture state and repeat counts. The Windows x86 workflow
also executes a real **hidden-window/no-GPU-ICD** test: two HWNDs, callback userdata,
resize, title, posted keyboard/Unicode messages, queue isolation and a close that
leaves the HWND alive until Drop. This is distinct from executing the WGL GPU
renderer and does not qualify Win7 or the X1650 ICD.

Local Rust 1.88: **36 standalone tests**, host/i686 Clippy, **3x21 OSMesa readback
outcomes**, **642 Python tests (1 skipped)**. Own run
[41795980600](https://github.com/lukintrees/horizon/actions/runs/41795980600)
at `3339003` passed all three jobs: ten actual-game GPU fixtures, full-client
check/game Clippy, runtime matrix, **35 executed x86 Windows workspace tests**
(including the real HWND test) and ordinary x86 probe linking.

Actual x86 execution exposed two issues not caught by ABI compilation: a mistaken
SWP_NOSIZE flag and caption minimums imposed by DefWindowProc. The final code uses
NOMOVE/NOZORDER/NOACTIVATE and bypasses WM_WINDOWPOSCHANGING's caption constraint
**only inside a validated programmatic resize**, including initial client sizing.
Normal interactive tracking retains system limits. The native test proves exact
1x1 and 96x48 sizes, plus actual HWND destruction after owner Drop. The game
fixture collects unloaded section resources at the documented retirement boundary.
Own failed runs 41793934111/41794523640/41794968040/41795204955 are not qualification
results; the final run above includes the fixes. Failure output is now captured
in own-branch check annotations as well as logs, since sandbox egress cannot fetch
the redirected Actions log blobs.
EGL resize compiles but has not been runtime-qualified here. Relative/raw mouse,
fullscreen/DPI policy, gameplay bindings/timing, normal Game backend selection,
remaining sky/material passes and Win7 toolchain/PE/CRT/ISA/ICD remain incomplete.


## Actual game frame preparation (not default Game startup)

`legacy_gfx::frame::Prepared::new` now consumes the real `gfx::Frame`, drawable
size, explicit visible solid/translucent section lists, `Limits`, and brightness/
fullbright options. `TerrainHudRenderer::render_game_frame` prepares these inputs,
updates the lightmap only when its pixels change, and calls the existing typed
geometry/HUD coordinator. Visibility, uploads and section-internal translucent
sorting are still the caller's job. No alternate game/mesher types or assets are
introduced. This is a flat-screen classic-SDR subset, **not a playable backend**.

* World lens/rotation use `gfx::View::screen`; bob and spin retain the exact game
  multiplication order. The hand uses `camera::hand_projection`, bob and view,
  without world spin or world FOV. `gl_reversed_projection` maps near=w/far=0 to
  near=-w/far=w, so the existing GL LESS/depth-clear-1 contract is correct. The
  older `gl_projection` is ONLY for non-reversed source depth; they are not
  interchangeable.
* Visible static block-entity snapshots are folded into the classic CPU stream
  in visible-list order, before dynamic entities. Section-local position plus
  `(origin - camera)` is evaluated in f64 before conversion. Invisible geometry
  is not copied/charged; missing snapshots mean no static model, as in the game.
  Duplicate visible keys/snapshots are errors. With no visible static geometry,
  the dynamic stream is borrowed instead of cloned. Static + dynamic + hand +
  see-through + generated lightning + source additive + weather vertices share the same
  bounded entity budget; scratch allocation is fallible. This does not yet add resident static-model GPU caching.
* `lightmap::pixels` is the original CPU calculation extracted unchanged from
  wgpu's `update_lightmap`. Both backends now call it, including night vision,
  effective gamma, flicker, darkness, boss darkening and fullbright. The GL
  texture and CPU entity-light sampling receive the same 1024 RGBA8 bytes.
* `fog::world_parameters` also shares the existing game's override/water/default
  selection. `Fog::game` adds cylindrical render-distance ranges and color alpha
  to spherical environmental fog. GLSL uses
  `max(linear(spherical), linear(max(length(xz),abs(y)))) * alpha`, with the same
  vertex-distance interpolation and <=start / >=end branch order as WGSL.
  Equal ranges support steps and the disabled `(MAX,MAX)` pair without 0/0.
  `game: None` retains the original strictly-increasing spherical-only contract.
  HUD/hand keep the coordinator's existing independent no-fog views.

The actual-frame entry point checks camera/projection arithmetic, current HUD
size (rebuild after resize), fog and classic lightmap parameters, glint, counts,
visibility duplicates and explicitly requested missing passes before submission.
Unsupported: panorama, visible sky/End sky, clouds/celestial/stars,
lines/outlines, GPU skin frames (caller must request CPU
poses), glowing silhouettes, dynamic/colored lights, emissive layers,
border, post/menu blur, chunk fade and water mirrors. These are **errors,
not automatically disabled options**. Consequently normal Overworld/title frames
are not yet accepted; a no-sky classic frame without these passes is supported.
Shadow/motion/pack-uniform metadata alone does not request an additional classic
pass. Unsupported materials remain rejected by the typed bridges before clear.
CPU rejection preserves the framebuffer; lightmap/resource state is not a
transaction, and native submission failures still permit partial frames.

The eleventh actual-type GPU fixture calls `render_game_frame` with perspective,
a camera at x=30,000,000.25, resident terrain, visible static and dynamic entities,
shared half-intensity lightmap, fog override with alpha/equal ranges, independently
projected hand after a depth clear, and HUD. It also compares the entire previous
framebuffer after unsupported-sky, malformed-quad and missing-section failures.
Four CPU frame tests and one shared-lightmap test exercise matrices/near/far,
pose order, f64 precision, borrowed geometry, combined limits, fallbacks,
parameter rejection and resized/stale HUD. The independent mandatory outcome
`backend_game_frame_inputs` covers both fog distances winning, alpha 0/.25/.5,
equal/disabled ranges, invalid-input preservation, overlay isolation and real
reversed-depth occlusion in all three Mesa profiles.

Local: **37 standalone Rust tests**, host/i686 Clippy, **3x22 OSMesa outcomes**,
**642 Python tests (1 skipped)**. Own branch run
[41799081051](https://github.com/lukintrees/horizon/actions/runs/41799081051)
at `e568d92` passed **all three jobs**: the four frame + one shared-lightmap CPU
tests, **eleven actual-game GPU fixtures**, full-client check, game Clippy,
independent runtime matrix, **36 executed x86 Windows workspace tests** and
ordinary x86 probe linking. CPU/Clippy failure diagnostics are also surfaced as
own check annotations instead of depending on redirected log-blob downloads.
Win7/X1650, remaining passes, game startup/resource/input scheduling, PE/CRT/ISA
and a complete playable executable remain unqualified.


## Actual camera overlays: sprite-local screen sampling

`render_game_frame` now accepts the game's `ScreenQuad` list and draws it **after
world/particles/see-through/hand and before the HUD**, in source order. This
implements the classic SDR path for water, block/fire overlays, vignette,
pumpkin/powder-snow/portal overlays emitted by `screen_effects`. `world_tint`
changes the HDR path only; in this explicitly SDR backend it has the ordinary
screen-alpha meaning. Sky/panorama and other requested missing passes are still
errors; enabling screen effects does not make normal Game startup playable.

`legacy_gfx::screen::Prepared` validates every quad, common per-quad rectangle,
finite clip/local UVs, [0,1] color and sampling flags. Sprite rectangles must
recover integer texel boundaries from atlas-normalized f32 values (within .01
texel); arbitrary subtexel crops are rejected, not rounded into a different
image. Native maximum texture size is checked before world clear.

Sprites are extracted from the compressed virtual HUD backing, decoding only
one bounded page at a time and copying **interiors, never page gutters**. This
works across virtual page boundaries and does not reconstruct the giant atlas.
Identical rectangles share one CPU image per frame. There is no stale persistent
sprite-image cache: current HUD patches are applied before extraction, and atlas
reloads are reflected in the next preparation. Default limits are **256 screen
quads / 8 MiB extracted pixels plus one decoded page**; descriptor arrays are
separately bounded by the quad count. GPU texture/mesh payload budgets still
apply independently. A sprite exceeding either profile or native limits is an
explicit error before submission.

`Pass::Screen` uses an independent sprite texture and its own per-draw
nearest/linear + repeat/clamp state. Local UVs remain unwrapped through
interpolation; hardware addressing/filtering reproduces sprite-edge/seam
sampling instead of repeating the whole atlas or baking `fract` at vertices.
General textures now accept NPOT dimensions (desktop GL2 core; no mipmaps),
without resampling/padding the sprite or changing its repeat period. Terrain and
virtual GUI **pages** retain their existing POT layout. Runtime qualification
now includes an actual 3x2 NPOT repeat/linear readback, not merely a version-string
assumption. Real ATI/Win7 qualification is still required.

The screen shader exits before lighting/fog/glint/GUI alpha cutoff. Alpha effects
discard only nonpositive alpha; vignette outputs its RGB even for zero texture/
tint alpha and uses ZERO/ONE_MINUS_SRC_COLOR, preserving destination alpha.
Ordinary GUI/terrain/entity draws reset screen mode and sampler state. A screen
texture may alias the unused lightmap: its binding must not override image
sampling parameters. The existing ordinary-vignette alias restriction remains.

Before clear, the coordinator validates CPU inputs and available space for the
largest sprite, a 188-byte screen mesh, and a virtual GUI page when needed.
GUI pages are released before world-budget sizing and between hand and screen.
Only **one screen image + one mesh** are live on the GPU at a time; they are
released before HUD paging resumes. Thus image and GUI-page peaks are max(), not
sum(), and the original atlas quota cannot silently overcommit. Failed native
cleanup IDs remain owned for retry/Drop. CPU rejection preserves the framebuffer;
native upload/draw/delete failures still have no whole-frame rollback guarantee.

The mandatory `backend_screen_sampling` outcome checks 160 independent RGBA
samples across all 8 blend/sampler combinations on a 3x2 image, transparent
vignette samples, ignored lighting/fog, sampler aliasing, reset to an ordinary
overlay, tiny positive alpha and complete resource release. Two CPU game tests
cover all **420** subrectangles of a 7x5 atlas across page seams, deduplication,
patches and invalid/budget inputs. The twelfth actual-game GPU fixture drives
real `gfx::Frame::screen`, uses all four sampling modes, proves hand→screen→HUD
order, patches/reloads, full-frame rejection preservation, and exact **1156
texture / 516 peak mesh bytes** (the paged hand has six 84-byte vertices, not four).

Local: **38 standalone Rust tests**, host/i686 Clippy, **3x23 OSMesa outcomes**,
**642 Python tests, 1 skipped**. Own branch run
[41801421945](https://github.com/lukintrees/horizon/actions/runs/41801421945)
at `c076df5` passed **all three jobs**, including both new CPU tests, **12
actual-game GPU fixtures**, full-client check/game Clippy, the independent runtime
matrix, **37 executed x86 Windows workspace tests**, and ordinary probe linking.
The initial GPU attempt correctly rejected an undersized hand-mesh test budget;
the final fixture budgets the actual paged six-vertex stream. This is not
Win7/X1650 or complete gameplay qualification.


## World lightning and explicit additive geometry

The actual `gfx::Frame` adapter now accepts `lightning::Input` and `Frame.additive`.
Lightning reuses the game's unchanged deterministic seed generator: 224 quads /
896 vertices per bolt, the same UV, packed color and fullbright material. Each
`bolt.at - camera` subtraction happens in f64, followed by conversion to f32 and
addition of the local generated f32 vertex. Input bolt order is retained; source
additive quads follow the bolts. No persistent legacy seed cache is claimed.
The default cap is 64 bolts. Generated vertices and source additive vertices
share the existing combined static/dynamic/hand/see-through entity ceiling.
Count arithmetic, finite translations and temporary output reservation are
checked before any framebuffer clear; generation uses one bounded bolt template
at a time. Invalid positions/materials and both CPU/GPU budget failures remain
explicit errors rather than silently omitting effects.

`entity::additive_runs` is a separate material view, not permission to submit
these materials through the ordinary body/hand path. It accepts only source
codes 0, 2, 3 and 37, preserving source order and merging only adjacent identical
pipeline policies. Textured codes 0/3 retain CPU lightmap sampling; code 2 is
unlit. They ignore overlay/glint/cutout/back-color correction and world fog,
matching the game's classic additive shader. Code 37 (dragon death rays) is
unlit and ignores texture, lightmap and UV entirely. Its spherical/cylindrical
fog ramp fades **both RGB and alpha**, independently of fog-color alpha.

Both policies use `SRC_ALPHA, ONE` for RGB **and alpha**, and LEQUAL depth.
Textured additive draws are two-sided without depth writes. Rays use back-face
culling and **do write depth**, including when source alpha is zero or the fog
fade completely extinguishes the color. The native shader resets its additive
mode for every draw, so later ordinary fogged entities cannot inherit it.
The coordinator submits body/text/shadow/item/water-mask first, then lightning +
source additive, translucent terrain, particles, weather, see-through text, hand, screen
effects, and HUD. All additive mesh/page reservations join pre-clear preflight.
There is no new hardware feature beyond OpenGL 2.0 / GLSL 1.10.

The required `backend_world_additive` readback covers RGBA blend factors, lightmap
versus unlit behavior, no textured fog/cutout, ray fog/texture independence,
culling, zero-alpha ray depth, and ordinary shader-state reset. The additional
actual-game GPU fixture renders seeded bolts near a 30-million-block coordinate,
checks duplicate-seed instances and full-fog independence, proves translucent
terrain overlays lightning (coverage comes from texture alpha, not mesher HDR
flags), and checks ray depth/fog and pre-clear rejection. Own CI [41805814823](https://github.com/lukintrees/horizon/actions/runs/41805814823) at `083bb8e` passed all three jobs: full-client
check, game-bridge Clippy, the new CPU coverage, **13 actual-game GPU fixtures**,
**3x24 independent runtime outcomes**, **39 executed x86 Windows workspace
tests**, and ordinary x86 probe linking. Local verification also passed 40
standalone Rust tests, host/i686 Clippy and 642 Python tests (one skipped).
Earlier runs exposed a test-only terrain-coverage mistake and current-Clippy
fixture issues; the final run includes those fixes.

Sky/panorama/End/celestial/stars/clouds and the other explicitly rejected
passes remain unsupported. This does not wire the normal wgpu-based game startup
to the legacy backend and is not X1650/Windows 7 runtime qualification.


## Actual rain and snow (2026-11-04)

`render_game_frame` now consumes the game's unsplit `weather::Vertex` stream and
masked `weather::Time`, including real rain and snow animation metadata. No
alternate gameplay precipitation generator is introduced. Positions are already
camera-relative and pass through unchanged. Rain preserves f32 operation order
and signed remainder; snow reconstructs the source f64 random bases before the
f64-base-plus-f32-drift rounding. This avoids requiring shader int64/doubles or
additional attributes on GL2. The CPU phase is added to each UV before upload,
**without wrapping or tiling on the CPU**.

Weather rectangles are origin + extent, not the screen path's origin + end.
After glyph/image patches, each distinct sprite is extracted from compressed
virtual-atlas pages, including sprites crossing page seams. One image is kept
across adjacent quads sharing a sprite; switching sprites never reorders quads.
There is no atlas-wide repeat or padding bleed. GLSL 1.10 applies `fract` AFTER
interpolation and samples the extracted image with NEAREST/CLAMP, matching the
original clamped integer-texel selection even for NPOT sprites and negative UVs.
The ordinary entity/HUD modes reset that behavior on every draw.

The policy is two-sided, LEQUAL, no depth writes, source-alpha RGB blending and
ONE/ONE_MINUS_SRC_ALPHA alpha blending. The 0.1 alpha cutout runs **before** CPU
lightmap multiplication. Weather ignores back-face color correction, receives
the shared bilinear RGBA lightmap and ordinary spherical/cylindrical world fog
(including fog-color alpha). It follows translucent terrain and particles,
precedes see-through text, first-person hand, screen effects and HUD.

The default caps are 4096 quads and 8 MiB for retained extracted sprite bytes +
one decoded backing page. Source weather vertices also reserve space in the
combined entity/hand/additive budget BEFORE page-clipping expansion of other
streams. Malformed clocks, mixed metadata, invalid snow bases/UV arithmetic,
subtexel/oversized crops and CPU/GPU capacity errors are rejected before clear.
Weather streams one **348-byte lit mesh** and one sprite at a time, then releases
both. The native resource owner retains failed cleanup handles for retry/Drop.
Screen and weather images do not coexist with the streamed GUI page cache.

`backend_world_weather` is required in all three GL modes: repeating 3x2 NPOT
RGBA sprites, negative/interpolated UVs, two-sided color, pre-light cutout,
lightmap/partial fog, RGBA blend factors, LEQUAL, no depth writes, ordinary
shader reset and complete cleanup. Three actual-game CPU tests check animation
against the existing independent column reference (negative/large ticks and
snow rounding), page seams/patches/dedup/source order and invalid inputs.
The fourteenth actual-game GPU fixture checks moving rain/snow, repeated and
switched sprites, translucent → weather → text/hand → screen → HUD order,
patch/reload visibility, exact capacity boundaries and framebuffer preservation.
It uses only 1156 texture bytes, including fixed textures + one cached HUD page,
and rejects 535 mesh bytes where 188 resident + 348 transient bytes are needed.

Local: 40 standalone Rust tests, host/i686 Clippy, **3x25 llvmpipe readbacks**,
642 Python tests (one skipped). Full-game startup/platform/resource wiring,
sky/End/panorama/celestial/stars/clouds and the remaining rejected passes are
still unfinished. Neither this nor ordinary Windows x86 linking qualifies
execution on Windows 7 32-bit / Radeon X1650.

Final own CI [41808986326](https://github.com/lukintrees/horizon/actions/runs/41808986326) at `866c613`: **all three jobs passed**,
including all three new weather CPU tests, **14 actual-game GPU fixtures**,
full-client check/game Clippy, **3x25 independent runtime outcomes**,
**39 executed Windows x86 workspace tests** and ordinary x86 probe linking.
The initial weather implementation also passed own CI 41808013392; this final
run additionally verifies adjacent-image reuse and repeated/switched sprites.
The following docs-only commit changes no tested code.
