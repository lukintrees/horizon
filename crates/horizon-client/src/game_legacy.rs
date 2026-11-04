//! Full PR 8: the `#[cfg]`-selected legacy game loop.
//!
//! This replaces the wgpu `game.rs` in a `--no-default-features --features
//! legacy-gl` build. It keeps the two renderer creation points, parses the
//! legacy CLI/settings, and uses the real GL2 backend in `legacy_gl.rs`.
//! No wgpu-typed `Renderer` field or method is faked here.

use std::error::Error;
use std::fmt;
use std::path::Path;

use horizon_client_core::lang;
use horizon_client_render::gfx21::facade::{LegacyArgs, LegacyGraphicsApi, LegacySettings};
use horizon_client_render::graphics::{Graphics, GraphicsApi};

use crate::legacy_gl::{RenderFrame, Renderer};

pub mod exit_code {
    pub const USAGE: i32 = 2;
    pub const ERROR: i32 = 1;
}

#[derive(Debug)]
pub struct UsageError(pub String);

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for UsageError {}

#[derive(Clone, Debug)]
struct Options {
    settings: LegacySettings,
    graphics: Graphics,
    unknown: Vec<String>,
}

impl Options {
    #[cfg(feature = "legacy-gl")]
    fn read() -> Result<Option<Self>, Box<dyn Error>> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let parsed = LegacyArgs::parse(&args);
        let mut settings = parsed.settings;
        let graphics = Graphics::load(Path::new("."));
        if graphics.api == GraphicsApi::LegacyGl {
            settings.legacy_gl = true;
            settings.api = LegacyGraphicsApi::LegacyGl;
        }
        for unknown in &parsed.unknown {
            println!(
                "{}",
                lang::tr("horizon.legacyGl.unknownArgument", &[unknown])
            );
        }
        if settings.help || settings.launch_api {
            println!("{}", lang::get("horizon.legacyGl.help"));
            println!("{}", lang::get("horizon.legacyGl.launchApi"));
            return Ok(None);
        }
        Ok(Some(Self {
            settings,
            graphics,
            unknown: parsed.unknown,
        }))
    }
}

/// The wgpu `build_renderer` counterpart: one real GL2 window/context.
#[cfg(feature = "legacy-gl")]
fn build_renderer(options: &Options) -> Result<Renderer, Box<dyn Error>> {
    Renderer::new(&options.settings)
}

/// The wgpu `loading_renderer` counterpart. Kept as a separate cfg point;
/// the loading screen uses the same GL2 context in this minimal game loop.
#[cfg(feature = "legacy-gl")]
fn loading_renderer(options: &Options) -> Result<Renderer, Box<dyn Error>> {
    Renderer::new(&options.settings)
}

#[cfg(feature = "legacy-gl")]
fn loading_frame() -> RenderFrame {
    RenderFrame::empty()
}

struct Game {
    renderer: Option<Renderer>,
    options: Options,
}

impl Game {
    #[cfg(feature = "legacy-gl")]
    fn new(options: Options) -> Result<Self, Box<dyn Error>> {
        let renderer = build_renderer(&options)?;
        Ok(Self {
            renderer: Some(renderer),
            options,
        })
    }

    #[cfg(feature = "legacy-gl")]
    fn run(&mut self) -> Result<(), Box<dyn Error>> {
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| UsageError(lang::get("horizon.legacyGl.unavailable")))?;
        renderer.run(self.options.settings.frames)?;
        Ok(())
    }
}

#[cfg(feature = "legacy-gl")]
pub fn run_with() -> Result<(), Box<dyn Error>> {
    let Some(options) = Options::read()? else {
        return Ok(());
    };
    let mut game = Game::new(options)?;
    game.run()
}
