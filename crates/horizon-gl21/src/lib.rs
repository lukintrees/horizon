//! Horizon on OpenGL 2.1, on a real driver: the context, the window and the calls.
//!
//! `docs/opengl21-plan.md` is the plan, and `horizon-client-render::gl21` is its arithmetic — the
//! atlas's layout, the frame's draw order, the shader tables — written so that it can be tested on a
//! machine with no OpenGL at all, which is CI and the machine this was written on. This crate is the
//! other half, and it is the part that needs a driver: an EGL context that is OpenGL 2.1, a window
//! surface for it, a frame texture, and a `gl21::Device` implemented on `glow`.
//!
//! It is a crate of its own because the render crate is `#![forbid(unsafe_code)]` and a GL context
//! is nothing but unsafe calls: `horizon-hdr` exists for the same reason, for the Vulkan calls wgpu
//! does not make.
//!
//! The pieces:
//!
//! - [`egl`] — the display, the config, the context, the window surface and the frame texture. Every
//!   failure is a sentence naming what the driver answered and what a player can do.
//! - [`driver`] — `gl21::Device` on `glow`: three programs compiled and linked once, the world's one
//!   vertex buffer, the HUD's, the present quad's, and the calls in the order `Renderer::draw` asks
//!   for them.
//! - [`resources`] — what is uploaded and what makes an upload wrong: the levels of a texture's mip
//!   chain, which shaders a program compiles, which attributes it is bound with.
//! - [`quad`] — the present pass's four vertices, which are the one piece of geometry that is not
//!   the mesher's.
//!
//! None of it changes what Horizon draws by default. A run opens a GL 2.1 context only where a
//! caller asks for one (`HORIZON_GL_PROFILE=x1650`, or a caller that names this crate's device), and
//! the wgpu renderer keeps the window everywhere else.
//!
//! ```no_run
//! # use horizon_gl21::{Driver, Renderer};
//! # fn draw(window: raw_window_handle::RawWindowHandle, display: raw_window_handle::RawDisplayHandle)
//! #     -> Result<(), String> {
//! // Three shaders, the world's buffer and the HUD's, on a window winit made.
//! let mut driver = Driver::new(window, display, 1280, 720, 65_536, 16_384, None)?;
//! let mut renderer = Renderer::new(65_536);
//! // `renderer.draw(&mut driver, view, &sections, sky, hud, true)` once a frame, and the device
//! // presents: `end_frame` swaps when the frame asked for a present pass.
//! # Ok(())
//! # }
//! ```

pub mod driver;
#[cfg(not(target_arch = "wasm32"))]
pub mod egl;
pub mod quad;
pub mod resources;

pub use driver::Driver;
pub use resources::{Level, TextureId, Upload};

/// The render crate's own `Renderer`: named here so that a caller of this crate needs one import.
pub use horizon_client_render::gl21::Renderer;

/// The renderer's names, for a caller that draws: `Renderer::draw`'s view, sections, sky and HUD.
pub use horizon_client_render::gl21::{
    Atlas, Device, FrameUniforms, GlHudVertex, GlVertex, HudDraws, HudFrame, Pass, Program,
    Section, Sky, TerrainUniforms, Texture, View, Viewport,
};
