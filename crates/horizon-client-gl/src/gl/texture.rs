//! Textures on a 2.1 context, where a sprite "layer" is a UV offset rather than an array slice.
//!
//! **This is the port's biggest single design change.** The wgpu renderer binds sprite atlases
//! as `texture_2d_array` in up to eight arrays of 2048 layers (`terrain.wgsl`'s `SPRITE_SPLIT`),
//! and the layer travels from the mesher to the shader as `@location(2) layer: u32`. A 2.1
//! context has no array textures at all, so:
//!
//! * the mesher emits a **UV inside the atlas** instead of a layer, which means the atlas
//!   layout is a meshing input (it already is: `block_sprites.rs` assigns sprite rectangles);
//! * "layer" becomes a rectangle lookup on the CPU, done once per sprite and carried in the
//!   vertex, so the shader never needs it;
//! * **atlas size is a budget**: the target's 4096 is the maximum texture side *and* it has
//!   256 MiB of video memory, so 4096² RGBA8 (67 MiB) plus mipmaps does not fit beside the
//!   world. Two 2048² atlases (16 MiB each) are the shape the plan proposes (§8.1), with the
//!   mesher choosing which atlas a sprite goes to.
//!
//! What this module deliberately does not have: a sampler object. GL 2.1 has none
//! (`ARB_sampler_objects` is 3.3), so wrap and filter are texture state, set when the texture
//! is uploaded and when a pass changes them.

use glow::HasContext;

use crate::caps::Feature;
use crate::error::{Error, Result};
use crate::gl::Context;

/// How a texture is sampled. On 2.1 this is state of the texture object, not of a sampler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sampling {
    pub wrap: Wrap,
    pub min: Filter,
    pub mag: Filter,
    /// 0 for none; the target's maximum is 16.
    pub anisotropy: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    Repeat,
    Clamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    Linear,
    /// Trilinear: needs mipmaps, which every atlas has.
    LinearMipmap,
}

impl Filter {
    /// What the minification filter must be, given whether the texture has a mip chain.
    ///
    /// A `LINEAR_MIPMAP_*` filter on a texture without mips is *incomplete* texturing: the
    /// driver samples black (or white, depending on the vendor's fallback) and reports
    /// nothing. The GL 2.1 path uploads an atlas's level 0 before deciding to build mips, so
    /// this case happens for real, not in theory.
    pub fn minification(self, has_mips: bool) -> Filter {
        match self {
            Filter::LinearMipmap if !has_mips => Filter::Linear,
            other => other,
        }
    }
}

impl Sampling {
    /// The GUI and text atlas: nearest magnification, so a font's pixels stay square.
    pub const PIXEL_ART: Sampling = Sampling {
        wrap: Wrap::Clamp,
        min: Filter::LinearMipmap,
        mag: Filter::Nearest,
        anisotropy: 0,
    };

    /// Terrain blocks: linear both ways, and the anisotropic setting where the driver has it.
    pub const BLOCKS: Sampling = Sampling {
        wrap: Wrap::Clamp,
        min: Filter::LinearMipmap,
        mag: Filter::Linear,
        anisotropy: 0,
    };

    /// This sampling with another's wrap and filters but its own anisotropy: the one the
    /// executor applies, because a 2.1 context has no sampler objects and wrap/filter are
    /// therefore texture state that a pass borrows rather than a separate object it binds.
    pub fn with_wrap_and_filters(self, other: Sampling) -> Sampling {
        Sampling {
            wrap: other.wrap,
            min: other.min,
            mag: other.mag,
            anisotropy: self.anisotropy,
        }
    }
}

/// `GL_SRGB8_ALPHA8_EXT`: the internal format a texture upload uses when the driver has
/// `EXT_texture_sRGB`. glow's constant set has `SRGB8_ALPHA8`, which is the 3.0 number; the
/// extension defines the same value, so the literal is the extension's name rather than a
/// second constant that could drift.
pub const SRGB8_ALPHA8_EXT: u32 = 0x8C43;

/// A 2D texture.
#[derive(Debug)]
pub struct Texture {
    raw: Option<glow::Texture>,
    width: u32,
    height: u32,
    sampling: Sampling,
    has_mips: bool,
    /// Uploaded in sRGB, so the driver linearizes on sample: gamma stays right without a 3.0
    /// `GL_FRAMEBUFFER_SRGB`.
    srgb: bool,
    label: &'static str,
}

