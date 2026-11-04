//! The OpenGL 2.x path for GPUs without OpenGL 3.3, such as the Radeon X1650 under Windows 7 (see
//! docs/plans/opengl21-radeon-x1650.md). `atlas` repacks the block sprites into 2D pages, `batch`
//! turns the mesher's solid quads into float vertices grouped by page, `draw` draws the terrain,
//! `hud` draws the GUI, `sky` the stars, the sun, the moon, the clouds, the lightning, the rain and
//! the snow, and `renderer` is the frame loop the game's `renderer::Renderer` forwards to.

/// What a screenshot request answers on this path. The game shows it to the player.
pub const SCREENSHOT_UNSUPPORTED: &str = "скриншоты в OpenGL 2 не поддерживаются";

pub mod atlas;
#[rustfmt::skip]
pub mod batch;
#[rustfmt::skip]
pub mod draw;
pub mod hud;
pub mod renderer;
mod sky;
mod shader;
mod stubs;
#[cfg(windows)]
mod wgl;

pub use renderer::Renderer;
