# D01a — Legacy GL dependency/feature audit

**Task-ID:** D01a
**Variant:** D01a dependency inventory + minimal ordered manifest requests (documentation only)
**Scope:** this document only; no Cargo manifest/lockfile, facade/contract, central Game/GL wiring, workflow, or policy-script changes.
**Depends-on:** C01/R01 before implementing renderer cuts; P01/P02 before changing the window/input stack.
**Integration base / baseline SHA:** `arena/a7e96628-horizon` / `fa703a67f635999f1f09a6ca669ee48e7bc12c97`.
**Review owner:** coordinating Arena.ai integrator. Issues are disabled; this PR is the scoped work record.

> This is a manifest- and call-site-backed audit, **not** a Cargo-resolved tree dump or a Cargo patch. The current shell has neither `cargo` nor `rustc`; exact resolver feature unification, transitive registry target closure, and builds therefore remain unverified. Findings below distinguish the checked-in manifest graph from that missing evidence.

## 1. Graph variants and result

The client package is `horizon-client` (`crates/horizon-client/Cargo.toml`). It has default features `audio` and `controllers`, but **does not define a `legacy-gl` feature**. The current native backend feature is owned by `horizon-client-render`:

```toml
# crates/horizon-client-render/Cargo.toml
legacy-gl = ["dep:horizon-legacy-render"]
```

Therefore the intended comparison is:

```sh
# Ordinary player client, target-specific Windows-x86 edges visible
cargo tree -p horizon-client --target i686-pc-windows-msvc -e normal,build,features

# Existing no-default baseline (not legacy-only)
cargo tree -p horizon-client --no-default-features --target i686-pc-windows-msvc -e normal,build,features

# Legacy renderer compiled into the client, defaults off
cargo tree -p horizon-client --no-default-features \
  --features horizon-client-render/legacy-gl \
  --target i686-pc-windows-msvc -e normal,build,features

# Useful inverse checks after each integration cut
cargo tree -p horizon-client --no-default-features \
  --features horizon-client-render/legacy-gl \
  --target i686-pc-windows-msvc -e normal,build,features -i wgpu
cargo tree -p horizon-client --no-default-features \
  --features horizon-client-render/legacy-gl \
  --target i686-pc-windows-msvc -e normal,build,features -i winit
```

The static direct/first-party graph in the manifests is:

```text
horizon-client
├── horizon-client-lib (default-features = false; root forwards audio/controllers only when enabled)
│   ├── horizon-client-render (modern render deps are unconditional)
│   ├── horizon-client-input (winit is unconditional)
│   ├── horizon-hdr, wgpu, winit, naga, pollster (unconditional)
│   └── gameplay, network, mod, telemetry, resource and server subsystems (unconditional)
├── horizon-client-render (unconditional; wgpu + winit + horizon-hdr are unconditional)
├── horizon-client-input (unconditional; winit + arboard are unconditional)
├── horizon-hdr (unconditional; its own wgpu/Vulkan edge is unconditional)
├── wgpu + winit + pollster + naga (unconditional direct edges)
└── platform edges (see §4)

horizon-client-render --feature legacy-gl --> horizon-legacy-render
```

| Graph | Feature delta established by manifests | What remains |
|---|---|---|
| Normal `horizon-client` | Defaults activate `audio` and `controllers`. Root `audio` forwards to `horizon-client-render/audio` and `horizon-client-lib/audio`; root `controllers` activates optional `gilrs`, `horizon-dualsense`, and controller support in client-lib/input. | Modern wgpu renderer, HDR, winit, shader systems, and the rest of Game remain in the build graph. |
| `--no-default-features` | Removes those default audio/controller activations, including cpal/gilrs/DualSense paths. | It does **not** disable `wgpu`, `winit`, `horizon-hdr`, `naga`, `pollster`, or the game's UI/gameplay. The same `horizon-client` binary still handles both client and `--dedicated` modes. |
| `--no-default-features --features horizon-client-render/legacy-gl` | Adds the optional `horizon-legacy-render` dependency and compiles legacy modules. | It still also compiles/includes the modern renderer and Game window stack; the `legacy-gl` feature does not cut modern dependencies or select legacy startup. |

