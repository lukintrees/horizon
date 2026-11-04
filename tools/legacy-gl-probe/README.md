# Legacy desktop OpenGL bootstrap probe

Stage 0 of the [X1650 / Win7 x86 plan](../../docs/design/opengl2-win7-x86.md).
This is **not a game renderer**. It needs no Minecraft resources, Java, wgpu,
Cargo registry downloads or real player data. It has Linux EGL/OSMesa and native
Windows WGL paths. **Only Linux runtime is verified**; Win7 PE32 linking/loading,
WGL execution, window presentation, input and ATI driver compatibility remain unverified.

The independent Cargo workspace avoids building the game's dependency graph. Unsafe
FFI now lives in the reusable `horizon-legacy-render` crate; the game's `forbid(unsafe_code)` policy is unchanged. Contexts
are thread-confined and outlive the loaded GL functions and framebuffer storage.
The tool is a diagnostic fixture using the new reusable renderer; it is not a game.

## Run

Install Rust >= 1.85 and a C linker. Install Mesa desktop EGL plus the software DRI
driver, or Mesa OSMesa (for example Debian's `libegl1-mesa`, `libgl1-mesa-dri`,
`libosmesa6`). Runtime libraries are loaded dynamically; development headers and
Xvfb are not required for these offscreen tests.

From the repository root:

```sh
cargo test --offline --manifest-path tools/legacy-gl-probe/Cargo.toml \
  --target-dir target/legacy-gl-probe
cargo clippy --offline --manifest-path tools/legacy-gl-probe/Cargo.toml \
  --target-dir target/legacy-gl-probe --all-targets -- -D warnings
python3 -m unittest discover -s tests -p test_legacy_gl_smoke.py

# Builds the probe, executes all three cases, writes an ignored JSON report.
python3 tools/legacy_gl_smoke.py                  # surfaceless desktop EGL
python3 tools/legacy_gl_smoke.py --context osmesa # in-memory desktop OSMesa
```

For a custom OSMesa installation, set `HORIZON_OSMESA_LIBRARY` to its library path
(default `libOSMesa.so.8`) and use `LD_LIBRARY_PATH` for its dependencies.
`--binary PATH` skips the build; `--output PATH` selects a report location;
`--timeout SECONDS` limits each runtime case. Nonzero exit means blocked or failed,
not passed. Default report: `target/legacy-gl-probe/report.json`.

The probe can also be invoked directly with `--context egl|osmesa|wgl`,
`--shader-version 110|120`, `--require-llvmpipe`, and `--expect-gl 2.0|2.1`.
The runner supplies the required Mesa environment; a direct invocation must set it.

## What is actually tested

Each case creates a desktop GL context, compiles and links shaders, uploads a VBO
and u16 EBO, draws an indexed green triangle, and reads pixels back. Independent
expected colors verify the clear color outside the triangle, the center pixel,
and depth rejection of a farther red triangle. A deliberately invalid shader must
be rejected; unexpected GL errors fail the test. Readback permits 2/255 rounding.

Before loading GL2 symbols, the probe rejects GLES, desktop GL below 2.0 and
GDI Generic with driver-specific diagnostics. The report records target OS and
pointer width, whether a window buffer swap succeeded, vendor/renderer, GL/GLSL
versions, depth bits, texture size,
texture units, attribute/uniform/varying limits, context type and binary SHA-256.
The runner rejects software renderers other than llvmpipe and validates every
mandatory check. It runs:

| Case | GL override | Shader source | Reported GLSL override |
| --- | --- | --- | --- |
| gl20 | 2.0 | `#version 110` | 120 |
| gl21 | 2.1 | `#version 120` | 120 |
| native control | removed | `#version 110` | removed |

**Why 120 for the first row?** Mesa 23.1.4 rejects
`MESA_GLSL_VERSION_OVERRIDE=110`: querying the language version returns null with
an implementation error. We observed this failure, not a successful GLSL110
runtime. A 120 ceiling still compiles explicit GLSL110 source and avoids that Mesa
bug. This is not proof of behavior on an old ATI GLSL compiler. Overrides also do
not remove all extensions or reproduce X1650 limits. The probe intentionally uses
only GL2 entry points, but full-game integration and ATI-specific limits remain future work.
Texture/atlas/transparency checks are now executed by the reusable backend suite
described below.

## Recorded development run (2026-11-04)

