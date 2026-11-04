//! Render targets: a framebuffer object with a colour texture and a depth renderbuffer.
//!
//! On a 2.1 context this is the only way to render to a texture, and it has three
//! constraints the wgpu path does not feel:
//!
//! * **The depth format is `GL_DEPTH24_STENCIL8`, not `GL_DEPTH32F_STENCIL8` or
//!   `GL_DEPTH_COMPONENT32F`.** 32-bit float depth is 3.0; the target is 2.1 with
//!   `EXT_packed_depth_stencil`, so 24-bit depth plus 8-bit stencil is what exists. 24 bits
//!   over a 512-block far plane is the precision the world gets, and the plan's fog distances
//!   are built around it (docs/gl21-x1650-plan.ru.md §8).
//! * **Colour targets are RGBA8.** `EXT_color_buffer_float` is absent, so there is no HDR, no
//!   TAA history and no FSR on this path; a pass that asks for a float target gets a
//!   [`crate::error::Error::Missing`] naming `FloatRenderTargets` rather than a black screen.
//! * **No `ARB_texture_storage`**: a resize reallocates, which is why the target's renders are
//!   sized in steps (the menu blur at a quarter of the window, the water mirror at its own
//!   size) rather than the window's exact size.

use glow::HasContext;

use crate::caps::Feature;
use crate::error::{Error, Result};
use crate::gl::texture::{Filter, Texture, Wrap};
use crate::gl::Context;

/// What a target's colour attachment holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorFormat {
    /// RGBA8: the only colour target the target GPU can render to.
    Rgba8,
}

/// What a target's depth attachment holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepthFormat {
    None,
    /// 24-bit depth, 8-bit stencil: `EXT_packed_depth_stencil`.
    Depth24Stencil8,
}

/// A framebuffer with a colour texture, an optional depth renderbuffer, and the checks that
/// make a driver's silence ("framebuffer incomplete") into a sentence.
#[derive(Debug)]
pub struct Target {
    framebuffer: Option<glow::Framebuffer>,
    color: Texture,
    depth: Option<glow::Renderbuffer>,
    depth_size: (u32, u32),
    depth_format: DepthFormat,
    label: &'static str,
}

impl Target {
    /// Creates a target `width` x `height`, with the depth attachment asked for.
    ///
    /// The size is checked against the target GPU's renderbuffer limit first: on the target
    /// that is 4096, and a 4K window would exceed it, so a player at 3840x2160 gets a refusal
    /// at startup rather than a black window.
    pub fn create(
        ctx: &Context,
        width: u32,
        height: u32,
        depth: DepthFormat,
        label: &'static str,
    ) -> Result<Self> {
        let max = ctx.caps().limits.max_renderbuffer_size;
        if width == 0 || height == 0 || width > max || height > max {
            return Err(Error::budget(
                label,
                format!(
                    "{width}x{height}, and this GPU's framebuffer limit is {max} per side: \
                     lower the window size or the resolution scale"
                ),
            ));
        }
        if depth == DepthFormat::Depth24Stencil8 && !ctx.has(Feature::PackedDepthStencil) {
            return Err(Error::Missing(Feature::PackedDepthStencil));
        }

        let color = Texture::create(
            ctx,
            width,
            height,
            crate::gl::texture::Sampling {
                wrap: Wrap::Clamp,
                min: Filter::Linear,
                mag: Filter::Linear,
                anisotropy: 0,
            },
            label,
        )?;
        let gl = ctx.gl();
        let framebuffer = unsafe {
            gl.create_framebuffer()
                .map_err(|e| Error::gl("glGenFramebuffers", e))?
        };
        let depth_buffer = match depth {
            DepthFormat::None => None,
            DepthFormat::Depth24Stencil8 => Some(unsafe {
                gl.create_renderbuffer()
                    .map_err(|e| Error::gl("glGenRenderbuffers", e))?
            }),
        };

        let mut target = Self {
            framebuffer: Some(framebuffer),
            color,
            depth: depth_buffer,
            depth_size: (width, height),
            depth_format: depth,
            label,
        };
        target.attach(ctx)?;
        Ok(target)
    }

