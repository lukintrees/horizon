//! The game's OpenGL 2.1 window (docs/plans/opengl21-radeon-x1650.md, phase 1).
//!
//! `frame` draws the gate-G1 frame into a window of any size. `window` (Windows only) owns the winit
//! window and attaches a WGL context to it. `version` and `report` are shared by the run and its tests.

pub mod frame;
pub mod report;
pub mod version;
#[cfg(windows)]
pub mod window;
