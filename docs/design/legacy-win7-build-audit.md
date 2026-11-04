# O01a — Win7 x86 build and PE/import audit

2026-11-04. **Build investigation + static tooling, not Win7/X1650 qualification.**

- Task-ID: **O01a**; Variant: **A — independent Python-stdlib PE reader**.
- Integration draft: [#4](https://github.com/lukintrees/horizon/pull/4);
  task draft: [#7](https://github.com/lukintrees/horizon/pull/7).
- Integration base: `arena/a7e96628-horizon`;
  fetched baseline **`453966cb6253adf848deb58165eba354693e7acc`**.
- Depends-on: existing native probe; no new facade/contract dependency.
- Owner/reviewer: lukintrees's scoped Arena.ai agent / coordinating Arena.ai integrator.
- Scope: this document, `tools/legacy_win7_audit.py`, `tests/test_legacy_win7_audit.py`.
  **No** manifests/lockfiles, contracts, central Game/GL wiring, workflows or policy edits.

## 1. Decision: target support is not a downloadable std

The game/client/lib/render/input manifests require **Rust 1.90, edition 2024**;
workspace/native-probe MSRV 1.85 does not lower the game's requirement.

| Path | Finding | Decision |
|---|---|---|
| Stable **1.90.0**, `i686-win7-windows-msvc`, `rustup target add` | Built-in **tier 3** target; upstream explicitly ships **no precompiled artifacts** | Not a binary-std installation path. A suggested `rustup target add` in E0463 is not proof of availability |
| Stable/nightly, `i686-pc-windows-msvc` | Ordinary x86 ABI/control build, not the Win7 std configuration | Do not relabel it Win7, lower only PE version fields, or infer compatibility from Windows-latest |
| **nightly-2025-09-25**, matching `rust-src`, `-Z build-std` | rustc **1.92.0-nightly**, satisfies the stated game MSRV; recognized Win7 target | **Recommended candidate build path**, §3. Full std compilation/linking still unverified here |
| Self-built **Rust 1.90.0 + target std** | Upstream-supported alternative to build-std; source tag resolves to `1159e78c4747b02ef996e55082b704c09b970588` | Stable fallback, not executed here; bootstrap host + target libraries, install as a private pinned toolchain, archive provenance |

Authoritative pinned references: Rust 1.90
[target documentation](https://github.com/rust-lang/rust/blob/1159e78c4747b02ef996e55082b704c09b970588/src/doc/rustc/src/platform-support/win7-windows-msvc.md),
[i686 target definition](https://github.com/rust-lang/rust/blob/1159e78c4747b02ef996e55082b704c09b970588/compiler/rustc_target/src/spec/targets/i686_win7_windows_msvc.rs),
and [Win7-specific std branches](https://github.com/rust-lang/rust/blob/1159e78c4747b02ef996e55082b704c09b970588/library/std/src/sys/pal/windows/c.rs).
The target uses vendor `win7`, x86/SSE2 ABI, CPU `pentium4`, `/SAFESEH`,
`/LARGEADDRESSAWARE`, and disables native TLS because of Win7-x86 TLS alignment behavior.
Its LLVM target spelling being `i686-pc-windows-msvc` does **not** make the Rust targets equivalent.
The [same no-artifact statement at the tested nightly commit](https://github.com/rust-lang/rust/blob/caccb4d0368bd918ef6668af8e13834d07040417/src/doc/rustc/src/platform-support/win7-windows-msvc.md)
was also checked. Do not use floating `nightly`, Rust 1.88 for the game, or
`RUSTC_BOOTSTRAP=1` as a purported supported stable build-std recipe.

Stable-source fallback: use that exact Rust commit, initialize its pinned submodules,
configure bootstrap host `x86_64-pc-windows-msvc` and targets including
`i686-win7-windows-msvc`, then build stage-1 compiler and target std (upstream bootstrap
procedure above). It needs a modern build host, bootstrap downloads and the same SDK/CRT
qualification; it is not a shortcut to building the game with an older compiler.

## 2. Evidence actually obtained in this sandbox

Debian 12 x86_64; Python **3.11.2**. Initially no rustup/Rust/MSVC/Windows SDK.
Allowed download hosts exclude the official Rust distribution and crates.io hosts.
No official `rustup target add` attempt, Windows link or Win7 runtime success is claimed.

Development-only bootstrap used these npm mirror components, all exactly **0.92.20250925**:
`@rusttonpm/rustc-linux-x64`, `@rusttonpm/cargo-linux-x64`, `@rusttonpm/rust-std-linux-x64`.
Each downloaded archive's SHA-512 was compared with its versioned registry `dist.integrity`
**before** extraction. Registry integrity is not independent Rust release authenticity;
these mirrors are **not** the recommended release supply chain. Everything stays in ignored
`.local/o01a/`; builds/reports in `target/o01a-*`. Reproduction on an unrestricted builder:
install official `nightly-2025-09-25` with rust-src (§3), verify these identities, and use
the same cargo commands below (with `+nightly-2025-09-25` if managed by rustup).

```text
rustc 1.92.0-nightly (caccb4d03 2025-09-24)
commit-hash: caccb4d0368bd918ef6668af8e13834d07040417
host: x86_64-unknown-linux-gnu; LLVM: 21.1.2
cargo 1.92.0-nightly (966f94733 2025-09-16)
```

Commands actually run from the repo root, with that compiler on PATH:

```sh
export PATH="$PWD/.local/o01a/toolchain/bin:$PATH"
export CARGO_HOME="$PWD/.local/o01a/cargo-home"
rustc -Vv; cargo -Vv
rustc --print target-list | grep -E 'i686-(pc|win7)-windows-msvc'
rustc --print cfg --target i686-win7-windows-msvc
rustc -Z unstable-options --print target-spec-json --target i686-win7-windows-msvc
cargo check --offline --locked --manifest-path tools/legacy-gl-probe/Cargo.toml \
  --target i686-win7-windows-msvc --target-dir target/o01a-probe
cargo test --offline --locked --manifest-path tools/legacy-gl-probe/Cargo.toml \
  --target-dir target/o01a-probe-host
cargo test --offline --locked --manifest-path crates/horizon-legacy-render/Cargo.toml \
  --workspace --target-dir target/o01a-native-host
cargo metadata --no-deps --offline --locked --format-version=1
```

| Observation | Result / boundary |
|---|---|
| target list / cfg / spec | Both triples listed; Win7 cfg: `target_vendor="win7"`, width 32, `sse`/`sse2`/`fxsr`/`x87`; tier 3, CPU pentium4 |
| Win7 probe check, no target std | **Exit 101, E0463: can't find crate for `std`**. No PE emitted |
| Host probe unit tests | **2 passed**, no WGL/GL runtime |
| Host native/API workspace tests | **38 passed** (11 + 27), no Windows runtime |
| Game manifest inventory | MSRV 1.90 confirmed; mandatory wgpu dependencies still include Windows DX12 and non-macOS GLES |

For an additional build-std attempt, the matching Rust **library** tree was extracted from
`https://codeload.github.com/rust-lang/rust/tar.gz/caccb4d0368bd918ef6668af8e13834d07040417`
into the toolchain's `lib/rustlib/src/rust/library` location (no repository manifests edited).
Only this library tree, not a complete rust-src installation with initialized submodules,
was assembled here. Running `cargo check --offline --locked -Z build-std=std,panic_abort`
with the same probe/target initially stopped at missing registry package **hashbrown**.
The existing helper was then invoked, unchanged:

```sh
python3 tools/cargo_github_vendor.py \
  --lock .local/o01a/toolchain/lib/rustlib/src/rust/library/Cargo.lock \
  --cache .local/o01a/std-vendor
cargo --config .local/o01a/std-vendor/offline.toml check --offline --locked \
  -Z build-std=std,panic_abort --manifest-path tools/legacy-gl-probe/Cargo.toml \
  --target i686-win7-windows-msvc --target-dir target/o01a-probe
cargo --config .local/o01a/std-vendor/offline.toml check --offline --locked \
  -p horizon-client --target i686-win7-windows-msvc
```

Vendor helper: **exit 2**, 30 verified / 6 missing (addr2line 0.25.0, cfg-if 1.0.1,
dlmalloc 0.2.10, gimli 0.32.0, rustc-literal-escaper 0.0.5, wit-bindgen 0.45.0).
Probe build-std: **exit 101, missing cfg-if**. Game check: **exit 101, missing libm**
(the std-only vendor inventory is not the game dependency closure).
These are **environment/source-availability blockers**, not observed Rust source/linker errors.
No successful std rebuild, linked native probe or game PE follows from them. Historical
Firefox/Chromium vendor metadata snapshots also failed to fill those six packages; no endless
retry, fabricated checksum, unpinned dependency upgrade or lockfile workaround was used.

## 3. Recommended candidate path on a provisioned builder — NOT executed here

Use a **modern Windows build host**, not Win7 as the compiler host. Select MSVC **v142 / 14.29**
x86 tools and Windows SDK **10.0.19041.0** in a developer shell, then start PowerShell inheriting
that environment. This is a proposed conservative CRT/toolset profile, **not a tested lock**:
record full `cl /Bv`, linker/resource-compiler versions, installed toolset patch, SDK and CRT
object hashes before declaring a reproducible artifact. Do not silently substitute latest VC
redistributables. Linux clang-cl/lld-link + a legally obtained SDK is another upstream option,
but this sandbox has neither that SDK nor proof of the resulting CRT closure.

```powershell
# Provisioned x86 developer environment; all paths below are repo-relative.
$N = "nightly-2025-09-25"
$T = "i686-win7-windows-msvc"
rustup toolchain install $N --profile minimal --component rust-src
if ($LASTEXITCODE) { throw "toolchain provisioning failed" }
rustc +$N -Vv
cargo +$N -Vv
where.exe cl
where.exe link
cl /Bv
# rustup target add is NOT needed: build-std supplies the missing target library.
$env:RUSTFLAGS = "-C target-cpu=pentium4 -C target-feature=+crt-static"
cargo +$N build --locked -Z build-std=std,panic_abort --release `
  --manifest-path tools/legacy-gl-probe/Cargo.toml --target $T --target-dir target/o01a-probe
if ($LASTEXITCODE) { throw "native probe build failed" }
# Rebuild in a fresh output directory, now offline, including the std rebuild.
cargo +$N build --offline --locked -Z build-std=std,panic_abort --release `
  --manifest-path tools/legacy-gl-probe/Cargo.toml --target $T --target-dir target/o01a-probe-offline
if ($LASTEXITCODE) { throw "offline reproduction failed" }
$PE = "target/o01a-probe-offline/$T/release/horizon-legacy-gl-probe.exe"
python tools/legacy_win7_audit.py $PE `
  --deny-dll bcryptprimitives.dll --deny-dll api-ms-win-core-synch-l1-2-0.dll `
  --deny-import 'kernel32.dll!WaitOnAddress' --deny-import 'kernel32.dll!WakeByAddressSingle' `
  --deny-import 'kernel32.dll!WakeByAddressAll' `
  --deny-import 'kernel32.dll!GetSystemTimePreciseAsFileTime' `
  > target/o01a-probe-offline/pe-audit.json
if ($LASTEXITCODE) { throw "static audit failed or input unsupported" }
dumpbin /headers /imports $PE
```

The deny examples are **not an exhaustive Win7 policy**. Archive the exact commit/clean status,
commands/environment, compiler/source/SDK/CRT provenance, dependency lock inventories, build logs,
EXE + every shipped DLL hash, audit JSON and independent dumpbin output. Check static CRT linker
choices for **all C/C++ dependencies**, not just Rust's `crt-static` flag. If dynamic CRT is
selected instead, freeze and test the x86 runtime/UCRT deployment and Win7 servicing prerequisites
(e.g. the UCRT update), not just the presence of a file named `vcruntime140.dll`.

**Probe and game are different gates.** The probe is its existing independent workspace,
needs no game resources and does not exercise the game's mimalloc/TLS/audio/network closure.
`--no-default-features` on the current game drops audio/controllers, **not mandatory wgpu/DX12**.
There is no approved legacy-only Game feature/build recipe at this baseline. Do not invent one
in this task. After integrator-owned D01/G01, apply the same target/std/CRT profile to the actual
Game artifact, record the resolved `cargo tree --locked --target ... -e features` graph and audit
**all** shipped PEs again. Native-probe success cannot stand in for that work.

## 4. Isolated static helper and reproducible negative fixtures

Runs with Python 3.10+ on the **build/audit host**; Python need not run on Win7.
No third-party parser, code execution, DLL loading, SDK or network is needed.

```sh
python3 -m unittest discover -s tests -p test_legacy_win7_audit.py -v
python3 tools/legacy_win7_audit.py path/to/probe.exe path/to/bundled.dll \
  --deny-import 'kernel32.dll!WaitOnAddress' --deny-dll ucrtbase.dll > target/pe-audit.json
```

- Reports SHA-256/size, COFF machine + PE32/PE32+, OS/subsystem versions,
  large-address-aware flag, **normal and delay** DLLs, names/`#ordinals`, CRT families.
- Parses RVA-based and old VA-based delay descriptors, PE32+ 64-bit thunks, and unbound
  IAT fallback when original names are absent. Bound IAT without original names is rejected.
- DLL deny matching is case-insensitive; symbol matching is case-sensitive; rules are **exact**,
  no wildcard or implicit API availability database. DLL names are reported as stored, not
  resolved through Windows search paths, API-set maps or forwarded exports.
- Exit **0**: requested static checks passed; **1**: wrong architecture (requires I386 **and**
  PE32), OS/subsystem version above 6.1 or explicit deny match; **2**: unreadable, malformed,
  unsupported input or invalid CLI arguments. Multi-file audit continues; exit 2 takes priority.
  Every file explicitly has `win7_qualified: false`, including status `static_checks_passed`.
- Fails closed on unmapped/zero-fill/cross-mapping ranges, missing terminators, ambiguous
  overlapping sections, unknown delay flags. Limits: 512 MiB/file, 96 sections,
  4096 descriptors/directory including terminator, 65536 thunk slots total including terminators,
  4096 bytes/string including NUL. These are conservative audit limits, **not** a full PE loader
  validator or a promise to accept packed/unusual but loadable images.

**16 tests passed**, including original positive and negative binary layouts generated in memory:
x64/ARM/machine-magic mismatch, newer header versions, prohibited normal **and delay** imports,
ordinals, CRT categories, legacy VA delay imports, bound IAT, truncated/overlapping/out-of-range
headers and tables, absent terminators, 187 truncations and 150 seeded structural mutations,
CLI JSON and all three exit codes. GNU objdump independently identified the PE32 fixture's
machine/version/normal imports and both data-directory entries. Fixtures are format tests,
**not executable Windows programs** or Win7 evidence; no OS/game binaries are committed.

## 5. What static audit does not prove / integrator follow-up

| Risk | Required evidence beyond this helper |
|---|---|
| Dynamic API loading | Inspect `LoadLibrary*`/`GetProcAddress`, Rust `raw-dylib`, plugin/driver DLLs, API-set/forwarder resolution and mandatory vs optional paths. Win7 std dynamically probes newer synchronization APIs; absence from imports does not prove fallback correctness. WGL sentinel/fallback unit tests are not an ATI ICD run |
| CRT and dependent DLLs | Audit every actual EXE/DLL and recursively resolve exports on the clean target OS; inspect debug runtime dependencies and embedded manifests. Missing imports do not prove static CRT, patch level, valid activation context, safe DLL search paths or successful startup |
| Allocator / pre-main initialization | Probe uses the system allocator; Game defaults to capped mimalloc, with native C/C++ and initialization risk. Existing `system-allocator` selects the global heap but does **not** remove mimalloc dependencies/calls from horizon-threads. Its POPCNT patch is not Win7-x86 qualification; no Windows Segment Heap experiment for this profile |
| CPU ISA | Record **actual CPU/CPUID**, i686/SSE2/FXSR support and any OS state required by runtime dispatch. X1650 says nothing about CPU. Do not use `target-cpu=native`, `/arch:AVX*` or unguarded POPCNT/LZCNT/SSE3+/AVX. Audit native objects, allocator, compiler builtins, dispatch guards and final code; current x64-oriented CPU policy is not a complete x86 ISA proof |
| 32-bit address space | `/LARGEADDRESSAWARE` does not give ordinary Win7 x86 4 GiB of user VA. Measure peak commit, mappings, stacks, fragmentation and largest allocation separately from RSS/RAM/VRAM. World streaming/reload/integrated server need real soak tests |

**Requests for the integrator, not central patches in O01a:** D01 must publish a real legacy-only
feature/dependency graph and audit native build flags/SDK requirements (including cc/tooling handling
of vendor `win7`). If system-allocator isolation is chosen, the integrator must make the native
allocator closure genuinely optional, not merely turn off the global-allocator attribute.
O03 must qualify files/threads/TLS/network and then audio/controllers/narrator individually.
No fallback DLL shim, duplicated renderer API or workflow/policy relaxation is introduced here.

## 6. Open acceptance gates

- [ ] Trusted official pinned toolchain + complete matching rust-src/dependency cache;
  successful offline Win7 std rebuild, **linked PE32 native probe**, independent import check.
- [ ] Freeze exact SDK/toolset/CRT revisions and deployment; test clean target without build tools.
- [ ] D01/G01 full-game legacy-only build, allocator/native ISA audit and dependency closure.
- [ ] Real **Win7 SP1 x86 + Radeon X1650 vendor ICD** probe: WGL context/readback/present,
  actual driver/CPU/CRT/limits recorded, no software/GDI substitute.
- [ ] Same target hardware executes real Game B2 input/world/save/reopen/reload/soak scenario.
  Use fresh scratch homes and no-audio labels per the launch plan; preserve only synthetic test saves.
- [ ] Explicit integrator review/selection and acceptance. CI or static exit 0 does not close these gates.

Without that stand, **hardware gate remains open**. O01a can be reviewed as an isolated audit/tooling
candidate; it does not assert B1/B2/B3, a Win7-compatible game artifact, or task acceptance.