- Debian 12 x86_64, Mesa 23.1.4, `llvmpipe (LLVM 17.0.6, 256 bits)`, **OSMesa**.
- GL 2.0 + GLSL110 source: passed; GL 2.1 + GLSL120 source: passed;
  native GL 4.5 compatibility control + GLSL110 source: passed.
- 2 Rust unit tests and 5 Python runner tests passed; Clippy passed with warnings
  denied. No window/GLX/WGL or game was started. No X1650 FPS measurement was made.
- EGL runtime did **not** pass here: the available EGL bundle lacked system
  dependencies. OSMesa success must not be presented as EGL success.

Sandbox bootstrap provenance (development tooling only): rustc/Cargo/Clippy
`1.92.0-nightly` from npm's `@rusttonpm/*-linux-x64@0.92.20250925`; rustfmt from
`@rustbin/rustfmt-1.88.0-x86_64-unknown-linux-gnu@1.8.0` with matching rustc shared
libraries. npm tarball integrity was checked against registry SHA-512 metadata;
this is **not** independent upstream authenticity verification or a production
Win7 toolchain choice. Mesa runtime libraries came from the PyPI
`lavavu-osmesa==1.9.10` CPython311 manylinux_2_28 x86_64 wheel (Mesa/LLVM libraries
only, no application source used). Binaries/downloads stayed under ignored
`.local/legacy-gl/`; reports/builds under ignored `target/`. Nothing is bundled
into the game or copied into the public tree.

## Windows WGL bootstrap (added, not runtime-verified)

The WGL context owns a `CS_OWNDC` window, a double-buffered RGBA/depth pixel format
and a legacy `wglCreateContext` context. No EGL, GL3 core profile, modern context
extensions, winit, Vulkan or DX12 are involved. The window is briefly shown before
readback (hidden windows have no reliable pixel-ownership guarantee). Keep it
unobscured on the real test machine. No long-lived event loop or input test exists.

`wglGetProcAddress` null/1/2/3/-1 results are never called. The loader falls back to
`GetProcAddress` on the already loaded `opengl32.dll`, as required for GL1 exports
such as `glGetString`. Missing symbols are errors. Generic GDI pixel formats are
rejected with an ICD installation diagnostic. Context, DC, window and class are
released in reverse order on success or failure, on the creating thread.
After readback, `SwapBuffers` must succeed before a passed JSON report is emitted;
`buffer_swap` is false for offscreen Linux contexts and true for successful WGL.

Compile-only ABI check (also runnable on Linux with the corresponding target std):

```sh
cargo check --offline --manifest-path tools/legacy-gl-probe/Cargo.toml \
  --target i686-pc-windows-msvc
cargo clippy --offline --manifest-path tools/legacy-gl-probe/Cargo.toml \
  --target i686-pc-windows-msvc --all-targets -- -D warnings
```

On a modern Windows development machine with the MSVC x86 linker/SDK installed:

```powershell
cargo build --offline --manifest-path tools/legacy-gl-probe/Cargo.toml --target i686-pc-windows-msvc
.\tools\legacy-gl-probe\target\i686-pc-windows-msvc\debug\horizon-legacy-gl-probe.exe --context wgl --shader-version 110
```

**That standard target is NOT a Win7-supported artifact.** It is only a way to
check Win32 calling conventions, struct layout and linking. `cargo check` emits
no EXE and cannot validate imports or CRT deployment. A production Win7 build
still requires the dedicated `i686-win7-windows-msvc` std/toolchain strategy from
the plan, PE/DLL audit, then execution on Win7 x86 with the installed vendor ICD.
Don't run the Linux smoke runner on Windows: its Mesa overrides/llvmpipe requirement
are intentionally Linux-only. Run the Rust probe directly and preserve its JSON
and stderr. No `--require-llvmpipe` on a real X1650; use the default minimum GL2
qualification rather than forcing an exact reported GL version.

### Continuation checks (2026-11-04)

- 8 Rust unit tests passed: prior readback/report tests plus driver qualification
  and WGL sentinel/fallback outcomes. These tests are not a mocked WGL runtime.
- Linux and `i686-pc-windows-msvc` all-target Clippy passed with `-D warnings`;
  Windows x86 `cargo check` passed. Const guards check the raw Win32 ABI layouts.
