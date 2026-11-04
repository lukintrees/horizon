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
- GL2-only ABI, GLSL110 programs, ordinary uniforms, 4 vertex attributes, 2 texture
  units. No VAO, UBO, SSBO, instancing, arrays, compute, FBO or GL3 requirement.
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
18 backend checks execute real uploads/draws/readbacks: texture quadrants, texture
updates and bounds, depth, cutout holes, ordered alpha blend with depth writes off,
overlay/scissor reset, lightmap, fog, large coordinates, validation/budgets,
context isolation, resource reclamation, and the shared terrain-atlas conversion
contract, resident-section replacement/rollback, eviction, lightmap updates,
cleanup after partial construction, GUI compositing/depth/glyph updates, and
paged GUI streaming with linear gutters, depth continuity and one-page eviction. The runner requires all 18 outcomes; a triangle-only report cannot pass.
Report schema 2: `target/legacy-gl-probe/report.json`, ignored by Git.

The terrain end-to-end fixture uses the real CPU contract and renderer with
original synthetic pixels, **not stand-in game/mesher types**. The tests in `horizon-client-render::{render_bridge,gui_bridge}` target actual
game types. They now compile/run in the branch-only GitHub workflow, including
a real `HudAtlas::boot`/`HudBatch` GL2 readback (no Mojang resources).

## Verified / blocked (2026-11-04)

Verified here: stable Rust 1.88.0, 24 workspace unit tests plus 2 probe unit tests;
Linux and Windows-x86 compile-only Clippy; real OSMesa Mesa 23.1.4 / llvmpipe
LLVM 17.0.6, all three modes, 18 backend checks per mode. Python regression suite:
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
playable title/world/inventory path, full GUI glint/portal/CIT support, entities/hand,
CPU poses, resource animation/reload integration, streaming upload scheduler,
resize/event-loop integration, mipmapped/repeating sprite UVs, or final culling.
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
Portal, vanilla/CIT glint and pack-text shaders are not silently drawn as plain
atlas quads. Large HUD atlases now use bounded POT pages rather than a single
oversized texture; the upstream full-image CPU atlas construction still needs
Win7 address-space qualification.
Ordinary entity/item/particle and hand paths are now integrated (below); sky,
remaining special entity materials and input/resize integration remain work.

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
Supported overlays: 0, 1, 5, 13, 14, 15, 16..31, 32, 35; HDR tier/OIT tags are ignored only as
they are in classic SDR. Other flags, glint/portal/crumbling/energy/dissolve
materials are rejected, not flattened into a normal textured quad.

Entity meshes have a checked 76-byte layout (base vertex, back colour, light
colour); ordinary terrain/GUI meshes stay 44 bytes. Typed mesh/pass mismatches
and malformed overlays are rejected before clears. Six attributes and GLSL110
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