    fn attach(&mut self, ctx: &Context) -> Result<()> {
        let gl = ctx.gl();
        let Some(framebuffer) = self.framebuffer else {
            return Err(Error::DeviceLost);
        };
        let Some(color) = self.color.raw() else {
            return Err(Error::DeviceLost);
        };
        let (width, height) = self.color.size();
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            // The colour attachment must be rendered to with level 0 and no mip chain: a
            // mipmapped target that is not mip-complete draws nothing on some drivers.
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(color),
                0,
            );
            if let Some(depth) = self.depth {
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
                // GL_DEPTH24_STENCIL8 comes from EXT_packed_depth_stencil in a 2.1 context;
                // the token is the same number either way.
                gl.renderbuffer_storage(
                    glow::RENDERBUFFER,
                    glow::DEPTH24_STENCIL8,
                    width as i32,
                    height as i32,
                );
                gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_STENCIL_ATTACHMENT,
                    glow::RENDERBUFFER,
                    Some(depth),
                );
                gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            }
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            if status != glow::FRAMEBUFFER_COMPLETE {
                return Err(Error::gl(
                    "glCheckFramebufferStatus",
                    format!("{} is incomplete: {}", self.label, framebuffer_status(status)),
                ));
            }
        }
        Ok(())
    }

    /// Binds the framebuffer and sets the viewport to the whole of it. `None` binds the
    /// window's framebuffer (0), which is how a pass returns to the screen.
    pub fn bind(&self, ctx: &Context) {
        let gl = ctx.gl();
        let (width, height) = self.color.size();
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, self.framebuffer);
            gl.viewport(0, 0, width as i32, height as i32);
        }
    }

    /// Unbinds to the window's default framebuffer.
    pub fn unbind(ctx: &Context) {
        unsafe { ctx.gl().bind_framebuffer(glow::FRAMEBUFFER, None) };
    }

    pub fn texture(&self) -> &Texture {
        &self.color
    }

    pub fn size(&self) -> (u32, u32) {
        self.color.size()
    }

    pub fn has_depth(&self) -> bool {
        self.depth_format != DepthFormat::None
    }

    pub fn label(&self) -> &'static str {
        self.label
    }

    pub fn destroy(&mut self, ctx: &Context) {
        let gl = ctx.gl();
        if let Some(framebuffer) = self.framebuffer.take() {
            unsafe { gl.delete_framebuffer(framebuffer) };
        }
        if let Some(depth) = self.depth.take() {
            unsafe { gl.delete_renderbuffer(depth) };
        }
        self.color.destroy(ctx);
    }
}

/// `GL_SAMPLES` (1.3): the sample count a renderbuffer actually got. A driver that silently
/// halves a 4x request to 2x is visible through it, which is why the settings screen asks.
const SAMPLES: u32 = 0x80A9;

/// `GL_MAX_SAMPLES_EXT`: the most samples this driver's renderbuffers take.
const MAX_SAMPLES_EXT: u32 = 0x8D57;

/// A multisampled colour renderbuffer, resolved by a blit before it is sampled.
///
/// The target has `EXT_framebuffer_object` and `EXT_framebuffer_blit` and, on its own driver,
/// `EXT_framebuffer_multisample`: a 2x or 4x multisampled *renderbuffer* can be resolved into a
/// normal texture. What it cannot do is render into a multisampled *texture*
/// (`ARB_texture_multisample`
/// is 3.2), so this type is deliberately not a [`Texture`]: it is a `Target` whose colour
/// attachment is a renderbuffer and whose `resolve` blits into the caller's texture.
#[derive(Debug)]
pub struct MultisampleTarget {
    framebuffer: Option<glow::Framebuffer>,
    color: Option<glow::Renderbuffer>,
    depth: Option<glow::Renderbuffer>,
    size: (u32, u32),
    samples: u32,
    label: &'static str,
}

