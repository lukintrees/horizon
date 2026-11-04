//! CPU-side building blocks for the experimental OpenGL 2 renderer.
//!
//! This module does not create a GL context and is not connected to the game renderer yet.
//! A single-level atlas is not sufficient for mipmapped terrain.

/// Prototype shaders; they are not used by the current renderer.
pub const TERRAIN_VERTEX_SHADER: &str = include_str!("legacy_gl_terrain.vert");
pub const TERRAIN_FRAGMENT_SHADER: &str = include_str!("legacy_gl_terrain.frag");

/// A sprite's unpadded dimensions in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpriteSize {
    pub width: u32,
    pub height: u32,
}

/// The sprite's content rectangle (excluding padding) on a square atlas page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub page: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A deterministic layout. Placements are in the same order as the input sprites.
#[derive(Debug, PartialEq, Eq)]
pub struct AtlasLayout {
    page_size: u32,
    padding: u32,
    pages: usize,
    sprites: Vec<Placement>,
}

impl AtlasLayout {
    pub fn page_size(&self) -> u32 {
        self.page_size
    }
    pub fn padding(&self) -> u32 {
        self.padding
    }
    pub fn page_count(&self) -> usize {
        self.pages
    }
    pub fn sprites(&self) -> &[Placement] {
        &self.sprites
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AtlasError {
    InvalidPageSize,
    EmptySprite { index: usize },
    SpriteTooLarge { index: usize },
    TooManyPages,
    InvalidPixels { index: usize },
    InvalidSpriteArray,
    StaleAssets,
    InvalidPage,
    InvalidSpriteIndex,
    SizeOverflow,
    OutOfMemory,
}

/// Packs sprites onto fixed-size power-of-two pages using shelves.
///
/// This deliberately trades space efficiency for a simple, deterministic implementation.
/// No rotation is performed. `max_pages` bounds resource use, rather than assuming a
/// particular GPU's texture or memory limits. Padding is reserved on all four sides.
pub fn pack_atlas(
    sprites: &[SpriteSize],
    page_size: u32,
    padding: u32,
    max_pages: usize,
) -> Result<AtlasLayout, AtlasError> {
    if !page_size.is_power_of_two() {
        return Err(AtlasError::InvalidPageSize);
    }
    let border = padding.checked_mul(2).ok_or(AtlasError::SizeOverflow)?;
    let mut placements = Vec::with_capacity(sprites.len());
    let (mut pages, mut x, mut y, mut shelf_height) = (0usize, 0u32, 0u32, 0u32);

    for (index, sprite) in sprites.iter().enumerate() {
        if sprite.width == 0 || sprite.height == 0 {
            return Err(AtlasError::EmptySprite { index });
        }
        let width = sprite
            .width
            .checked_add(border)
            .ok_or(AtlasError::SizeOverflow)?;
        let height = sprite
            .height
            .checked_add(border)
            .ok_or(AtlasError::SizeOverflow)?;
        if width > page_size || height > page_size {
            return Err(AtlasError::SpriteTooLarge { index });
        }
        if pages == 0 {
            if max_pages == 0 {
                return Err(AtlasError::TooManyPages);
            }
            pages = 1;
        }
        if width > page_size - x {
            x = 0;
            y += shelf_height;
            shelf_height = 0;
        }
        if height > page_size - y {
            if pages >= max_pages {
                return Err(AtlasError::TooManyPages);
            }
            pages += 1;
            x = 0;
            y = 0;
            shelf_height = 0;
        }
        placements.push(Placement {
            page: pages - 1,
            x: x + padding,
            y: y + padding,
            width: sprite.width,
            height: sprite.height,
        });
        x += width;
        shelf_height = shelf_height.max(height);
    }
    Ok(AtlasLayout {
        page_size,
        padding,
        pages,
        sprites: placements,
    })
}

/// RGBA8 source pixels in row-major order, one slice for each input sprite.
/// Produces one page at a time and replicates edge pixels into its padding.
/// This builds only level zero: callers must not enable mipmaps based on it.
pub fn build_page(
    layout: &AtlasLayout,
    page: usize,
    pixels: &[&[u8]],
) -> Result<Vec<u8>, AtlasError> {
    if page >= layout.pages {
        return Err(AtlasError::InvalidPage);
    }
    if pixels.len() != layout.sprites.len() {
        return Err(AtlasError::InvalidSpriteArray);
    }
    let side = usize::try_from(layout.page_size).map_err(|_| AtlasError::SizeOverflow)?;
    let bytes = side
        .checked_mul(side)
        .and_then(|n| n.checked_mul(4))
        .ok_or(AtlasError::SizeOverflow)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(bytes)
        .map_err(|_| AtlasError::OutOfMemory)?;
    output.resize(bytes, 0);
    for (index, (place, source)) in layout.sprites.iter().zip(pixels).enumerate() {
        let w = usize::try_from(place.width).map_err(|_| AtlasError::SizeOverflow)?;
        let h = usize::try_from(place.height).map_err(|_| AtlasError::SizeOverflow)?;
        let expected = w
            .checked_mul(h)
            .and_then(|n| n.checked_mul(4))
            .ok_or(AtlasError::SizeOverflow)?;
        if source.len() != expected {
            return Err(AtlasError::InvalidPixels { index });
        }
        if place.page != page {
            continue;
        }
        let pad = usize::try_from(layout.padding).map_err(|_| AtlasError::SizeOverflow)?;
        let left = usize::try_from(place.x).map_err(|_| AtlasError::SizeOverflow)? - pad;
        let top = usize::try_from(place.y).map_err(|_| AtlasError::SizeOverflow)? - pad;
        for dy in 0..h + pad * 2 {
            for dx in 0..w + pad * 2 {
                let sx = dx.saturating_sub(pad).min(w - 1);
                let sy = dy.saturating_sub(pad).min(h - 1);
                let from = (sy * w + sx) * 4;
                let to = ((top + dy) * side + left + dx) * 4;
                output[to..to + 4].copy_from_slice(&source[from..from + 4]);
            }
        }
    }
    Ok(output)
}

fn layout_for_layers(
    size: u32,
    count: usize,
    pixels: &[u8],
    page_size: u32,
    padding: u32,
    max_pages: usize,
) -> Result<AtlasLayout, AtlasError> {
    let side = usize::try_from(size).map_err(|_| AtlasError::SizeOverflow)?;
    let layer_bytes = side
        .checked_mul(side)
        .and_then(|n| n.checked_mul(4))
        .ok_or(AtlasError::SizeOverflow)?;
    if layer_bytes == 0
        || pixels.len()
            != layer_bytes
                .checked_mul(count)
                .ok_or(AtlasError::SizeOverflow)?
    {
        return Err(AtlasError::InvalidSpriteArray);
    }
    let sizes = vec![
        SpriteSize {
            width: size,
            height: size
        };
        count
    ];
    pack_atlas(&sizes, page_size, padding, max_pages)
}

fn build_layer_page(
    layout: &AtlasLayout,
    page: usize,
    size: u32,
    pixels: &[u8],
) -> Result<Vec<u8>, AtlasError> {
    let side = usize::try_from(size).map_err(|_| AtlasError::SizeOverflow)?;
    let layer_bytes = side
        .checked_mul(side)
        .and_then(|n| n.checked_mul(4))
        .ok_or(AtlasError::SizeOverflow)?;
    if layer_bytes == 0
        || pixels.len()
            != layer_bytes
                .checked_mul(layout.sprites.len())
                .ok_or(AtlasError::SizeOverflow)?
        || layout
            .sprites
            .iter()
            .any(|p| p.width != size || p.height != size)
    {
        return Err(AtlasError::InvalidSpriteArray);
    }
    let layers: Vec<&[u8]> = pixels.chunks_exact(layer_bytes).collect();
    build_page(layout, page, &layers)
}

/// Build a layout from the actual terrain sprite array after `Sprites::finish`.
/// `finish_without_pixels` is unsuitable because no images are available there.
pub fn layout_for_sprites(
    sprites: &crate::sprites::Sprites,
    page_size: u32,
    padding: u32,
    max_pages: usize,
) -> Result<AtlasLayout, AtlasError> {
    layout_for_layers(
        sprites.size,
        sprites.opaque.len(),
        &sprites.pixels,
        page_size,
        padding,
        max_pages,
    )
}

/// Produces a level-zero page from the same `Sprites` instance as the layout.
pub fn build_sprites_page(
    layout: &AtlasLayout,
    page: usize,
    sprites: &crate::sprites::Sprites,
) -> Result<Vec<u8>, AtlasError> {
    build_layer_page(layout, page, sprites.size, &sprites.pixels)
}

/// A layout derived from the render assets the game actually hands to `Renderer`.
/// Its generation prevents mixing UVs/pages with a resource pack reloaded later.
pub struct RenderAssetAtlas {
    generation: u64,
    layout: AtlasLayout,
}
impl RenderAssetAtlas {
    pub fn prepare(
        assets: &crate::assets::RenderAssets,
        page_size: u32,
        padding: u32,
        max_pages: usize,
    ) -> Result<Self, AtlasError> {
        let count = usize::try_from(assets.texture_layers).map_err(|_| AtlasError::SizeOverflow)?;
        let layout = layout_for_layers(
            assets.sprite_size,
            count,
            &assets.pixels,
            page_size,
            padding,
            max_pages,
        )?;
        Ok(Self {
            generation: assets.generation,
            layout,
        })
    }
    pub fn layout(&self) -> &AtlasLayout {
        &self.layout
    }
    /// Build one page only from the same resource generation. This allows the
    /// caller to upload and release each CPU page rather than holding all pages.
    pub fn build_page(
        &self,
        assets: &crate::assets::RenderAssets,
        page: usize,
    ) -> Result<Vec<u8>, AtlasError> {
        if assets.generation != self.generation {
            return Err(AtlasError::StaleAssets);
        }
        build_layer_page(&self.layout, page, assets.sprite_size, &assets.pixels)
    }
}

/// A tightly packed, padded region suitable for `glTexSubImage2D` at level zero.
#[derive(Debug, PartialEq, Eq)]
pub struct SpriteUpdate {
    pub page: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Prepare one animation frame and update its padding together with its contents.
/// The frame must already have been resized to the dimensions used by the atlas.
pub fn sprite_update(
    layout: &AtlasLayout,
    index: usize,
    pixels: &[u8],
) -> Result<SpriteUpdate, AtlasError> {
    let place = layout
        .sprites
        .get(index)
        .ok_or(AtlasError::InvalidSpriteIndex)?;
    let border = layout
        .padding
        .checked_mul(2)
        .ok_or(AtlasError::SizeOverflow)?;
    let width = place
        .width
        .checked_add(border)
        .ok_or(AtlasError::SizeOverflow)?;
    let height = place
        .height
        .checked_add(border)
        .ok_or(AtlasError::SizeOverflow)?;
    let w = usize::try_from(place.width).map_err(|_| AtlasError::SizeOverflow)?;
    let h = usize::try_from(place.height).map_err(|_| AtlasError::SizeOverflow)?;
    let padded_w = usize::try_from(width).map_err(|_| AtlasError::SizeOverflow)?;
    let padded_h = usize::try_from(height).map_err(|_| AtlasError::SizeOverflow)?;
    let expected = w
        .checked_mul(h)
        .and_then(|n| n.checked_mul(4))
        .ok_or(AtlasError::SizeOverflow)?;
    if pixels.len() != expected {
        return Err(AtlasError::InvalidPixels { index });
    }
    let bytes = padded_w
        .checked_mul(padded_h)
        .and_then(|n| n.checked_mul(4))
        .ok_or(AtlasError::SizeOverflow)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(bytes)
        .map_err(|_| AtlasError::OutOfMemory)?;
    output.resize(bytes, 0);
    let pad = usize::try_from(layout.padding).map_err(|_| AtlasError::SizeOverflow)?;
    for dy in 0..padded_h {
        for dx in 0..padded_w {
            let sx = dx.saturating_sub(pad).min(w - 1);
            let sy = dy.saturating_sub(pad).min(h - 1);
            let from = (sy * w + sx) * 4;
            let to = (dy * padded_w + dx) * 4;
            output[to..to + 4].copy_from_slice(&pixels[from..from + 4]);
        }
    }
    Ok(SpriteUpdate {
        page: place.page,
        x: place.x - layout.padding,
        y: place.y - layout.padding,
        width,
        height,
        pixels: output,
    })
}

/// Samples an existing animation on the CPU and prepares its padded atlas update.
/// Interpolated frames use the same integer rounding as the modern CPU fallback.
/// Callers should cache `animation.blend(tick)` and upload only when it changes.
pub fn animation_update(
    layout: &AtlasLayout,
    animation: &crate::sprites::SpriteAnimation,
    tick: u64,
) -> Result<SpriteUpdate, AtlasError> {
    let index = usize::try_from(animation.layer).map_err(|_| AtlasError::InvalidSpriteIndex)?;
    let place = layout
        .sprites
        .get(index)
        .ok_or(AtlasError::InvalidSpriteIndex)?;
    let blend = animation.blend(tick);
    let mut current = animation.resized_pixels(blend.frame, place.width, place.height);
    if blend.millis != 0 {
        let next = animation.resized_pixels(blend.next, place.width, place.height);
        for (a, b) in current.iter_mut().zip(next) {
            *a = ((u32::from(*a) * (1000 - blend.millis) + u32::from(b) * blend.millis + 500)
                / 1000) as u8;
        }
    }
    sprite_update(layout, index, &current)
}

/// The backend's GL vertex type; terrain conversion produces upload-ready vertices.
/// `TerrainVertex` alpha encodes emission, not transparency, so conversion uses 1.0.
pub use horizon_legacy_gl::Vertex as LegacyTerrainVertex;

#[derive(Debug, PartialEq, Eq)]
pub enum TerrainConversionError {
    MissingSprite {
        layer: u16,
    },
    MixedSpriteLayers,
    /// A sprite boundary can otherwise sample a neighboring atlas entry.
    MissingPadding,
    RepeatingUv,
}

/// Converts one section quad, returning its atlas page and four vertices.
///
/// This initial path explicitly rejects UVs outside [0, 1]: repeating within a
/// sprite cannot be emulated by the GL texture wrap mode on an atlas page.
/// It does not handle tiled distant terrain, packed meshes or emissive lighting.
pub fn convert_terrain_quad(
    layout: &AtlasLayout,
    quad: &[crate::mesher::TerrainVertex; 4],
) -> Result<(usize, [LegacyTerrainVertex; 4]), TerrainConversionError> {
    if quad.iter().any(|v| v.uv().iter().any(|&uv| uv > 1.0)) {
        return Err(TerrainConversionError::RepeatingUv);
    }
    convert_terrain_quad_unwrapped(layout, quad)
}

fn convert_terrain_quad_unwrapped(
    layout: &AtlasLayout,
    quad: &[crate::mesher::TerrainVertex; 4],
) -> Result<(usize, [LegacyTerrainVertex; 4]), TerrainConversionError> {
    use crate::mesher::OPAQUE_LAYER;
    if layout.padding == 0 {
        return Err(TerrainConversionError::MissingPadding);
    }
    let layer = quad[0].layer & !OPAQUE_LAYER;
    let sprite = layout
        .sprites
        .get(usize::from(layer))
        .ok_or(TerrainConversionError::MissingSprite { layer })?;
    if quad.iter().any(|v| v.layer & !OPAQUE_LAYER != layer) {
        return Err(TerrainConversionError::MixedSpriteLayers);
    }
    let side = layout.page_size as f32;
    let vertices = quad.map(|v| {
        let uv = v.uv();
        let [r, g, b, _] = v.color.to_le_bytes();
        let sky = f32::from(v.light & 255);
        let block = f32::from(v.light >> 8);
        let light_coord = |value: f32| (value / 256.0 + 0.5 / 16.0).clamp(0.5 / 16.0, 15.5 / 16.0);
        LegacyTerrainVertex {
            pos: v.pos,
            uv: [
                (sprite.x as f32 + uv[0] * sprite.width as f32) / side,
                (sprite.y as f32 + uv[1] * sprite.height as f32) / side,
            ],
            light_uv: [light_coord(block), light_coord(sky)],
            color: [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                1.0,
            ],
        }
    });
    Ok((sprite.page, vertices))
}

/// Calculate the section origin relative to the camera in double precision
/// before converting it to the float uniform used by the GL 2 terrain shader.
/// This prevents large world coordinates from quantizing nearby geometry.
pub fn camera_relative_origin(section_origin: [i32; 3], camera: [f64; 3]) -> Option<[f32; 3]> {
    let mut origin = [0.0; 3];
    for axis in 0..3 {
        let relative = f64::from(section_origin[axis]) - camera[axis];
        if !relative.is_finite() {
            return None;
        }
        origin[axis] = relative as f32;
        if !origin[axis].is_finite() {
            return None;
        }
    }
    Some(origin)
}

/// One contiguous draw run. Keeping page changes in input order preserves
/// a caller's back-to-front translucent ordering. A run uses 16-bit indices.
#[derive(Debug, PartialEq)]
pub struct TerrainRun {
    pub page: usize,
    pub vertices: Vec<LegacyTerrainVertex>,
    pub indices: Vec<u16>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TerrainBatchError {
    IncompleteQuad,
    Quad {
        index: usize,
        error: TerrainConversionError,
    },
}

// A point on a source triangle. UVs are kept in sprite coordinates until
// clipping has separated each repeat: atlas texture wrapping cannot do that.
#[derive(Clone, Copy)]
struct ClipVertex {
    vertex: LegacyTerrainVertex,
    uv: [f32; 2],
}

impl ClipVertex {
    fn lerp(self, other: Self, t: f32) -> Self {
        fn blend<const N: usize>(a: [f32; N], b: [f32; N], t: f32) -> [f32; N] {
            std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
        }
        Self {
            uv: blend(self.uv, other.uv, t),
            vertex: LegacyTerrainVertex {
                pos: blend(self.vertex.pos, other.vertex.pos, t),
                uv: blend(self.vertex.uv, other.vertex.uv, t),
                light_uv: blend(self.vertex.light_uv, other.vertex.light_uv, t),
                color: blend(self.vertex.color, other.vertex.color, t),
            },
        }
    }
}

// Clip a convex polygon against an inclusive axis-aligned UV boundary.
fn clip_uv(input: &[ClipVertex], axis: usize, boundary: f32, lower: bool) -> Vec<ClipVertex> {
    let mut output = Vec::with_capacity(input.len() + 1);
    let Some(&last) = input.last() else {
        return output;
    };
    let inside = |v: ClipVertex| {
        if lower {
            v.uv[axis] >= boundary
        } else {
            v.uv[axis] <= boundary
        }
    };
    let mut previous = last;
    let mut previous_inside = inside(previous);
    for &current in input {
        let current_inside = inside(current);
        if current_inside != previous_inside {
            let t = (boundary - previous.uv[axis]) / (current.uv[axis] - previous.uv[axis]);
            let mut crossing = previous.lerp(current, t);
            crossing.uv[axis] = boundary;
            output.push(crossing);
        }
        if current_inside {
            output.push(current);
        }
        previous = current;
        previous_inside = current_inside;
    }
    output
}

fn append_polygon(runs: &mut Vec<TerrainRun>, page: usize, polygon: &[LegacyTerrainVertex]) {
    if polygon.len() < 3 {
        return;
    }
    if runs
        .last()
        .is_none_or(|run| run.page != page || run.vertices.len() + polygon.len() > 65536)
    {
        runs.push(TerrainRun {
            page,
            vertices: Vec::new(),
            indices: Vec::new(),
        });
    }
    let run = runs.last_mut().expect("run was created");
    let base = run.vertices.len() as u16;
    run.vertices.extend_from_slice(polygon);
    for i in 1..polygon.len() - 1 {
        run.indices
            .extend_from_slice(&[base, base + i as u16, base + i as u16 + 1]);
    }
}

/// Convert ordinary section quads into ordered atlas page runs. UVs beyond one
/// repeat the sprite: each source triangle is clipped at integer UV boundaries
/// and interpolated independently, preserving its original diagonal and shading.
/// This path uses the section UV scale; distant terrain uses a different encoding
/// and is not supported. Each generated run fits 16-bit indices.
pub fn convert_terrain_runs(
    layout: &AtlasLayout,
    vertices: &[crate::mesher::TerrainVertex],
) -> Result<Vec<TerrainRun>, TerrainBatchError> {
    if vertices.len() % 4 != 0 {
        return Err(TerrainBatchError::IncompleteQuad);
    }
    let mut runs = Vec::new();
    for (index, chunk) in vertices.chunks_exact(4).enumerate() {
        let quad: &[crate::mesher::TerrainVertex; 4] = chunk.try_into().expect("chunks_exact(4)");
        let (page, converted) = convert_terrain_quad_unwrapped(layout, quad)
            .map_err(|error| TerrainBatchError::Quad { index, error })?;
        if quad.iter().all(|v| v.uv().iter().all(|&uv| uv <= 1.0)) {
            append_polygon(&mut runs, page, &converted);
            continue;
        }
        let layer = quad[0].layer & !crate::mesher::OPAQUE_LAYER;
        let sprite = layout.sprites[usize::from(layer)];
        let side = layout.page_size as f32;
        for triangle in [[0, 1, 2], [0, 2, 3]] {
            let triangle = triangle.map(|i| ClipVertex {
                vertex: converted[i],
                uv: quad[i].uv(),
            });
            let ranges = [0, 1].map(|axis| {
                let min = triangle
                    .iter()
                    .map(|v| v.uv[axis])
                    .fold(f32::INFINITY, f32::min);
                let max = triangle
                    .iter()
                    .map(|v| v.uv[axis])
                    .fold(f32::NEG_INFINITY, f32::max);
                // A zero-width UV axis on a seam belongs to the lower tile, as in tile_uv.
                let first = if min == max && min > 0.0 {
                    min.ceil() as usize - 1
                } else {
                    min.floor() as usize
                };
                let last = if max == 0.0 {
                    0
                } else {
                    max.ceil() as usize - 1
                };
                (first, last.max(first))
            });
            for y in ranges[1].0..=ranges[1].1 {
                for x in ranges[0].0..=ranges[0].1 {
                    let mut poly = triangle.to_vec();
                    for (axis, tile) in [(0, x), (1, y)] {
                        poly = clip_uv(&poly, axis, tile as f32, true);
                        poly = clip_uv(&poly, axis, tile as f32 + 1.0, false);
                    }
                    let polygon: Vec<_> = poly
                        .into_iter()
                        .map(|v| {
                            let mut vertex = v.vertex;
                            vertex.uv = [
                                (sprite.x as f32
                                    + (v.uv[0] - x as f32).clamp(0.0, 1.0) * sprite.width as f32)
                                    / side,
                                (sprite.y as f32
                                    + (v.uv[1] - y as f32).clamp(0.0, 1.0) * sprite.height as f32)
                                    / side,
                            ];
                            vertex
                        })
                        .collect();
                    append_polygon(&mut runs, page, &polygon);
                }
            }
        }
    }
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_reserves_padding_and_starts_new_pages() {
        let sizes = [SpriteSize {
            width: 2,
            height: 2,
        }; 5];
        let layout = pack_atlas(&sizes, 8, 1, 2).unwrap();
        assert_eq!(layout.pages, 2);
        assert_eq!(
            layout
                .sprites
                .iter()
                .map(|p| (p.page, p.x, p.y))
                .collect::<Vec<_>>(),
            vec![(0, 1, 1), (0, 5, 1), (0, 1, 5), (0, 5, 5), (1, 1, 1)]
        );
        assert_eq!(pack_atlas(&sizes, 8, 1, 1), Err(AtlasError::TooManyPages));
    }

    #[test]
    fn page_replicates_corners_and_edges_without_touching_other_sprites() {
        let sizes = [
            SpriteSize {
                width: 2,
                height: 2,
            },
            SpriteSize {
                width: 1,
                height: 1,
            },
        ];
        let layout = pack_atlas(&sizes, 8, 1, 1).unwrap();
        let a = [1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255];
        let b = [9, 0, 0, 255];
        let page = build_page(&layout, 0, &[&a, &b]).unwrap();
        let red = |x: usize, y: usize| page[(y * 8 + x) * 4];
        assert_eq!((red(0, 0), red(3, 0), red(0, 3), red(3, 3)), (1, 2, 3, 4));
        assert_eq!((red(4, 0), red(6, 2), red(7, 0)), (9, 9, 0));
        assert_eq!(
            build_page(&layout, 0, &[&a, &[]]),
            Err(AtlasError::InvalidPixels { index: 1 })
        );
    }

    #[test]
    fn rejects_invalid_inputs_without_allocating_pages() {
        assert_eq!(pack_atlas(&[], 3, 0, 1), Err(AtlasError::InvalidPageSize));
        assert_eq!(
            pack_atlas(
                &[SpriteSize {
                    width: 0,
                    height: 1
                }],
                8,
                0,
                1
            ),
            Err(AtlasError::EmptySprite { index: 0 })
        );
        assert_eq!(
            pack_atlas(
                &[SpriteSize {
                    width: 8,
                    height: 8
                }],
                8,
                1,
                1
            ),
            Err(AtlasError::SpriteTooLarge { index: 0 })
        );
        assert_eq!(
            pack_atlas(
                &[SpriteSize {
                    width: 1,
                    height: 1
                }],
                8,
                u32::MAX,
                1
            ),
            Err(AtlasError::SizeOverflow)
        );
        assert_eq!(pack_atlas(&[], 8, 0, 0).unwrap().pages, 0);
    }

    #[test]
    fn converts_quad_uv_color_and_light_and_rejects_repetition() {
        use crate::mesher::{OPAQUE_LAYER, TerrainVertex};
        let layout = pack_atlas(
            &[SpriteSize {
                width: 2,
                height: 2,
            }],
            8,
            1,
            1,
        )
        .unwrap();
        let v = TerrainVertex::new(
            [1.0, 2.0, 3.0],
            [0.5, 1.0],
            OPAQUE_LAYER,
            0x80402010,
            0xf080,
        );
        let (page, vertices) = convert_terrain_quad(&layout, &[v; 4]).unwrap();
        assert_eq!(page, 0);
        assert_eq!(vertices[0].pos, [1.0, 2.0, 3.0]);
        assert_eq!(vertices[0].uv, [0.25, 0.375]);
        assert_eq!(vertices[0].light_uv, [0.96875, 0.53125]);
        assert_eq!(
            vertices[0].color,
            [16.0 / 255.0, 32.0 / 255.0, 64.0 / 255.0, 1.0]
        );
        let repeat = TerrainVertex::new([0.0; 3], [1.5, 0.0], 0, 0, 0);
        assert_eq!(
            convert_terrain_quad(&layout, &[repeat; 4]),
            Err(TerrainConversionError::RepeatingUv)
        );
        let other = TerrainVertex::new([0.0; 3], [0.0; 2], 1, 0, 0);
        assert_eq!(
            convert_terrain_quad(&layout, &[v, v, v, other]),
            Err(TerrainConversionError::MixedSpriteLayers)
        );
        assert_eq!(
            convert_terrain_quad(&layout, &[other; 4]),
            Err(TerrainConversionError::MissingSprite { layer: 1 })
        );
    }
    #[test]
    fn repeated_uv_triangles_split_at_seams_and_preserve_interpolation() {
        use crate::mesher::TerrainVertex;
        let layout = pack_atlas(
            &[SpriteSize {
                width: 4,
                height: 4,
            }],
            8,
            1,
            1,
        )
        .unwrap();
        let vertex = |x: f32, y: f32| {
            TerrainVertex::new(
                [x, y, 0.0],
                [x * 1.5, y * 1.5],
                0,
                if x == 0.0 { 0xff000000 } else { 0xff0000ff },
                0xf0f0,
            )
        };
        let quad = [
            vertex(0., 0.),
            vertex(1., 0.),
            vertex(1., 1.),
            vertex(0., 1.),
        ];
        let runs = convert_terrain_runs(&layout, &quad).unwrap();
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert!(run.vertices.len() > 4);
        let mut area = 0.0;
        for tri in run.indices.chunks_exact(3) {
            let [a, b, c] = [
                run.vertices[tri[0] as usize],
                run.vertices[tri[1] as usize],
                run.vertices[tri[2] as usize],
            ];
            let cross = (b.pos[0] - a.pos[0]) * (c.pos[1] - a.pos[1])
                - (b.pos[1] - a.pos[1]) * (c.pos[0] - a.pos[0]);
            assert!(cross >= -1e-6, "winding reversed");
            area += cross * 0.5;
        }
        assert!((area - 1.0).abs() < 1e-5, "area {area}");
        for v in &run.vertices {
            assert!((0.125..=0.625).contains(&v.uv[0]));
            assert!((0.125..=0.625).contains(&v.uv[1]));
            assert!((v.color[0] - v.pos[0]).abs() < 1e-5);
        }
        // The same seam position maps to both edges, not across a neighboring sprite.
        let seam: Vec<_> = run
            .vertices
            .iter()
            .filter(|v| (v.pos[0] - 2.0 / 3.0).abs() < 1e-5)
            .collect();
        assert!(seam.iter().any(|v| (v.uv[0] - 0.125).abs() < 1e-5));
        assert!(seam.iter().any(|v| (v.uv[0] - 0.625).abs() < 1e-5));
    }

    #[test]
    fn varied_repeated_uvs_conserve_each_quad_area() {
        use crate::mesher::TerrainVertex;
        let layout = pack_atlas(
            &[SpriteSize {
                width: 2,
                height: 2,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let mut seed = 7u32;
        for _ in 0..1000 {
            let mut next_uv = || {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed % 65536) as f32 / 32768.0
            };
            let positions = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];
            let quad = positions
                .map(|pos| TerrainVertex::new(pos, [next_uv(), next_uv()], 0, 0xffffffff, 0));
            let runs = convert_terrain_runs(&layout, &quad).unwrap();
            let mut area = 0.0;
            for run in runs {
                for t in run.indices.chunks_exact(3) {
                    let [a, b, c] = [
                        run.vertices[t[0] as usize].pos,
                        run.vertices[t[1] as usize].pos,
                        run.vertices[t[2] as usize].pos,
                    ];
                    let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                    assert!(cross >= -1e-6);
                    area += cross * 0.5;
                }
            }
            assert!((area - 1.0).abs() < 1e-4, "area {area}");
        }
    }

    #[test]
    fn repeated_constant_seam_and_split_index_limit() {
        use crate::mesher::TerrainVertex;
        let layout = pack_atlas(
            &[SpriteSize {
                width: 2,
                height: 2,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let v = |x, y| TerrainVertex::new([x, y, 0.], [1.0, y * 1.5], 0, 0xffffffff, 0);
        let quad = [v(0., 0.), v(1., 0.), v(1., 1.), v(0., 1.)];
        let runs = convert_terrain_runs(&layout, &quad).unwrap();
        assert!(
            runs[0]
                .vertices
                .iter()
                .all(|v| (v.uv[0] - 0.75).abs() < 1e-6)
        );
        let input = quad.repeat(10000);
        let runs = convert_terrain_runs(&layout, &input).unwrap();
        assert!(runs.len() > 1);
        assert!(runs.iter().all(|run| {
            run.vertices.len() <= 65536
                && run
                    .indices
                    .iter()
                    .all(|&i| usize::from(i) < run.vertices.len())
        }));
    }

    #[test]
    fn animated_update_matches_page_padding() {
        let layout = pack_atlas(
            &[SpriteSize {
                width: 2,
                height: 1,
            }],
            8,
            1,
            1,
        )
        .unwrap();
        let frame = [1, 2, 3, 255, 4, 5, 6, 255];
        let update = sprite_update(&layout, 0, &frame).unwrap();
        assert_eq!(
            (update.x, update.y, update.width, update.height),
            (0, 0, 4, 3)
        );
        let page = build_page(&layout, 0, &[&frame]).unwrap();
        for y in 0..3 {
            assert_eq!(
                &page[y * 8 * 4..y * 8 * 4 + 4 * 4],
                &update.pixels[y * 4 * 4..(y + 1) * 4 * 4]
            );
        }
        assert_eq!(
            sprite_update(&layout, 1, &frame),
            Err(AtlasError::InvalidSpriteIndex)
        );
        assert_eq!(
            sprite_update(&layout, 0, &frame[..4]),
            Err(AtlasError::InvalidPixels { index: 0 })
        );
    }

    #[test]
    fn builds_atlas_from_sprite_array_and_validates_source() {
        let mut sprites = crate::sprites::Sprites::default();
        sprites.size = 2;
        sprites.opaque = vec![true, false];
        sprites.pixels = vec![1; 16];
        sprites.pixels.extend_from_slice(&[2; 16]);
        let layout = layout_for_sprites(&sprites, 4, 0, 2).unwrap();
        let page = build_sprites_page(&layout, 0, &sprites).unwrap();
        assert_eq!((page[0], page[8]), (1, 2));
        sprites.pixels.pop();
        assert_eq!(
            layout_for_sprites(&sprites, 4, 0, 2),
            Err(AtlasError::InvalidSpriteArray)
        );
        assert_eq!(
            build_sprites_page(&layout, 0, &sprites),
            Err(AtlasError::InvalidSpriteArray)
        );
    }

    #[test]
    fn terrain_runs_preserve_order_and_split_u16_indices() {
        use crate::mesher::TerrainVertex;
        let layout = pack_atlas(
            &[SpriteSize {
                width: 2,
                height: 2,
            }; 2],
            4,
            1,
            2,
        )
        .unwrap();
        let vertex = |layer| TerrainVertex::new([0.0; 3], [0.0; 2], layer, 0, 0);
        let mut input = Vec::new();
        for layer in [0, 1, 0] {
            input.extend_from_slice(&[vertex(layer); 4]);
        }
        let no_padding = pack_atlas(
            &[SpriteSize {
                width: 2,
                height: 2,
            }],
            2,
            0,
            1,
        )
        .unwrap();
        assert_eq!(
            convert_terrain_runs(&no_padding, &[vertex(0); 4]),
            Err(TerrainBatchError::Quad {
                index: 0,
                error: TerrainConversionError::MissingPadding
            })
        );
        let runs = convert_terrain_runs(&layout, &input).unwrap();
        assert_eq!(
            runs.iter().map(|r| r.page).collect::<Vec<_>>(),
            vec![0, 1, 0]
        );
        assert_eq!(runs[0].indices, [0, 1, 2, 0, 2, 3]);
        let many = vec![vertex(0); 4 * (16384 + 1)];
        let runs = convert_terrain_runs(&layout, &many).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].vertices.len(), 65536);
        assert_eq!(runs[0].indices.last(), Some(&65535));
        assert_eq!(runs[1].vertices.len(), 4);
        assert_eq!(
            convert_terrain_runs(&layout, &input[..3]),
            Err(TerrainBatchError::IncompleteQuad)
        );
    }
    #[test]
    fn samples_interpolated_animation_into_padded_atlas_region() {
        let animation = crate::sprites::test_frames(
            0,
            vec![vec![0, 10, 100, 255], vec![200, 30, 200, 127]],
            r#"{"frametime":2,"interpolate":true}"#,
        );
        let layout = pack_atlas(
            &[SpriteSize {
                width: 2,
                height: 2,
            }],
            8,
            1,
            1,
        )
        .unwrap();
        let update = animation_update(&layout, &animation, 1).unwrap();
        assert_eq!((update.width, update.height), (4, 4));
        assert!(
            update
                .pixels
                .chunks_exact(4)
                .all(|pixel| pixel == [100, 20, 150, 191])
        );
        assert_eq!(
            animation_update(&pack_atlas(&[], 8, 1, 1).unwrap(), &animation, 0),
            Err(AtlasError::InvalidSpriteIndex)
        );
    }
    #[test]
    fn camera_origin_subtracts_before_converting_to_float() {
        assert_eq!(
            camera_relative_origin(
                [30_000_000, -64, -30_000_000],
                [30_000_000.25, -63.5, -29_999_999.875]
            ),
            Some([-0.25, -0.5, -0.125])
        );
        assert_eq!(camera_relative_origin([0; 3], [f64::NAN, 0.0, 0.0]), None);
        assert_eq!(camera_relative_origin([0; 3], [f64::MAX, 0.0, 0.0]), None);
    }
}