This is sufficient to reject “`--no-default-features` means legacy-only.” It is also consistent with the source: `horizon-client/src/game.rs` still stores `Option<Renderer>`, creates `Renderer::new` through `pollster`, uses winit `ApplicationHandler`/`EventLoop::run_app`, and has wgpu-specific diagnostics. The `renderer_facade` in `horizon-client-render` has Modern and feature-gated Legacy adapters, but its module is not yet a replacement for the Game-owned modern renderer. See `crates/horizon-client-render/src/renderer_facade.rs` and `src/lib.rs`.

**Documentation inconsistency to resolve centrally:** `docs/server.md` §3 / §“Binaries” says the no-default `horizon-engine` links no wgpu/winit, while the current target manifests declare those dependencies non-optional and Game/client-input call sites use them. Cargo is unavailable here, so I cannot tell whether some undocumented linker/build behavior is intended; do not rely on that statement as evidence of a no-wgpu graph. The integrator should reconcile the docs and exact artifact after a tree/build is available.

## 2. Backend and modern-window closure

| Manifest edge / consumer | Evidence in this baseline | Cut request / guard |
|---|---|---|
| `crates/horizon-client/Cargo.toml`: direct `wgpu`, `winit`, `pollster`; `crates/horizon-client-lib/Cargo.toml`: same; `crates/horizon-client-render/Cargo.toml`: same | `client/src/game.rs` owns event-loop/startup and `Renderer`; `client-render/src/gfx.rs` is extensively wgpu-typed; `graphics.rs` exposes `wgpu::AdapterInfo`; `game/debug_keys.rs` and `game/control.rs` use wgpu enums; `client/src/mod_api.rs` re-exports both `wgpu` and `winit`. | After C01/R01 and Game migration to the agreed facade/platform contract, request one coherent `modern-render` dependency/feature boundary across these manifests. Do not add an independent client-local renderer API or simply make a still-referenced crate optional. Keep the modern default build unchanged. |
| `crates/horizon-client-render/Cargo.toml`: `horizon-hdr`; `crates/horizon-client/Cargo.toml` and `horizon-client-lib/Cargo.toml`: also direct `horizon-hdr` | `client-render/src/gfx.rs` calls `open_vulkan_device`, `set_metadata`, and Wayland watch; `gfx/hdr.rs`, HDR calibration/UI and WGSL contain the modern HDR path. `horizon-hdr` itself depends on wgpu Vulkan and (non-macOS) ash. | Gate/move this modern-only path with the modern adapter after R01. First prove no Game/common consumer remains; preserve the default HDR path and its HDR metadata/calibration tests. Legacy GL2 SDR must neither create wgpu types nor link HDR's Vulkan/ash stack. |
| `crates/horizon-client-render/Cargo.toml`: normal `naga` + shader deps; `crates/horizon-client-lib/Cargo.toml` / client manifest: direct `naga` | `gfx.rs`, `post_chain.rs`, `menu_blur.rs`, shader-pack code and `horizon-shaders`/`horizon-shader-api` use Naga/shader translation. Some uses are tests, but the render manifest explicitly makes Naga a normal dependency and the game exposes shader packs. | Separate modern shader-pack compilation from shared asset/CPU contracts only after R01. Do not turn all Naga usage into dev-only by assumption. Regression: shader-pack parse/compile/modern rendering tests and no shader-pack capability on GL2. |
| `crates/horizon-client-render/Cargo.toml`: direct `winit`; `crates/horizon-client-lib/Cargo.toml` and root manifest: direct `winit`; `crates/horizon-client-input/Cargo.toml`: direct `winit` + `arboard` | `client/src/game.rs`, `game/event_loop.rs`, `game/split.rs` use `ApplicationHandler`, event loop, `WindowEvent`, and `Window`; `client-input/src/{window,input,keybinds,clipboard}.rs` and numerous screen/control call sites use winit event/key/window types. | Keep until P01/P02 and Game loop are on the agreed platform contract. Then request target platform adapters and remove the winit dependency edge only where no public/input consumer remains. Preserve Unicode/IME, resize/focus, mouse capture, fullscreen, clipboard and split-screen modern tests. `arboard` is an independent clipboard edge, not automatically removed with winit. |
| `crates/horizon-client-render/Cargo.toml`: `pollster`; root client manifest: `pollster` | Game startup uses `pollster::block_on(Renderer::new(...))`; renderer GPU initialization and some tests also use it. | Candidate to move with the modern renderer after C01/R01; check for any shared startup/blocking use before removing. |