impl MultisampleTarget {
    /// `samples` is clamped to what the driver reports (2, 4 or 6 on the target); 0 means the
    /// driver has no multisampled renderbuffers, and the caller must use a plain [`Target`].
    pub fn create(
        ctx: &Context,
        width: u32,
        height: u32,
        samples: u32,
        label: &'static str,
    ) -> Result<Self> {
        if !ctx.has(Feature::FramebufferMultisample) {
            return Err(Error::Missing(Feature::FramebufferMultisample));
        }
        let max = ctx.caps().limits.max_renderbuffer_size;
        if width == 0 || height == 0 || width > max || height > max {
            return Err(Error::budget(
                label,
                format!("{width}x{height}, and this GPU's framebuffer limit is {max} per side"),
            ));
        }
        let gl = ctx.gl();
        let framebuffer = unsafe {
            gl.create_framebuffer()
                .map_err(|e| Error::gl("glGenFramebuffers", e))?
        };
        let color = unsafe {
            gl.create_renderbuffer()
                .map_err(|e| Error::gl("glGenRenderbuffers", e))?
        };
        let depth = unsafe { gl.create_renderbuffer().ok() };
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(color));
            // `glRenderbufferStorageMultisampleEXT` is reached through glow as
            // `renderbuffer_storage_multisample`, the ARB/core entry point the EXT one is
            // identical to.
            gl.renderbuffer_storage_multisample(
                glow::RENDERBUFFER,
                samples as i32,
                // The *unsized* internal format, as everywhere else on this path: `GL_RGBA8`
                // is 3.0, the extension's multisampled storage takes `GL_RGBA` and picks the
                // precision itself.
                glow::RGBA,
                width as i32,
                height as i32,
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::RENDERBUFFER,
                Some(color),
            );
            if let Some(depth) = depth {
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
                gl.renderbuffer_storage_multisample(
                    glow::RENDERBUFFER,
                    samples as i32,
                    glow::DEPTH24_STENCIL8,
                    width as i32,
                    height as i32,
                );
                gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_STENCIL_ATTACHMENT,
                    glow::RENDERBUFFER,
                    Some(depth),
                );
            }
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            if status != glow::FRAMEBUFFER_COMPLETE {
                return Err(Error::gl(
                    "glCheckFramebufferStatus",
                    format!("{label} (multisampled) is incomplete: {}", framebuffer_status(status)),
                ));
            }
        }
        Ok(Self {
            framebuffer: Some(framebuffer),
            color: Some(color),
            depth,
            size: (width, height),
            samples,
            label,
        })
    }

    /// Binds it for drawing, with the viewport covering the whole target.
    pub fn bind(&self, ctx: &Context) {
        let gl = ctx.gl();
        let (width, height) = self.size;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, self.framebuffer);
            gl.viewport(0, 0, width as i32, height as i32);
        }
    }

    /// Resolves into `destination` with `glBlitFramebufferEXT`: the only way a multisampled
    /// image reaches a texture on this driver.
    ///
    /// The destination is a [`Target`], not a [`Texture`]: a blit binds framebuffers on both
    /// sides, and a texture on its own is not one — passing the texture and looking up its
    /// framebuffer would be a second table to keep in step with `Resources`.
    pub fn resolve(&self, ctx: &Context, destination: &Target) -> Result<()> {
        if !ctx.has(Feature::FramebufferBlit) {
            return Err(Error::Missing(Feature::FramebufferBlit));
        }
        if destination.size() != self.size {
            return Err(Error::budget(
                self.label,
                format!(
                    "resolving {}x{} into a {}x{} texture",
                    self.size.0,
                    self.size.1,
                    destination.size().0,
                    destination.size().1
                ),
            ));
        }
        let (Some(read), Some(write)) = (self.framebuffer, destination.framebuffer) else {
            return Err(Error::DeviceLost);
        };
        let gl = ctx.gl();
        unsafe {
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(read));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(write));
            let (width, height) = self.size;
            gl.blit_framebuffer(
                0,
                0,
                width as i32,
                height as i32,
                0,
                0,
                width as i32,
                height as i32,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
        }
        Ok(())
    }

    /// The sample counts worth offering: 2x and 4x where the driver reports they fit.
    ///
    /// 6x is what the chip also supports and what this path does not offer: at 1366x768 it is
    /// most of the frame time on a GPU this size, and the plan's budget has no room for it
    /// (docs/gl21-x1650-plan.ru.md §9). Empty when MSAA is unavailable, which is what the
    /// settings screen shows instead of a switch that does nothing.
    pub fn sample_counts(ctx: &Context) -> Vec<u32> {
        if !ctx.has(Feature::FramebufferMultisample) {
            return Vec::new();
        }
        let gl = ctx.gl();
        let max = unsafe { gl.get_parameter_i32(MAX_SAMPLES_EXT) };
        [2i32, 4]
            .into_iter()
            .filter(|samples| *samples <= max)
            .map(|samples| samples as u32)
            .collect()
    }

    /// What the driver actually gave this renderbuffer: a 4x request that became 2x is worth
    /// a log line rather than a mystery.
    pub fn actual_samples(&self, ctx: &Context) -> u32 {
        let gl = ctx.gl();
        let Some(color) = self.color else { return 0 };
        unsafe {
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(color));
            let samples = gl.get_renderbuffer_parameter_i32(glow::RENDERBUFFER, SAMPLES);
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            samples.max(0) as u32
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn samples(&self) -> u32 {
        self.samples
    }

    pub fn label(&self) -> &'static str {
        self.label
    }

    pub fn destroy(&mut self, ctx: &Context) {
        let gl = ctx.gl();
        if let Some(framebuffer) = self.framebuffer.take() {
            unsafe { gl.delete_framebuffer(framebuffer) };
        }
        if let Some(color) = self.color.take() {
            unsafe { gl.delete_renderbuffer(color) };
        }
        if let Some(depth) = self.depth.take() {
            unsafe { gl.delete_renderbuffer(depth) };
        }
    }
}

