//! Horizon's OpenGL 2.1 renderer: the legacy path.
//!
//! # Why this crate exists
//!
//! Horizon's renderer needs Vulkan, DirectX 12 or OpenGL 3.3. wgpu's OpenGL backend refuses a
//! desktop context below 3.3 (`third_party/wgpu-hal/src/gles/adapter.rs`), and the GPUs this
//! crate is for stop at 2.1: an ATI Radeon X1650 (RV535, 2006) reports `2.1.8545`, GLSL 1.20,
//! 0 vertex texture units, no `ARB_texture_array`, no `ARB_uniform_buffer_object`, no compute,
//! no `EXT_color_buffer_float`. No setting of the existing renderer makes that work, so this
//! is a second renderer, not a configuration.
//!
//! The plan, the target's measured limits and the phase gates are in
//! `docs/gl21-x1650-plan.ru.md`; the development environment (two machines, mirrors, what
//! can and cannot be checked without the target) is in `docs/gl21-dev-env.md`.
//!
//! # How the work is split
//!
//! | Module | What it decides or does | Needs a GPU |
//! | --- | --- | --- |
//! | [`caps`] | what the device may do; strict mode substitutes the target's table | to detect |
//! | [`budget`] | measures GLSL against the target's limits before a driver sees it | **no** |
//! | [`draw`] | one frame described once: passes, draws, state, requirements | **no** |
//! | [`exec`] | executes a recorded frame and counts what it cost | **yes** |
//! | [`loader`] | finds libGL/opengl32 and its entry points, with a sentence on failure | to open |
//! | [`gl`] | programs, buffers, textures, framebuffers, the state cache, the context | **yes** |
//!
//! The GPU-free half is the half that carries the risk of the port (a shader that works on
//! llvmpipe and not on the target), so it is the half written and tested first.
//!
//! # Testing without the target
//!
//! llvmpipe reports OpenGL 4.5 and accepts everything this path must not use, so a picture
//! drawn on it proves nothing about the X1650. `HORIZON_GL21_STRICT=1` makes the runtime
//! behave as the target's driver would, and `tools/gl21_budget.py` checks the same budgets
//! statically in CI. What neither can check — the driver's shader compiler, frame times, the
//! 256 MiB of video memory — is listed in `docs/gl21-x1650-plan.ru.md` §15.

pub mod budget;
pub mod caps;
pub mod draw;
pub mod error;
pub mod exec;
pub mod gl;
pub mod loader;
pub mod mesh;
pub mod multi_draw;

pub use error::{Error, Result};

/// The crate's version, in the log line a bug report should carry.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