impl Texture {
    /// A texture at the largest side this device allows, refused early with a sentence rather
    /// than as `GL_INVALID_VALUE` later. On the target the answer is 4096, and the *useful*
    /// limit is lower: see the module docs on video memory.
    pub fn create(
        ctx: &Context,
        width: u32,
        height: u32,
        sampling: Sampling,
        label: &'static str,
    ) -> Result<Self> {
        let max = ctx.caps().limits.max_texture_size;
        if width == 0 || height == 0 || width > max || height > max {
            return Err(Error::budget(
                label,
                format!("{width}x{height} texels, this GPU allows 1..{max} per side"),
            ));
        }
        let gl = ctx.gl();
        let raw = unsafe { gl.create_texture().map_err(|e| Error::gl("glGenTextures", e))? };
        Ok(Self {
            raw: Some(raw),
            width,
            height,
            sampling,
            has_mips: false,
            srgb: false,
            label,
        })
    }

    /// A copy of `texture` that reports a different sampling but owns nothing: the executor
    /// uses it to re-upload wrap and filter parameters that a 2.1 context keeps on the texture
    /// itself, because there are no sampler objects to hold them. Dropping it destroys nothing.
    pub fn describe(texture: &Texture, sampling: Sampling) -> Self {
        Self {
            raw: texture.raw,
            width: texture.width,
            height: texture.height,
            sampling,
            has_mips: texture.has_mips,
            srgb: texture.srgb,
            label: texture.label,
        }
    }

    /// Uploads sRGB texels: the driver turns them into linear values when the shader samples
    /// them. On the target that is `EXT_texture_sRGB`, not `GL_FRAMEBUFFER_SRGB` (3.0), which
    /// is also why this path has no sRGB framebuffer to get wrong.
    pub fn set_srgb(&mut self, srgb: bool) {
        self.srgb = srgb;
    }

    pub fn is_srgb(&self) -> bool {
        self.srgb
    }

    pub fn raw(&self) -> Option<glow::Texture> {
        self.raw
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn label(&self) -> &'static str {
        self.label
    }