The legacy renderer crate itself is small and backend-independent in its manifest (`horizon-render-api` only). The problem is its **consumer closure**: enabling `horizon-client-render/legacy-gl` only adds that crate, while `horizon-client-render/src/lib.rs` continues to compile/export `gfx`, `graphics`, `post_chain`, `menu_blur`, etc. unconditionally. The modern surface is therefore still a hard dependency even when the legacy adapter is compiled.

## 3. Runtime, build-only, and genuinely optional subsystems

These categories must not be mixed when estimating the final PE/runtime closure.

| Category / current edge | Consumers and current behavior | Minimal request / regression evidence |
|---|---|---|
| **Runtime: core client/game** — assets, world, core, protocol, items, client input/render | Used throughout `game.rs`, save/loading, world simulation, screens, and network paths. | Keep in first legacy game artifact. A legacy renderer is not a reason to remove gameplay or persistence crates. Test the B2 menu/world/edit/save/reopen path. |
| **Runtime: launcher/mod content/native API** — `horizon-mods` is unconditional in root and client-lib, explicitly with `features=["index"]`; `horizon-mods/Cargo.toml`'s `index` activates optional `wasmparser`, `sha2`, `minisign-verify`, `horizon-shader-api` | `client/src/main.rs` embeds the mod API marker; `game/mods.rs`, `mod_content.rs`, `mod_api.rs` implement content/native hooks; render `mod_list.rs` persists enablement. The dependency comment mentions a `mods` feature, but neither client manifest defines that feature. This is not an absent/default-off subsystem in the build graph. | Do not drop `horizon-mods` wholesale. Later request: split content/native API from registry/index/WASM support, and only gate the latter after Game/launcher/native-mod consumers have a defined off behavior. Tests: ordinary no-mod world, enabled content mod, marker/native bundle validation, launcher registry compatibility. |
| **Runtime: shader packs** — `horizon-shaders`, `horizon-shader-api` direct in root/client-lib/render | `game/shader_packs.rs`, render shader options/list, `gfx.rs`/`gfx/native_*` consumers; facade advertises `Feature::ShaderPacks` for Modern and none for Legacy. | Good post-R01 cut candidate, but split pack UI/translation from shared vanilla shaders and existing public mod/API surface. Test modern pack load/reload; legacy capability/error; vanilla shader/material render remains. |
| **Runtime: friends networking** — `horizon-friends` unconditional in root and client-lib | `net.rs`, `game/friends.rs`, multiplayer/invite UI use invites, NAT traversal/relay/tunnel. Ordinary protocol/networking is separate. | Candidate standalone `friends` feature only if disabled UI/invite routes fail explicitly and standard server/LAN/direct multiplayer still work. Keep protocol/socket requirements used by LAN/gameplay. |
| **Runtime: narrator/accessibility** — `horizon-narrator` unconditional in root, client-lib, render | Startup narration, accessibility/status controls in `game/narrator.rs`, `screen_narrator.rs`, `settings.rs`, `options_ui.rs`, widgets. Windows selects SAPI via `windows` crate. | Could be an explicit accessibility feature after a no-narrator implementation preserves screen navigation and reports unavailable/off. Regression: default SAPI path on Windows; no-feature game startup/options; do not classify the SAPI dependency as proven Win7-incompatible. |
| **Runtime: telemetry/benchmark** — `horizon-telemetry` unconditional with `default-features=false, features=["live"]` in root/client-lib/render; `horizon-bench` unconditional | `heartbeat.rs` uses live ping; crash/performance/repro/benchmark code uses local telemetry structures and consent; render telemetry persists reports. `horizon-telemetry` makes `ureq` optional, but `live` activates it despite client defaults being off. | Separate network upload/live ping from local crash/perf/report types. Regression: no unexpected network/consent changes; local report/crash tests; telemetry-enabled path. Don't drop `horizon-bench` before its dedicated-server and client benchmark/report consumers are separated. |
| **Runtime: dedicated/LAN server support** — `horizon-anticheat`, `horizon-chat-filter`, `horizon-server-files`, `mio`, `horizon-protocol` | Dedicated server code is in the same `horizon-client` package/binary (`entry.rs`, `game/dedicated.rs`); LAN hub/guest uses anticheat/filter; client/server profile/list/save code uses server-files. | Not a first renderer cut. A `--dedicated` run-time branch is not a Cargo feature. First split client/server target or define a real feature boundary, then test both dedicated and Open-to-LAN, player prediction/filtering, and save/list behavior. |
| **Already optional: audio/controllers** — root defaults `audio`,`controllers`; `audio` forwards to render/lib; `controllers` activates optional `gilrs`, `horizon-dualsense`, input support. Render `cpal` is optional; DualSense uses `hidapi` and `cpal` on non-iOS targets. | Code has feature-specific audio implementations (`client-render/src/audio.rs`) and controller code (`client/src/game/pad.rs`, hardware); `horizon-client-lib` also defines defaults, but root consumes it with `default-features=false` and explicitly forwards root features. | Keep these as the first no-default comparison delta. Regression: normal audio and physical/gamepad path; no-default silent audio/virtual pad; confirm Linux has no ALSA/libudev linkage where intended. On Windows x86 verify controller/audio target closure separately if enabled. |
| **Build-only (not player runtime)** — `horizon-mod-api-gen` build-dependency in root and client-render; root `winresource` is a Windows build-dependency | Codegen and icon resource generation run during build. They are not runtime renderer dependencies; the build script/host toolchain still matters for cross builds. | Keep them out of runtime cut claims. Test build-script execution for host→i686 Windows cross-build; `winresource` should not be removed as if it were a runtime DLL. |
| **Test/dev-only** — `horizon-client` and `horizon-client-lib` dev-dependencies include renderer `test-support`, `horizon-items/test-support`, and additional Naga output features; render has dev-dependencies including legacy render and Naga backends. | Cargo's ordinary `normal,build` tree excludes dev edges; `cargo test` includes them and can feature-unify additional test-only support. | Record `cargo tree -e all`/test target separately from the shipping binary graph. Run tests after feature cuts; do not infer PE imports from test-only crates. |