/// `GL_FRAMEBUFFER_INCOMPLETE_FORMATS_EXT`, which arrives through `EXT_framebuffer_object`
/// on a 2.1 context and so is not in glow's core-3.0 constants.
const FRAMEBUFFER_INCOMPLETE_FORMATS: u32 = 0x8CDA;

/// A driver's framebuffer status as a sentence.
pub fn framebuffer_status(status: u32) -> &'static str {
    match status {
        glow::FRAMEBUFFER_UNDEFINED => "GL_FRAMEBUFFER_UNDEFINED",
        glow::FRAMEBUFFER_INCOMPLETE_ATTACHMENT => "GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT",
        glow::FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT => {
            "GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT"
        }
        glow::FRAMEBUFFER_INCOMPLETE_DIMENSIONS => "GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS",
        // Not in glow's 2.1 constant set (it is an EXT_framebuffer_object status, not core 3.0).
        FRAMEBUFFER_INCOMPLETE_FORMATS => "GL_FRAMEBUFFER_INCOMPLETE_FORMATS",
        glow::FRAMEBUFFER_INCOMPLETE_DRAW_BUFFER => "GL_FRAMEBUFFER_INCOMPLETE_DRAW_BUFFER",
        glow::FRAMEBUFFER_INCOMPLETE_READ_BUFFER => "GL_FRAMEBUFFER_INCOMPLETE_READ_BUFFER",
        glow::FRAMEBUFFER_UNSUPPORTED => "GL_FRAMEBUFFER_UNSUPPORTED",
        glow::FRAMEBUFFER_INCOMPLETE_MULTISAMPLE => "GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE",
        glow::FRAMEBUFFER_COMPLETE => "GL_FRAMEBUFFER_COMPLETE",
        _ => "an unknown framebuffer status",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::Limits;

    #[test]
    fn the_depth_format_is_the_one_a_21_context_has() {
        // 32-bit float depth is 3.0; packed depth+stencil comes from EXT_packed_depth_stencil.
        assert_eq!(Limits::X1650.max_renderbuffer_size, 4096);
        assert!(matches!(DepthFormat::Depth24Stencil8, DepthFormat::Depth24Stencil8));
    }

    #[test]
    fn a_target_larger_than_the_framebuffer_limit_is_refused() {
        let max = Limits::X1650.max_renderbuffer_size;
        let fits = |w: u32, h: u32| w >= 1 && h >= 1 && w <= max && h <= max;
        assert!(fits(1366, 768));
        assert!(fits(4096, 4096));
        assert!(!fits(4097, 768), "a 4K+ window must be refused, not drawn black");
        assert!(!fits(3840, 2160) || 3840 <= max, "4K fits only because 3840 < 4096");
        assert!(fits(3840, 2160));
        assert!(!fits(5000, 100));
    }

    #[test]
    fn the_offered_sample_counts_are_two_and_four_never_six() {
        // The rule `sample_counts` applies, without a driver: 6x exists on the chip and is
        // deliberately not offered.
        for (max, expected) in [
            (0i32, vec![]),
            (1, vec![]),
            (2, vec![2]),
            (4, vec![2, 4]),
            (6, vec![2, 4]),
            (8, vec![2, 4]),
        ] {
            let offered: Vec<u32> = [2i32, 4]
                .into_iter()
                .filter(|samples| *samples <= max)
                .map(|samples| samples as u32)
                .collect();
            assert_eq!(offered, expected, "max {max}");
            assert!(!offered.contains(&6), "6x is never offered");
        }
    }

    #[test]
    fn every_status_has_a_name() {
        for status in [
            glow::FRAMEBUFFER_UNDEFINED,
            glow::FRAMEBUFFER_INCOMPLETE_ATTACHMENT,
            glow::FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT,
            glow::FRAMEBUFFER_INCOMPLETE_DIMENSIONS,
            FRAMEBUFFER_INCOMPLETE_FORMATS,
            glow::FRAMEBUFFER_INCOMPLETE_DRAW_BUFFER,
            glow::FRAMEBUFFER_INCOMPLETE_READ_BUFFER,
            glow::FRAMEBUFFER_UNSUPPORTED,
            glow::FRAMEBUFFER_INCOMPLETE_MULTISAMPLE,
        ] {
            let name = framebuffer_status(status);
            assert!(name.starts_with("GL_FRAMEBUFFER_"), "{name}");
        }
        assert_eq!(framebuffer_status(glow::FRAMEBUFFER_COMPLETE), "GL_FRAMEBUFFER_COMPLETE");
        assert!(framebuffer_status(0xDEAD_BEEF).contains("unknown"));
    }
}