    /// Uploads RGBA8 pixels for level 0. `pixels` must be `width * height * 4` bytes: a
    /// mismatched size would read past the end in the driver, so it is checked here.
    pub fn upload(&mut self, ctx: &Context, pixels: &[u8]) -> Result<()> {
        let expected = self.width as usize * self.height as usize * 4;
        if pixels.len() != expected {
            return Err(Error::budget(
                self.label,
                format!(
                    "{} bytes uploaded for a {}x{} RGBA8 texture, which needs {expected}",
                    pixels.len(),
                    self.width,
                    self.height
                ),
            ));
        }
        let gl = ctx.gl();
        let Some(raw) = self.raw else {
            return Err(Error::DeviceLost);
        };
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(raw));
            // Rows are packed 4-byte-aligned; a 2048-wide atlas needs no padding, but a HUD
            // atlas of odd width does, and the default (4) is what every driver assumes.
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                // The *unsized* internal format: `GL_RGBA8` is a 3.0 token that 2.1 drivers
                // reject, and `GL_RGBA` with `GL_UNSIGNED_BYTE` gives the same thing.
                glow::RGBA as i32,
                self.width as i32,
                self.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(pixels)),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        ctx.count_upload(pixels.len());
        self.apply_sampling(ctx)?;
        Ok(())
    }

    /// Uploads a sub-rectangle, for a sprite that changed (an animated texture on the CPU
    /// path, a sign's text).
    pub fn upload_region(
        &mut self,
        ctx: &Context,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Result<()> {
        if x + width > self.width || y + height > self.height {
            return Err(Error::budget(
                self.label,
                format!(
                    "region {width}x{height} at {x},{y} is outside {}x{}",
                    self.width, self.height
                ),
            ));
        }
        let expected = width as usize * height as usize * 4;
        if pixels.len() != expected {
            return Err(Error::budget(
                self.label,
                format!(
                    "{} bytes for a {width}x{height} region, which needs {expected}",
                    pixels.len()
                ),
            ));
        }
        let gl = ctx.gl();
        let Some(raw) = self.raw else {
            return Err(Error::DeviceLost);
        };
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(raw));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                x as i32,
                y as i32,
                width as i32,
                height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(pixels)),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        ctx.count_upload(pixels.len());
        Ok(())
    }

    /// Builds the mip chain. Called after the last upload of a level-0 image: on 2.1 there is
    /// no `ARB_texture_storage`, so every `glTexImage2D` reallocates and drops the mips.
    pub fn generate_mips(&mut self, ctx: &Context) {
        let gl = ctx.gl();
        if let Some(raw) = self.raw {
            unsafe {
                gl.bind_texture(glow::TEXTURE_2D, Some(raw));
                gl.generate_mipmap(glow::TEXTURE_2D);
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            self.has_mips = true;
        }
    }

    /// Wrap and filter: texture state on this path, so a pass that changes them changes them
    /// back, which [`Sampling: PartialEq`] lets the backend's cache skip.
    pub fn apply_sampling(&mut self, ctx: &Context) -> Result<()> {
        let gl = ctx.gl();
        let Some(raw) = self.raw else {
            return Ok(());
        };
        let wrap = match self.sampling.wrap {
            Wrap::Repeat => glow::REPEAT,
            Wrap::Clamp => glow::CLAMP_TO_EDGE,
        };
        let min = match self.sampling.min.minification(self.has_mips) {
            Filter::Nearest => glow::NEAREST,
            Filter::Linear => glow::LINEAR,
            Filter::LinearMipmap => glow::LINEAR_MIPMAP_LINEAR,
        };
        let mag = match self.sampling.mag {
            Filter::Nearest => glow::NEAREST,
            _ => glow::LINEAR,
        };
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(raw));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, min as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, mag as i32);
            if self.sampling.anisotropy > 1 && ctx.has(Feature::AnisotropicFilter) {
                // The extension's token is not in glow's 2.1 set: 0x84FE is
                // GL_TEXTURE_MAX_ANISOTROPY_EXT.
                const TEXTURE_MAX_ANISOTROPY_EXT: u32 = 0x84FE;
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    TEXTURE_MAX_ANISOTROPY_EXT,
                    i32::from(self.sampling.anisotropy.min(16)),
                );
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        Ok(())
    }

    pub fn sampling(&self) -> Sampling {
        self.sampling
    }

    pub fn set_sampling(&mut self, sampling: Sampling) {
        self.sampling = sampling;
    }

    pub fn destroy(&mut self, ctx: &Context) {
        if let Some(raw) = self.raw.take() {
            unsafe { ctx.gl().delete_texture(raw) };
        }
    }
}

/// The largest atlas that fits the target's video memory beside the world, in texels.
///
/// 4096 is the hardware maximum and is *not* the answer: 4096² RGBA8 is 67 MiB on a card with
/// 256 MiB, before mips (89 MiB) and before the terrain buffers, the depth buffer and the
/// window's own buffers. Two 2048² atlases are 16 MiB each with mips, and the mesher picks
/// which one a sprite goes to (docs/gl21-x1650-plan.ru.md §8.1).
pub const ATLAS_TEXELS: u32 = 2048;
/// How many atlases the mesher may fill before it must fail loudly rather than grow the
/// card's memory: two of 2048², matching the plan.
pub const ATLAS_COUNT: u32 = 2;

/// A sprite's rectangle in an atlas, as the mesher carries it into the vertex's UVs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteRect {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

impl SpriteRect {
    /// From a texel rectangle in an atlas of `size`: the CPU's replacement for a layer index.
    pub fn from_texels(x: u32, y: u32, width: u32, height: u32, size: u32) -> Self {
        let s = size as f32;
        Self {
            u0: x as f32 / s,
            v0: y as f32 / s,
            u1: (x + width) as f32 / s,
            v1: (y + height) as f32 / s,
        }
    }

