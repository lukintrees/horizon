//! Building blocks for an experimental OpenGL 2.1 renderer.
//!
//! This module is not connected to the game loop yet. The current wgpu renderer
//! remains the only selectable renderer. Keep these CPU structures independent
//! of either graphics API so they can also be tested without a GPU.
pub mod atlas;
pub mod terrain;

/// The prototype terrain shaders. They require a 2D atlas page with one-pixel
/// gutters and non-mipmapped filtering; this is not the complete world pipeline.
pub use horizon_legacy_gl::{TERRAIN_FRAGMENT_SHADER, TERRAIN_VERTEX_SHADER};

/// Adapt the existing, finished sprite array to the experimental atlas format.
/// A `finish_without_pixels` server atlas is rejected by the pixel count check.
pub fn atlas_from_sprites(
    sprites: &crate::sprites::Sprites,
    max_texture_size: u32,
) -> Result<atlas::Atlas, atlas::AtlasError> {
    let layers = u32::try_from(sprites.count()).map_err(|_| atlas::AtlasError::SizeOverflow)?;
    atlas::Atlas::new(sprites.size, layers, max_texture_size, &sprites.pixels)
}