- All three OSMesa/llvmpipe modes passed again; 5 Python runner tests passed.
- This run used stable Rust/Cargo/Clippy/rustfmt **1.88.0** components from npm's
  `@rustbin/*-1.88.0-*` mirrors (with registry integrity verification), including
  the ordinary i686 MSVC std. The dedicated Win7 target exists in that rustc's
  target list but its std is not installed. This is not a production toolchain lock.
- Added `.github/workflows/legacy-gl-probe.yml`: resource-free Linux OSMesa tests
  and a modern-Windows x86 build job, explicitly **not Win7 certification**.
  The workflow has not been executed on GitHub in this session.

Next: complete Win7 toolchain/PE32 and real-machine checks independently; proceed
with the backend-neutral frame/resource boundary on Linux. Coordinate terrain
work with existing draft PR #1 rather than duplicating it.


## Reusable backend milestone

Native context code now lives in [`horizon-legacy-render`](../../crates/horizon-legacy-render/README.md).
The probe borrows that context and tests the actual reusable renderer in addition
to its independent triangle test. Its safe `RenderBackend` contract is defined in
`horizon-render-api`; the game library has real terrain/GUI adapters and an opt-in terrain/HUD
coordinator, not yet the normal Game renderer.

All three llvmpipe modes now require **18 backend outcomes per mode**, including
real texture updates, cutout, transparency/depth write, lightmap, fog, clipped
UI-like overlay, large-coordinate rendering, padded atlas/terrain conversion,
context switching, resource validation and reclamation, and resident-section
replacement/rollback/eviction through `WorldScene`, GUI blend/depth compositing
and dynamic glyph patches including POT edge updates. Paged GUI checks enforce
a one-page GPU cache and verify alpha order, linear gutters, depth across
evictions, dirty-page reload and resource release. Reports include a
`backend_checks` list; runner schema is 2 and old triangle-only reports fail.
Run the 24 native/API unit tests independently (the probe itself has 2 tests):

```sh
cargo test --offline --manifest-path crates/horizon-legacy-render/Cargo.toml --workspace
```

For this continuation, 26 standalone Rust unit tests, host and x86 Windows Clippy, all runtime
modes and the Python suite (642 tests, 1 skipped) passed. Local full-game dependency resolution remains incomplete (287/488 exact packages
from pinned GitHub mirrors). Our branch-only Actions now compile and test the real
game bridges and run a real HudAtlas/HudBatch GL2 fixture. This is not a complete
Game build or gameplay smoke; prepared game resources are absent. No complete legacy game or Win7-compatible
EXE was built. See the renderer README for the explicit remaining integration work.


The branch-only workflow has now also linked the ordinary i686 MSVC probe on
modern Windows (own run 41774182610 passed). Its artifact is labelled
`probe-modern-windows-x86-not-win7-qualified`: no Win7-specific std/import/CRT or
real Radeon ICD qualification is inferred from that build. The same run checked
the full client with `legacy-gl` and ran three actual-game GL2 fixtures; default
Game renderer selection is still wgpu.


Paged HUD continuation: own runs 41776018494 and 41776564666 passed, including
full-client checking, CPU compressed-backing tests and actual-game large-atlas
readbacks. Run 41776564666 covers five opt-in actual-game GPU fixtures; a 16385x2
source atlas is addressed through one 512x512 resident GPU page. Backend selection,
upstream CPU atlas peak memory and real Win7/X1650 qualification remain unfinished.


