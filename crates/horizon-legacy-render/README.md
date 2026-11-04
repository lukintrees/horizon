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
15 backend checks execute real uploads/draws/readbacks: texture quadrants, texture
updates and bounds, depth, cutout holes, ordered alpha blend with depth writes off,
overlay/scissor reset, lightmap, fog, large coordinates, validation/budgets,
context isolation, resource reclamation, and the shared terrain-atlas conversion
contract, resident-section replacement/rollback, eviction, lightmap updates,
cleanup after partial construction, and GUI compositing/depth/glyph updates. The runner requires all 15 outcomes; a triangle-only report cannot pass.
Report schema 2: `target/legacy-gl-probe/report.json`, ignored by Git.

The terrain end-to-end fixture uses the real CPU contract and renderer with
original synthetic pixels, **not stand-in game/mesher types**. The tests in `horizon-client-render::{render_bridge,gui_bridge}` target actual
game types. They now compile/run in the branch-only GitHub workflow, including
a real `HudAtlas::boot`/`HudBatch` GL2 readback (no Mojang resources).

## Verified / blocked (2026-11-04)

Verified here: stable Rust 1.88.0, 17 workspace unit tests plus 2 probe unit tests;
Linux and Windows-x86 compile-only Clippy; real OSMesa Mesa 23.1.4 / llvmpipe
LLVM 17.0.6, all three modes, 15 backend checks per mode. Python regression suite:
641 tests, 1 skipped. Windows is type-checked, not linked/executed. Existing
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
Geometry is uploaded before any GUI draw, with cleanup on a staging failure.

`Renderer::owned` and the owning `WorldScene<B>` remove self-referential lifetime
requirements for storing this coordinator in a Game. Borrowed backends still work
through `RenderBackend for &mut B`. Every native operation remains thread-confined.

This is an explicit terrain/HUD subset, **not the normal game renderer selection**.
Portal, vanilla/CIT glint and pack-text shaders are not silently drawn as plain
atlas quads. Large HUD atlases still need paging to meet X1650 texture/VA limits;
POT padding is bounded and an oversized upload fails rather than shrinking assets.
The entities/hand/sky path and input/resize integration remain separate work.

Our own `.github/workflows/legacy-gl-branch.yml` triggers only on
`arena/a7e96628-horizon`. The first two runs passed both independent GL2 readbacks
and actual game bridge compilation/tests; the second additionally passed the
real boot-atlas/HUD GL2 test. Run 41773789614 also passed the owning coordinator, combined real mesh/HUD
readback, glyph patches, atlas reload and game-bridge Clippy. These are not tests on Win7/X1650.


Latest own run: [41774182610](https://github.com/lukintrees/horizon/actions/runs/41774182610)
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
