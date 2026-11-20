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
/// Checked RGBA8 byte length for producer-side validation before decoding.
pub fn image_bytes(width: u32, height: u32) -> Result<usize, ResourceError> {
    byte_len(width, height, 1)
}
impl TextureLayers<'_> {
    pub fn validate(&self) -> Result<usize, ResourceError> {
        let bytes = byte_len(self.size, self.size, self.layers)?;
        if self.rgba.len() != bytes {
            return Err(ResourceError::InvalidPixels);
        }
        Ok(bytes)
    }
}
impl Resources<'_> {
    /// Validate before allocation. `max_bytes` bounds the total borrowed base
    /// pixels, not peak staging/VRAM usage; A01 must budget those separately.
    pub fn validate(&self, max_bytes: usize) -> Result<usize, ResourceError> {
        let mut total = self.terrain.validate()?;
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

/// CPU preparation bounds, independent from measured driver limits/VRAM.
#[derive(Clone, Copy, Debug)]
pub struct TerrainAtlasLimits {
    pub page_size: u32,
    pub max_pages: usize,
    pub max_layers: usize,
    /// One decoded page plus the borrowed layer-slice table. Layout metadata is
    /// separately bounded by max_layers; this is not a complete process peak.
    pub staging_bytes: usize,
    /// Mip levels below each page's base level that the layout has to survive.
    ///
    /// The gutter around every sprite is `1 << mip_levels` texels, as vanilla's
    /// `Stitcher` pads by `(1 << mipLevel) << anisotropyBits`. A one-texel gutter
    /// works at level zero and is gone by level two: the halving collapses it to
    /// zero texels, so a block face's edge texel bilinearly blends its neighbour
    /// sprite -- or an unwritten page texel, which is transparent -- into every
    /// face. The cutout pass then discards those fragments, and the world shows a
    /// block-sized grid of holes from the distance the chain takes over. Sixteen
    /// texels (four levels) still leaves one texel of gutter at the last level.
    pub mip_levels: u32,
}
impl Default for TerrainAtlasLimits {
    fn default() -> Self {
        Self {
            page_size: 512,
            max_pages: 128,
            max_layers: 32768,
            staging_bytes: 2 << 20,
            mip_levels: 0,
        }
    }
}
impl TerrainAtlasLimits {
    /// The gutter every sprite gets, `1 << mip_levels` texels as vanilla pads.
    pub fn padding(&self) -> u32 {
        1 << self.mip_levels.min(4)
    }
}

/// Layer IDs remain placement indices. Borrow pixels; decode/upload one padded
/// page at a time instead of keeping a second complete terrain atlas in RAM.
pub struct TerrainAtlas<'a> {
    layout: crate::atlas::AtlasLayout,
    layers: Vec<&'a [u8]>,
    page_bytes: usize,
}
impl<'a> TerrainAtlas<'a> {
    pub fn new(
        terrain: &TextureLayers<'a>,
        limits: TerrainAtlasLimits,
    ) -> Result<Self, ResourceError> {
        let total = byte_len(terrain.size, terrain.size, terrain.layers)?;
        if terrain.rgba.len() != total {
            return Err(ResourceError::InvalidPixels);
        }
        if terrain.layers > 32768 || terrain.layers as usize > limits.max_layers {
            return Err(ResourceError::Budget);
        }
        let page_bytes = byte_len(limits.page_size, limits.page_size, 1)?;
        let table = (terrain.layers as usize)
            .checked_mul(std::mem::size_of::<&[u8]>())
            .and_then(|n| n.checked_add(page_bytes))
            .ok_or(ResourceError::Budget)?;
        if table > limits.staging_bytes {
            return Err(ResourceError::Budget);
        }
        let sizes = vec![
            crate::atlas::SpriteSize {
                width: terrain.size,
                height: terrain.size
            };
            terrain.layers as usize
        ];
        let layout =
            crate::atlas::pack_atlas(&sizes, limits.page_size, limits.padding(), limits.max_pages)
                .map_err(|e| match e {
                    crate::atlas::AtlasError::TooManyPages => ResourceError::Budget,
                    _ => ResourceError::InvalidDimensions,
                })?;
        let layer_bytes = byte_len(terrain.size, terrain.size, 1)?;
        let layers = terrain.rgba.chunks_exact(layer_bytes).collect();
        Ok(Self {
            layout,
            layers,
            page_bytes,
        })
    }
    pub fn layout(&self) -> &crate::atlas::AtlasLayout {
        &self.layout
    }
    /// One sprite layer's edge, in pixels: every layer of this atlas is the same square.
    pub fn size(&self) -> u32 {
        self.layout.sprites().first().map_or(0, |place| place.width)
    }
    /// One layer's pixels, in the layer-major order the mesher's IDs use.
    pub fn layer(&self, index: usize) -> Option<&'a [u8]> {
        self.layers.get(index).copied()
    }
    /// Every layer, in the same order as `layout().sprites()`.
    pub fn layers(&self) -> &[&'a [u8]] {
        &self.layers
    }
    pub fn texture_bytes(&self) -> Result<usize, ResourceError> {
        self.page_bytes
            .checked_mul(self.layout.page_count())
            .ok_or(ResourceError::Budget)
    }
    /// What the same pages cost with `levels` mip levels below each one: every
    /// level is a quarter of the one above, and the whole chain three quarters of
    /// the base page.
    pub fn texture_bytes_with_mips(&self, levels: u32) -> Result<usize, ResourceError> {
        let pages = self.layout.page_count();
        let mut total = self.texture_bytes()?;
        for level in 1..=levels {
            if self.layout.page_size() >> level == 0 {
                break;
            }
            let level_bytes = self.page_bytes >> (2 * level);
            total = total
                .checked_add(
                    level_bytes
                        .checked_mul(pages)
                        .ok_or(ResourceError::Budget)?,
                )
                .ok_or(ResourceError::Budget)?;
        }
        Ok(total)
    }
    pub fn page(&self, page: usize) -> Result<Vec<u8>, crate::atlas::AtlasError> {
        crate::atlas::build_page(&self.layout, page, &self.layers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::{SpriteSize, mip_level_box, pack_atlas};
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
    /// The gutter is vanilla's `1 << mipLevels`: one texel per level of chain,
    /// so the deepest level still has a texel of padding on every side. A flat
    /// one-texel gutter is gone by level two (see the field's comment).
    #[test]
    fn the_gutter_keeps_a_texel_per_level_of_chain() {
        for levels in 0..=4 {
            let padding = TerrainAtlasLimits {
                mip_levels: levels,
                ..TerrainAtlasLimits::default()
            }
            .padding();
            assert_eq!(padding, 1 << levels, "levels {levels}");
            assert!(padding >> levels >= 1, "levels {levels} leaves no gutter");
        }
    }
    /// 64 px sprites in a 1024 px page, as the shipped terrain atlas packs them:
    /// every sprite keeps at least one texel of its own edge around it even at
    /// the deepest level. With a one-texel gutter a sprite at an even coordinate
    /// had `pad` zero at level two, so its edge texel sat against the
    /// neighbour's and bilinear sampling at distance blended the two -- the
    /// reported block-sized grid.
    #[test]
    fn a_packed_page_keeps_a_gutter_at_the_deepest_level() {
        let layout = pack_atlas(
            &[SpriteSize {
                width: 64,
                height: 64,
            }; 96],
            1024,
            TerrainAtlasLimits {
                mip_levels: 4,
                ..TerrainAtlasLimits::default()
            }
            .padding(),
            16,
        )
        .unwrap();
        for place in layout.sprites() {
            let bounds = mip_level_box(&layout, place, 4).unwrap();
            for axis in 0..2 {
                assert!(
                    bounds.pad[axis] >= 1,
                    "level 4 pad {:?} on sprite at {:?}",
                    bounds.pad,
                    (place.x, place.y)
                );
                assert!(bounds.at[axis] + bounds.size[axis] <= layout.page_size() >> 4);
            }
        }
    }
    #[test]
    fn terrain_paging_borrows_pixels_preserves_ids_and_extrudes_each_layer() {
        let pixels = [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255];
        let terrain = TextureLayers {
            size: 1,
            layers: 3,
            rgba: &pixels,
        };
        let limits = TerrainAtlasLimits {
            page_size: 4,
            max_pages: 3,
            max_layers: 3,
            staging_bytes: 64 + 3 * std::mem::size_of::<&[u8]>(),
            mip_levels: 0,
        };
        let plan = TerrainAtlas::new(&terrain, limits).unwrap();
        assert_eq!(plan.layers[0].as_ptr(), pixels.as_ptr());
        assert_eq!(plan.texture_bytes(), Ok(192));
        for index in 0..3 {
            let place = plan.layout().sprites()[index];
            assert_eq!(place.page, index);
            let page = plan.page(index).unwrap();
            for (x, y) in [(0, 0), (1, 1), (2, 2)] {
                assert_eq!(&page[(y * 4 + x) * 4..][..4], &pixels[index * 4..][..4]);
            }
        }
        assert!(plan.page(3).is_err());
        assert!(
            TerrainAtlas::new(
                &terrain,
                TerrainAtlasLimits {
                    staging_bytes: limits.staging_bytes - 1,
                    ..limits
                }
            )
            .is_err()
        );
        assert!(
            TerrainAtlas::new(
                &terrain,
                TerrainAtlasLimits {
                    max_pages: 2,
                    ..limits
                }
            )
            .is_err()
        );
        assert!(
            TerrainAtlas::new(
                &terrain,
                TerrainAtlasLimits {
                    max_layers: 2,
                    ..limits
                }
            )
            .is_err()
        );
        assert!(
            TerrainAtlas::new(
                &TextureLayers {
                    size: 1,
                    layers: 3,
                    rgba: &pixels[..8]
                },
                limits
            )
            .is_err()
        );
    }
}