The classic-entity continuation is verified by own run
[41779419733](https://github.com/lukintrees/horizon/actions/runs/41779419733)
at `7dd78a8`: all jobs passed, including six actual-game GPU fixtures and the
full-client check. `backend_paged_entities` adds real readbacks for two-sided
lighting/culling, hurt overlay before lightmap, interpolated per-vertex light,
constant-X 3D clipping, item/particle depth policy, camera-relative world-border
precision, typed-buffer/overlay rejection and transient cleanup. The coordinator
also draws the basic first-person hand with an independent depth/projection view.
Special entity materials, sky, startup selection and Win7 runtime qualification
remain incomplete. The report's positive Python fixture now includes the new
outcome; omission of any required backend outcome is explicitly tested.


The world-layer continuation passed own runs 41781489098 and 41782002701.
`backend_entity_layers` verifies faint shadows without depth writes, depth-only
boat masks with transparent/unused textures, polygon-offset/cullable text,
see-through text retaining world fog, and mask/bias/layout resets inside mixed
draw lists and across views. The game coordinator now has seven actual-game GPU
fixtures. Latest local totals are 26 standalone Rust tests, 3x18 llvmpipe outcomes
and 642 Python tests (1 skipped); Win7 hardware/runtime qualification is unchanged.


Final world-layer validation: own run
[41782569337](https://github.com/lukintrees/horizon/actions/runs/41782569337)
passed all three jobs at `5fa39c0`, including the GUI scratch-lifetime cleanup.
Markdown-only pushes are now actually excluded by the branch workflow's negative
path pattern; previous crate-wide globs also matched READMEs.


### Animated entity glint continuation

The entity-glint stage required **19 backend outcomes** in each GL2.0/GL2.1/native
llvmpipe process. `backend_entity_glint` runs the production renderer with an
independently repeating/linear third sampler and 84-byte entity vertices. It
compares bilinear/wrap colours and a 60-pixel UV gradient across atlas page seams,
armour image/clock selection, half/zero strength, hurt-before-light,
glint-after-light-before-fog, alpha restoration and pre-glint cutout. It also
rejects sampler aliases before clear, restores normal nearest/clamp state and
checks all transient resources are reclaimed. Original synthetic pixels only.

Local results: **27 standalone Rust tests**, host Clippy and **3x19 Mesa checks**.
The full-game coordinator adds an eighth GPU fixture for the real vertex stream,
world/hand, asset reload/rollback and exact page-budget reclamation. Own run
[41785733646](https://github.com/lukintrees/horizon/actions/runs/41785733646)
at `1ba6133` passed all three jobs, including all eight actual-game GPU fixtures,
full-client check/game Clippy and ordinary Windows x86 probe linking. Full local
Python regression: **642 tests, 1 skipped**. Startup is
still wgpu; GUI glint and Win7/X1650 runtime qualification are not complete.


### Inventory/GUI foil continuation

Current validation requires **20 backend outcomes per mode**, including
`backend_gui_glint`. The production GUI path uses a distinct checked 52-byte
vertex layout. Its fragment output follows `hud.wgsl`, not world glint: original
item alpha mask, .1 cutoff, squared strength/sample RGB, mask alpha, no tint/light.
It preserves ONE/ONE additive versus source-alpha blending and original model
depth groups across plain/foil changes, u16 chunks and atlas pages.

The fixture compares a 60-pixel repeating reference gradient across three pages
with only one resident page. It tests half/zero strength, half masks and cutout,
GUI/model scissor and shader-state transitions, both depth-material orders,
degenerate prefixes preserving pending clears, typed/alias rejection without
clearing, nonfinite UV rejection and final resource reclamation. The real-game
coordinator adds a ninth GPU fixture with dynamic patches, atlas/image reload,
invalid-input preservation and exact **1164 texture / 324 mesh bytes**.

Local results: **31 Rust tests**, host Clippy, **3x20 llvmpipe checks** and
**642 Python tests (1 skipped)**. Own run
[41791059747](https://github.com/lukintrees/horizon/actions/runs/41791059747)
at `6c87c53` passed all three jobs: nine actual-game GPU fixtures, full-client
check/game Clippy, GL2 runtime matrix and modern Windows x86 probe linking.
This remains an opt-in coordinator; startup and Win7/X1650 runtime
qualification are unfinished. CIT/portal are explicit errors, not plain quads.


### Drawable resize and native window continuation

The validator now requires **21 backend outcomes** in each mode.
`backend_surface_lifecycle` retains mesh/texture IDs through 96x48, 31x79, 1x1,
128x32 and back to 64x64. It checks redraws, texture updates, size/readback bounds,
invalid/same-size framebuffer preservation, fixed payload counters and cleanup.
OSMesa backing is replaced only after glFinish; this is not a context rebuild.
EGL replacement/retirement is implemented and compile-checked, not runtime-verified
in this sandbox. Native surface memory is outside tracked texture/mesh payloads.

The WGL owner adds bounded event pumping, in-place client resize, Unicode title,
resize/focus/close, physical keys/repeat counts, UTF-16 text, signed pointer/button
and wheel events, and pointer capture/loss. Close does not destroy a drawable
behind the renderer. Offscreen polling is empty; WGL minimization reports zero
size, and render/present suspend until restored. A poll count bound is not a hard
latency bound: Windows modal moving/sizing can block DispatchMessage.

Local checks: **36 Rust tests**, host/i686 Clippy and **3x21 llvmpipe outcomes**.
The game suite adds its tenth actual-type GPU fixture retaining terrain/pages/
foil and rebuilding HudBatch after resize. Own Windows CI now executes x86 CPU
and a hidden two-HWND message/resize/close test **without a GPU context**, in
addition to linking the ordinary WGL probe. Own run
[41795980600](https://github.com/lukintrees/horizon/actions/runs/41795980600)
at `3339003` passed all three jobs: ten actual-game GPU fixtures, full-client
check/game Clippy, runtime matrix, **35 executed x86 Windows workspace tests**
and ordinary x86 probe linking. Full local Python suite: **642 tests, 1 skipped**.
Native tests exposed/fixed SWP_NOSIZE and caption-size constraints; programmatic
resize now scopes its WM_WINDOWPOSCHANGING bypass without weakening interactive
tracking limits. The native test proves exact 1x1/96x48 and HWND teardown on Drop.
None of this is Win7/X1650 runtime or complete gameplay qualification;
relative/raw mouse, fullscreen/DPI policy and the normal game loop are still work.


## Game-frame depth and dual-distance fog

The validator now requires **22 backend outcomes** per GL2.0/GL2.1/unrestricted
profile. `backend_game_frame_inputs` independently verifies conventional LESS
occlusion after converting reversed source depth; spherical and cylindrical fog
winning separately; fog color alpha; equal/disabled range branch order; rejected
invalid ranges/alpha without clearing; and unfogged overlays. Shader source stays
GLSL110/120 and uses no GL3 feature. Local **3x22** llvmpipe checks, **37 standalone
Rust tests**, host/i686 Clippy and **642 Python tests (1 skipped)** pass.

The game crate additionally has an actual `gfx::Frame` preparation/submit entry
point, shared wgpu/GL2 CPU lightmap and fog-input selection, static snapshot
translation/limits, and an eleventh real-type GPU fixture. See the
[renderer contract](../../crates/horizon-legacy-render/README.md#actual-game-frame-preparation-not-default-game-startup).
It rejects missing requested passes rather than pretending an incomplete frame
is a working game. Title/normal sky, remaining effects and default Game
startup/resource/input wiring are still unfinished. These checks do not certify
Win7, the X1650 driver, a compatible full-game binary, or gameplay.

Final own-branch verification:
[41799081051](https://github.com/lukintrees/horizon/actions/runs/41799081051)
at `e568d92`, **all three jobs passed**: four frame + one shared-lightmap CPU
tests, eleven actual-game GPU fixtures, full-client check/game Clippy, runtime
matrix, **36 executed x86 Windows workspace tests**, and ordinary probe linking.
The subsequent documentation-only commit does not change this tested code.


## Camera-overlay sampling

Required outcomes per profile: **23**. The new `backend_screen_sampling` uses a
real 3x2 NPOT image and an independent CPU bilinear/addressing oracle: 160 RGBA
samples across repeat/clamp, nearest/linear and alpha/vignette, including zero
alpha vignette color, ignored light/fog, sampler alias/reset, tiny positive alpha
and cleanup. This uses only desktop GL2/GLSL110, no mipmaps or GL3 feature.

The actual-game coordinator additionally consumes `gfx::Frame::screen`, extracts
sprites from compressed virtual-atlas pages after patches, streams one GPU image
and mesh between hand and HUD, and explicitly bounds decoded/payload storage.
Two CPU tests and a twelfth actual-game GPU fixture cover seams, patches/reload,
all sampling modes, compositing order, exact budgets and framebuffer preservation.
Local **38 Rust / 3x23 llvmpipe / 642 Python (1 skipped)**, host/i686 Clippy pass.
Normal Game backend selection, sky/panorama/other missing passes and Win7/X1650
qualification remain unfinished; no playable port is claimed.

Final own CI [41801421945](https://github.com/lukintrees/horizon/actions/runs/41801421945)
at `c076df5`: **all three jobs passed**, including the two new CPU tests, twelve
actual-game GPU fixtures, full-client check/game Clippy, 3x23 runtime matrix,
**37 executed x86 Windows workspace tests**, and ordinary x86 probe linking.
The subsequent documentation-only commit leaves the verified code unchanged.
