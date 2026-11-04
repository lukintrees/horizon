//! The game's OpenGL 2.1 window (docs/plans/opengl21-radeon-x1650.md, phase 1), and the terrain drawn in it (phase 2).
//!
//! `frame` draws the gate-G1 frame into a window of any size. `terrain` draws the GL2 terrain: section
//! meshes from `world`, camera maths from `math`. `window` (Windows only) owns the winit window and
//! attaches a WGL context to it. `version` and `report` are shared by the run and its tests.

pub mod frame;
pub mod math;
pub mod report;
pub mod terrain;
pub mod version;
pub mod world;
#[cfg(windows)]
pub mod window;