**Launcher is a separate artifact/workspace.** `launcher/Cargo.toml` declares GPUI (`gpui-fast`/`gpui-fast-platform`) and launcher install/update, accounts, packs, registry, telemetry, and native-mod flows. It is not a dependency edge from `horizon-client`; its wgpu/window stack cannot be counted as part of the game PE merely because it launches the game. If B3 acceptance requires the launcher itself to run on Win7 x86, that is a separate artifact/toolchain/API qualification. For a portable game artifact, test direct launch and launcher launch separately.

## 4. Windows x86 target/build closure and risks (not compatibility claims)

For `--target i686-pc-windows-msvc`, current manifests show the following target distinctions:

| Edge | Scope | Audit note |
|---|---|---|
| `wgpu` base features `std,parking_lot,vulkan,metal,wgsl,naga-ir` in root, client-lib and render; Windows adds `dx12`; non-macOS adds `gles` | Runtime/target-specific GPU closure | All are modern renderer features, not GL2 native-render requirements. `horizon-hdr` adds a separate `wgpu/vulkan` edge and `ash` on non-macOS. Cargo target filtering may omit platform implementation crates, but only actual Windows-target `cargo tree`/build/import inspection can say which objects reach the PE. Request a feature boundary, not version updates. |
| `winit 0.30.13`, `arboard 3.6.1`; Windows dependency closure of `wgpu`, `gilrs`, `mio`/socket crates | Runtime; some edges target-specific within upstream manifests | Modern window/input and GPU stacks. This audit has not verified Win7 imports or minimum OS of transitive Windows APIs. Replace/gate only after P01/P02 and test i686. |
| `open 5.4.5` under root/client-lib `cfg(windows)` | Runtime Windows-only; `game/packs.rs` calls `open::that_detached` | Pack-browser helper, potentially feature-gate with pack UI if that UI has an explicit unsupported path. Do not confuse with build-only `winresource`. Win7 API requirement unverified. |
| `winresource 0.1` under root `cfg(windows)` build-dependencies | Build-time only for Windows target | Needed for the exe icon resource. It runs as a build dependency; it is not an imported runtime DLL. Verify cross-build host support and do not include it in runtime feature totals. |
| `horizon-threads`: unconditional `mimalloc` + patched `libmimalloc-sys`; Windows `windows-sys 0.61` | Runtime allocator/thread/OS support; Windows target APIs | Used by Game and render worker pools/allocator/memory caps/boot paint. Root's `system-allocator` affects the global allocator selection but does not by itself remove the `horizon-threads` crate or its public uses. x86 address-space, CRT, ISA and imported OS API compatibility need O01/PE/runtime evidence. |
| `horizon-narrator`: Windows `windows 0.62.2` SAPI; optional controllers add `gilrs` and `horizon-dualsense` (`hidapi`, `cpal`) | Target-specific runtime; controllers are default-enabled only in normal root build | Candidate cuts are accessibility/peripherals features, not proven unsupported dependencies. Test default and no-default i686 graphs; inspect imports and run feature-enabled paths on supported Windows separately. |
| `horizon-hdr`: `ash` for non-macOS, Wayland client/protocol crates only for Linux | Runtime; Linux-specific Wayland part does not target Windows | HDR/Vulkan path is modern-only; ensure it disappears from the legacy Windows artifact only after its modern call sites are cfg-separated. Do not label Linux-only Wayland crates as Windows blockers. |
| `horizon-client` MSRV 1.90 / edition 2024; Windows x86 target/std availability | Toolchain/build prerequisite, not a Cargo dependency | The launch plan's O01 gate still applies: check pinned toolchain/std, build-std if needed, CRT/PE imports, dynamic DLL loading, ISA/CPUID and 32-bit address-space peaks. Windows-latest or ordinary i686 compilation is not Win7 qualification. |

