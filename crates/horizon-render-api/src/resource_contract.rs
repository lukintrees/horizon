//! Borrowed CPU resources. Mip preparation/decoding stays at the producer;
//! uploading/retiring GPU resources stays at the adapter. No duplicate atlas.
use crate::{Image, render_contract::ResourceGeneration};

pub struct TextureLayers<'a> {
    pub size: u32,
    pub layers: u32,
    /// Layer-major RGBA8, same stable layer IDs as the mesher.
    pub rgba: &'a [u8],
}

pub struct Resources<'a> {
    pub generation: ResourceGeneration,
    pub terrain: TextureLayers<'a>,
    pub hud: Image<'a>,
    pub item_glint: Image<'a>,
    pub armor_glint: Image<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceError {
    InvalidDimensions,
    InvalidPixels,
    Budget,
}

fn byte_len(width: u32, height: u32, layers: u32) -> Result<usize, ResourceError> {
    if width == 0 || height == 0 || layers == 0 {
        return Err(ResourceError::InvalidDimensions);
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(layers as usize))
        .and_then(|n| n.checked_mul(4))
        .ok_or(ResourceError::Budget)
}
impl Resources<'_> {
    /// Validate before allocation. `max_bytes` bounds the total borrowed base
    /// pixels, not peak staging/VRAM usage; A01 must budget those separately.
    pub fn validate(&self, max_bytes: usize) -> Result<usize, ResourceError> {
        let mut total = byte_len(self.terrain.size, self.terrain.size, self.terrain.layers)?;
        if self.terrain.rgba.len() != total {
            return Err(ResourceError::InvalidPixels);
        }
        for image in [&self.hud, &self.item_glint, &self.armor_glint] {
            let bytes = byte_len(image.width, image.height, 1)?;
            if image.rgba.len() != bytes {
                return Err(ResourceError::InvalidPixels);
            }
            total = total.checked_add(bytes).ok_or(ResourceError::Budget)?;
        }
        if total > max_bytes {
            return Err(ResourceError::Budget);
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_exact_lengths_and_aggregate_budget_without_allocating() {
        let image = || Image {
            width: 1,
            height: 1,
            rgba: &[255; 4],
        };
        let mut bundle = Resources {
            generation: ResourceGeneration(1),
            terrain: TextureLayers {
                size: 1,
                layers: 2,
                rgba: &[0; 8],
            },
            hud: image(),
            item_glint: image(),
            armor_glint: image(),
        };
        assert_eq!(bundle.validate(20), Ok(20));
        assert_eq!(bundle.validate(19), Err(ResourceError::Budget));
        bundle.terrain.rgba = &[0; 4];
        assert_eq!(bundle.validate(20), Err(ResourceError::InvalidPixels));
        bundle.terrain.size = 0;
        assert_eq!(bundle.validate(20), Err(ResourceError::InvalidDimensions));
        assert_eq!(
            byte_len(u32::MAX, u32::MAX, u32::MAX),
            Err(ResourceError::Budget)
        );
    }
}
