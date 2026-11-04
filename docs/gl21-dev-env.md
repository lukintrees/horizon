# Development environment for the OpenGL 2.1 port

Two different machines, two different problem sets. Read the section for the machine you
are on; `tools/gl21_preflight.sh` tells you which one it is.

```sh
tools/gl21_preflight.sh              # what is here, what is missing
tools/gl21_preflight.sh --install    # Python-side gates into .local/gl21-env (never system-wide)
tools/gl21_preflight.sh --json .local/gl21-preflight.json
```

Everything the script installs lands in `.local/` (gitignored). It never calls `apt` and
never writes outside the repository: on a container or a shared box that is the difference
between a script that helps and one that half-installs a toolchain.

## 1. Build and test machine (Linux, the one used here)

Needs: rustc + cargo, Mesa (llvmpipe is enough), and for windowed runs a virtual display.

```sh
# Debian/Ubuntu, with root:
sudo apt install build-essential pkg-config libgl1-mesa-dri libegl-mesa0 mesa-utils \
                 xvfb libxkbcommon-dev libasound2-dev libudev-dev
# and Rust, with a mirror (see §3):
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
```

Drawing without a GPU (this is what "test on llvmpipe" means in practice):

```sh
export LIBGL_ALWAYS_SOFTWARE=1          # force llvmpipe
export MESA_GL_VERSION_OVERRIDE=3.3     # lie about the version only when a wgpu path needs it
glxinfo -B | head -5                    # OpenGL renderer string: llvmpipe (LLVM ...)
xvfb-run -s "-screen 0 1280x720x24" cargo run -p horizon-client-gl --example first_light
```

For a windowless render use EGL surfaceless (`EGL_PLATFORM=surfaceless`) or the
`--offscreen` path, which needs no display at all.

**llvmpipe cannot check the target.** It reports OpenGL 4.5: it will happily accept array
textures, uniform blocks, compute shaders and `RGBA16F` targets, none of which exist on the
X1650. That is what `tools/gl21_budget.py` (static) and `HORIZON_GL21_STRICT=1` (runtime
allow-list) are for; see `gl21-x1650-plan.ru.md` §4.

## 2. Target machine (Windows 7 SP1, 32-bit, Radeon X1650)

| Piece | What it must be |
| --- | --- |
| OS | Windows 7 SP1 **x86**, all updates; the GPU driver is Catalyst 10.2 (last for X1000), 9.3+ works |
| GPU driver | Catalyst 8.591+ — reports `2.1.8545 Release`, GLSL `1.20`, 112 extensions |
| Runtime | Visual C++ 2015-2022 redistributable, **unless** the build is static (`+crt-static`) |
| WDDM/Aero | Aero off is faster on this chip; the renderer must not depend on it |
| Screen | 1280×1024 typical for these machines; plan on 800×600…1280×720 in game |

Toolchain for a binary that *starts* on Windows 7:

* Rust ≥ 1.78 dropped Windows 7: `*-pc-windows-*` targets now require Windows 10. Two ways:
  1. `i686-win7-windows-msvc` — tier 3, needs a **pinned nightly** and `-Z build-std=std,panic_abort`:

     ```sh
     rustup toolchain install nightly-2026-10-14 --profile minimal
     rustup +nightly-2026-10-14 component add rust-src
     cargo +nightly-2026-10-14 build -p horizon-client-gl --target i686-win7-windows-msvc \
       -Z build-std=std,panic_abort --release
     ```
  2. the ordinary `i686-pc-windows-msvc` plus import shims for the five or eight Windows
     8+ functions std pulls in (`WaitOnAddress`, `WakeByAddressSingle/All`, `ProcessPrng`,
     `GetSystemTimePreciseAsFileTime`, `combase!CoTaskMem*`). Cheaper to keep working, more
     code to maintain.
* Either way, `RUSTFLAGS="-C target-feature=+crt-static"` and a manifest with
  `MinVersion 6.1sp1`, and the import gate (§4) in CI.
* The launcher (`launcher/`, GPUI) does not build for 32-bit Windows 7; installing a
  legacy build is by hand (`docs/gl21-x1650-plan.ru.md` §9, D3).

## 3. Mirrors

The plan's development happens where crates.io and static.rust-lang.org may be slow or
blocked, so both are configurable per-user or per-invocation.

**Rust toolchain (rustup):**

```sh
export RUSTUP_DIST_SERVER=https://rsproxy.cn
export RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup
# alternatives: https://mirrors.ustc.edu.cn/rust-static, https://mirrors.tuna.tsinghua.edu.cn/rustup
```

**Crates (cargo):** write `.local/cargo-config.toml` and point `CARGO_HOME` at it, or add to
`~/.cargo/config.toml`:

```toml
[source.crates-io]
replace-with = "mirror"

[source.mirror]
registry = "sparse+https://rsproxy.cn/index/"
# alternatives: sparse+https://mirrors.ustc.edu.cn/crates.io-index/,
#               sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/
```

```sh
export CARGO_HOME=$PWD/.local/cargo   # keeps the mirror config and the cache out of ~
cargo build -p horizon-client-gl
```

**Python (the gates):**

```sh
tools/gl21_preflight.sh --install --mirror https://mirrors.aliyun.com/pypi/simple/
# or: PIP_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple tools/gl21_preflight.sh --install
```

**System packages (Mesa, Xvfb):** whatever the distribution's own mirror is; the script does
not touch them, on purpose.

## 4. What runs where