No dependency has been labelled “Win7-incompatible” without target-specific build/import/runtime evidence. The actionable D01 issue is that modern wgpu/HDR dependencies are unconditional, and default controllers/audio add further candidate system/API imports; the actual minimum OS of the full transitive closure remains unknown here.

## 5. Minimal feature-cut sequence and requests to the integrator

The requests below are intentionally manifest-level plans, **not proposed Cargo edits**. The integration protocol assigns all such edits to the integrator.

| Order | Request / exact manifest owners | Gate and regression checks |
|---|---|---|
| **0. Establish evidence (now / before any cut)** | On the declared integration baseline, run the §1 graphs for ordinary, no-default, and no-default + `horizon-client-render/legacy-gl` on i686 Windows; also inspect Linux target graph for Wayland-only edges. Save outputs and resolved target/toolchain. | `cargo tree -e normal,build,features`; inverse `-i wgpu`, `-i winit`, `-i horizon-hdr`, plus test/dev graph. This environment couldn't run these. Reconcile `docs/server.md` claim with actual tree and binary. |
| **1. Make backend selection real after C01/R01; largest high-confidence cut** | Request coordinated feature/target ownership in `crates/horizon-client/Cargo.toml`, `crates/horizon-client-lib/Cargo.toml`, `crates/horizon-client-render/Cargo.toml`, and `crates/horizon-hdr/Cargo.toml`. The integration owner must cfg/split modern `gfx`/HDR consumers and route startup through the existing agreed facade before marking `wgpu`, `pollster`, `horizon-hdr`, and modern-only Naga support optional. `crates/horizon-client-input/Cargo.toml` remains winit-bound until P01/P02. | Build/check modern default unchanged; build no-default + legacy renderer; run existing GL2 frame/native tests and actual Game startup when G01 exists. Assert `cargo tree -i wgpu`/`-i horizon-hdr` is empty for the legacy artifact (including normal/build edges), and inspect linked PE/import table. Do not treat `legacy-gl` currently being enabled as proof. |
| **2. Separate modern window/input stack after platform contract** | Request target/platform selection across root/client-lib/render/input manifests: `crates/horizon-client/Cargo.toml`, `crates/horizon-client-lib/Cargo.toml`, `crates/horizon-client-render/Cargo.toml`, `crates/horizon-client-input/Cargo.toml`; split `arboard` independently if P02 supplies the agreed clipboard path. | Modern window regression; legacy WGL lifecycle, keyboard/text/IME, mouse capture, clipboard, resize/minimize/focus and close tests. Assert legacy `cargo tree -i winit` is empty only after all consumers (including public mod API) migrate; inspect i686 imports. |
| **3. Preserve real optional feature wins; do not redesign core gameplay** | Existing audio/controllers: `crates/horizon-client/Cargo.toml`, `crates/horizon-client-lib/Cargo.toml`, `crates/horizon-client-render/Cargo.toml`, `crates/horizon-client-input/Cargo.toml`, `crates/horizon-dualsense/Cargo.toml`. Further independent proposals: shader packs (`client`, `client-lib`, `client-render`, `horizon-shaders`, `horizon-shader-api`); narrator (`client`, `client-lib`, `client-render`, `horizon-narrator`); friends (`client`, `client-lib`, `horizon-friends`); live telemetry (`client`, `client-lib`, `client-render`, `horizon-telemetry`). | For each feature: normal modern remains default; no-default graph removes only its dependencies; disabled mode has an explicit UI/API outcome; enabled regression path passes. Check shader-pack/reload, screen-reader accessibility, friend invitation + normal multiplayer, local telemetry/consent, audio and physical/virtual pad. Keep each feature request separate from the backend cut. |
| **4. Defer deeper mod/server consolidation** | Any split of `horizon-mods/index`, `horizon-anticheat`, `horizon-chat-filter`, `horizon-server-files`, `mio` or server-only modules needs explicit client/dedicated target/feature design across `crates/horizon-client/Cargo.toml`, `crates/horizon-client-lib/Cargo.toml`, and each owning crate manifest. | Not a first legacy GL cut. Require dedicated-server, LAN-host/client, native/content-mod, save/server-list and standard multiplayer regressions. A no-default build alone does not prove these consumers are absent. |

