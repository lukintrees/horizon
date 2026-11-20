//! CPU menu request: the one frame shape the modern and the legacy backend
//! submit through the same contract (`renderer_facade`). A static menu cannot
//! carry or drop a world geometry stream, so a menu is its own request, not a
//! stripped world frame.
//!
//! The world frame itself lives in `gfx::Frame`, which both backends already
//! read: this module must not name a `wgpu` type, and `menu` builds none.
pub mod menu;
pub use menu::{MenuBackground, MenuFrame};