| Gate | Needs | Runs on |
| --- | --- | --- |
| `tools/gl21_budget.py` | Python 3.9+ | anywhere, including a sandbox with no compiler |
| `tools/gl21_glsl.py` | Python 3.9+ | anywhere (GLSL 1.20 gate: budgets, grammar, stage pairs) |
| `tools/gl21_preflight.sh` | bash | anywhere (says what this machine has and lacks; `--setup` provisions what it can) |
| `tools/gl21_typecheck.py` | Python 3.9+ + tree-sitter | anywhere **with glow's sources** (`--glow-cargo` from a cargo cache, or `--glow <checkout>`) |
| `tools/gl21_rustsyntax.py` | `tree-sitter`, `tree-sitter-rust` (`.local/gl21-env`) | anywhere |
| `cargo test -p horizon-client-gl` | rustc | build machine |
| `cargo test -p horizon-client-gl -- --ignored` (context tests) | rustc + a GL 2.1+ driver | build machine with Mesa |
| `cargo run -p horizon-client-gl --bin horizon-gl21-info -- --probe` | rustc (+ a driver for the second half) | build machine |
| `cargo test -p horizon-client-gl` | rustc only: caps, budgets, the draw list, the executor, state, targets, the vertex format and the sprite maths | **any machine with rustc** |
| `python3 tools/gl21_typecheck.py --self-test` | tree-sitter only: proves the checker itself still flags what it must | anywhere |
| the `gl21` job in `.github/workflows/ci.yml` | nothing local: it is the compiler, the linter, the tests and the API check | **GitHub** |
| `tools/gl21_compare.py` (planned, M2) | rustc + Mesa + a captured scene | build machine |
| playability (frame times, driver quirks) | the real X1650 | target machine only |

**This sandbox, measured** (2026-11-04): PyPI, npm and GitHub are reachable; `apt-get`,
`static.rust-lang.org`, `crates.io`, Debian mirrors and every Rust mirror tried (rsproxy,
USTC, TUNA, SJTU, aliyun, tencent, bfsu, hust, nju, zju) are not; there is no `rustc`,
no `libGL`, no Mesa, no `/dev/dri`, no X server. So here the gates that run are the static
ones — `gl21_budget.py`, `gl21_glsl.py`, `gl21_rustsyntax.py` and the pure unit tests of
`horizon-client-gl` — and everything that needs `rustc`, a GL context or a window is written
and reviewed but **not run**. `gl21_preflight.sh` says exactly that, and says it again when
those pieces arrive.

## 5. Reading a failed start on the target

Crash-in-driver has no message. The renderer already logs each startup step
(`gfx.rs::startup_step`, flushed unbuffered): on Windows 7 the log's last line names the step
where the driver died. For the legacy path the same rule applies from the first commit: log
before `wglCreateContext`, before the first `glGetIntegerv`, before the first program link,
and flush.

## Where the compiler actually runs

The machine this crate was written on has no rustc and no libGL: every Rust mirror and every
package mirror is blocked there (`docs/gl21-dev-env.md`, the network matrix above). That is a
development-environment fact, not a reason to stop, and the arrangement that replaces a local
compiler has three parts:

1. **`.github/workflows/ci.yml`, job `gl21`** — the real build: `cargo +stable clippy
   -p horizon-client-gl --all-targets -- -D warnings`, `fmt --check`, `test`, then
   `tools/gl21_typecheck.py --glow-cargo` against the glow sources `Cargo.lock` pins, then
   the binary's own report. Every push to this branch runs it; a pull request runs it too.
   Push and read the log — that is the `cargo check` the sandbox cannot give.
2. **`tools/gl21_rustsyntax.py`** — parses every file with tree-sitter: catches a missing
   brace, an unbalanced `impl`, a stray token, in under a second and with no toolchain.
3. **`tools/gl21_typecheck.py`** — reads glow's `HasContext` trait and the crate's own
   functions, and reports a call with the wrong arity, a method glow does not have, a
   `glow::CONSTANT` glow does not define, or a literal of the wrong type. It is the cheap
   approximation of the compiler's most common complaint, and it is deliberately silent when
   it cannot name a receiver's type rather than guessing.

None of the three is a substitute for the fourth thing, which is a person running the game on
the X1650. `HORIZON_GL21_STRICT=1` plus the `gl21` job gets the code to the point where that
run is worth doing; it cannot make llvmpipe's GL 4.5 behave like Catalyst 9.3.

`tools/gl21_preflight.sh --setup` writes the environment those three expect: it fetches the
Rust toolchain from `GL21_RUSTUP_DIST_SERVER` (default `https://rsproxy.cn`), the Python gates
from `--mirror`, glow's source for the type check, and a `CARGO_HOME` registry replacement at
`GL21_CARGO_REGISTRY`. On this sandbox every one of those downloads is refused; the script says
so in one line each and points at the job above.

## The budget gate has a floor, and it is a file

`tools/gl21_budget.py --check` measures the *wgpu* shaders against the X1650's limits, so on a
tree that still carries the modern renderer it is red by construction: 80 findings, every one of
them a thing the GL path is being built to replace (`texture_2d_array`, SSBOs, compute loops).
A gate that is red on a clean checkout is a gate people learn to ignore, so CI runs it against
`tools/gl21-budget-baseline.json` — the 80 known rows — and fails only on a **new** finding:

```sh
python3 tools/gl21_budget.py --check --baseline tools/gl21-budget-baseline.json
# against tools/gl21-budget-baseline.json: 0 fixed, 0 new
```

Two rules keep that honest:

* **A ported shader lowers the file.** When a WGSL file's remaining work is done, delete its rows
  from the baseline in the same commit. The gate prints `N fixed` for rows that no longer match,
  so a stale baseline is visible rather than silent.
* **The GLSL path has no floor.** `tools/gl21_glsl.py --check` runs on `src/shaders/terrain/`
  with no baseline: those files are written for the target, and a finding there is a bug.