### Exact acceptance checks for the next integrator pass

1. Record actual output/versions for the three commands in §1 at a named SHA and target; separate `normal`, `build`, `target`, `dev` and build-script edges.
2. After step 1, modern default client build/tests remain green and the legacy artifact has no `wgpu`/`horizon-hdr` graph edge, no wgpu device/surface startup, and no silent modern fallback.
3. After step 2, legacy artifact has no `winit` edge; demonstrate actual WGL platform/input behavior, not only an empty tree.
4. On every change, also test existing default-audio/controllers behavior and the no-default server build; preserve server/gameplay features unless an explicitly separate server cut is accepted.
5. For B3, use the pinned i686 Win7 build path, PE/import/CRT/ISA review, and a real Win7 SP1 + Radeon X1650 run. Hosted Windows compile or a green CI is not hardware acceptance.

## 6. Work performed, tests and unverified limits

- Read `AGENTS.md`, `docs/design/legacy-gl-integration.md`, and `docs/design/legacy-gl-launch-plan.md`; confirmed exclusive central manifest ownership and integration base.
- Baseline checked: `fa703a67f635999f1f09a6ca669ee48e7bc12c97` (`arena/a7e96628-horizon`, current integration draft head at audit start).
- Reviewed root/client/client-lib/client-render/client-input/HDR/legacy-render and selected optional subsystem manifests; traced Game/renderer/input call sites cited above.
- Manifest evidence assertions using Python `tomllib`: PASS; `git diff --check`: PASS; `python3 tools/check_public_tree.py`: PASS; `python3 -m unittest discover -s tests`: PASS (653 tests, 26 skipped).
- Attempted both target graph commands, but this environment returns `cargo: command not found`; `rustc` is also absent. Thus there is no exact Cargo tree, Rust compile, Rust unit/integration test, or PE evidence in this PR.
- **Not verified:** transitive resolver feature union, current i686 target artifacts, build-script behavior, Windows imports/CRT/ISA, Win7 SP1 execution, Radeon X1650, full no-default server release, or B1/B2 Game startup.