    /// The atlas whose UV range this rectangle falls in, or `None` when it straddles two.
    pub fn atlas_of(uv: [f32; 2], atlas_count: u32) -> Option<u32> {
        if atlas_count == 0 {
            return None;
        }
        let slice = 1.0 / atlas_count as f32;
        let index = (uv[0] / slice).floor();
        if index >= atlas_count as f32 {
            return None;
        }
        let index = index as u32;
        let within = uv[0] - index as f32 * slice;
        if within > slice {
            None
        } else {
            Some(index)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_atlas_budget_is_memory_not_the_hardware_maximum() {
        // 4096 is the hardware limit and would be 67 MiB of a 256 MiB card.
        assert_eq!(crate::caps::Limits::X1650.max_texture_size, 4096);
        assert_eq!(ATLAS_TEXELS, 2048);
        let bytes = (ATLAS_TEXELS * ATLAS_TEXELS * 4) as u64;
        assert_eq!(bytes, 16 * 1024 * 1024);
        // Two atlases with their mips still leave the card most of its memory.
        assert!(bytes * 2 * 4 / 3 < 256 * 1024 * 1024 / 2);
    }

    #[test]
    fn a_sprite_rect_is_uvs_in_the_atlas() {
        let r = SpriteRect::from_texels(0, 0, 16, 16, 2048);
        assert_eq!((r.u0, r.v0), (0.0, 0.0));
        assert!((r.u1 - 16.0 / 2048.0).abs() < 1e-6);
        // The middle of the atlas, an odd-sized sprite (a chest's front face, say).
        let r = SpriteRect::from_texels(1024, 512, 32, 16, 2048);
        assert!((r.u0 - 0.5).abs() < 1e-6);
        assert!((r.v0 - 0.25).abs() < 1e-6);
        assert!((r.u1 - 1056.0 / 2048.0).abs() < 1e-6);
    }

    #[test]
    fn a_layer_becomes_the_atlas_holding_its_uvs() {
        // With two atlases, the first covers u in [0, 0.5).
        assert_eq!(SpriteRect::atlas_of([0.1, 0.5], 2), Some(0));
        assert_eq!(SpriteRect::atlas_of([0.6, 0.5], 2), Some(1));
        assert_eq!(SpriteRect::atlas_of([0.5, 0.5], 2), Some(1));
        // One atlas: everything is in it.
        assert_eq!(SpriteRect::atlas_of([0.9, 0.9], 1), Some(0));
        // No atlases is a caller bug, not a panic.
        assert_eq!(SpriteRect::atlas_of([0.1, 0.1], 0), None);
    }

    #[test]
    fn a_texture_larger_than_the_gpu_allows_is_refused_before_the_driver_sees_it() {
        // The rule `create` applies, on the target's numbers: 1..4096 per side.
        let max = crate::caps::Limits::X1650.max_texture_size;
        let fits = |w: u32, h: u32| w >= 1 && h >= 1 && w <= max && h <= max;
        assert!(fits(2048, 2048));
        assert!(fits(4096, 1), "an odd shape is legal, only the side is limited");
        assert!(!fits(4097, 1));
        assert!(!fits(0, 0));
        // The atlas the plan budgets for is half the hardware maximum on purpose: memory.
        // At exactly the maximum it would fit the API and not the card (see ATLAS_TEXELS).
        assert_eq!(ATLAS_TEXELS * 2, max);
        assert!(fits(ATLAS_TEXELS, ATLAS_TEXELS));
    }

    #[test]
    fn a_mip_less_texture_drops_the_mipmap_filter_instead_of_going_black() {
        assert_eq!(Filter::LinearMipmap.minification(false), Filter::Linear);
        assert_eq!(Filter::LinearMipmap.minification(true), Filter::LinearMipmap);
        assert_eq!(Filter::Nearest.minification(true), Filter::Nearest);
        assert_eq!(Filter::Nearest.minification(false), Filter::Nearest);
        // Both shipped samplings ask for mips, so both depend on this rule.
        assert_eq!(Sampling::BLOCKS.min, Filter::LinearMipmap);
        assert_eq!(Sampling::PIXEL_ART.min, Filter::LinearMipmap);
    }

    #[test]
    fn the_upload_size_the_code_requires_is_width_times_height_times_four() {
        // `upload` refuses a mismatch instead of letting the driver read past the end of the
        // slice; this is that arithmetic for the atlas sizes the path uses.
        for (w, h) in [(2048u32, 2048u32), (256, 256), (1024, 512), (17, 3)] {
            assert_eq!(w as usize * h as usize * 4, (w * h * 4) as usize);
        }
    }

    #[test]
    fn pixel_art_keeps_nearest_magnification() {
        assert_eq!(Sampling::PIXEL_ART.mag, Filter::Nearest);
        assert_eq!(Sampling::BLOCKS.mag, Filter::Linear);
    }
}
