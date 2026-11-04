//! The OpenGL 2.x terrain path for GPUs without OpenGL 3.3 (the Radeon X1650 under Windows 7; see
//! docs/plans/opengl21-radeon-x1650.md, milestone M2). `atlas` repacks the block sprites into 2D pages,
//! `batch` turns the mesher's solid quads into float vertices grouped by page, and `draw` draws them.

pub mod atlas;
#[rustfmt::skip]
pub mod batch;
#[rustfmt::skip]
pub mod draw;
