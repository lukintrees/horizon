//! Headless OpenGL 2.x check for the Radeon X1650 path (docs/plans/opengl21-radeon-x1650.md).
//!
//! `gl2-probe` creates a desktop OpenGL context through EGL on the surfaceless platform,
//! reports what the driver offers and renders a GLSL 1.10 triangle into a non-power-of-two
//! framebuffer. CI runs it on Mesa llvmpipe, with the version capped to 2.1 or 2.0 through
//! `MESA_GL_VERSION_OVERRIDE` and `MESA_GLSL_VERSION_OVERRIDE`.

pub mod egl;
pub mod probe;
pub mod report;
#[cfg(windows)]
pub mod wgl;
