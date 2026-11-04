//! Pure-Rust graphics settings for the `legacy-gl` build (full PR 8).
//!
//! The full `graphics.rs` belongs to the wgpu stack and is cfg-gated out here.
//! This module keeps the same on-disk key (`graphicsApi:legacygl` in
//! `horizon-graphics.txt`), the same environment override and the same UI
//! translation keys, without pulling `wgpu`, `naga`, `pollster` or the world
//! data modules. `options.txt` stays vanilla.

use std::{fs, path::Path};

pub const FILE: &str = "horizon-graphics.txt";
pub const HDR_MARKER: &str = "horizon-hdr-pending";

/// The graphics API the renderer runs on. `LegacyGl` is the GL 2.1/2.0 path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GraphicsApi {
    #[default]
    Auto,
    Vulkan,
    Dx12,
    OpenGl,
    LegacyGl,
}

impl GraphicsApi {
    pub const ALL: [GraphicsApi; 5] = [
        GraphicsApi::Auto,
        GraphicsApi::Vulkan,
        GraphicsApi::Dx12,
        GraphicsApi::OpenGl,
        GraphicsApi::LegacyGl,
    ];

    pub fn choices() -> &'static [GraphicsApi] {
        const WINDOWS: [GraphicsApi; 5] = [
            GraphicsApi::Auto,
            GraphicsApi::Vulkan,
            GraphicsApi::Dx12,
            GraphicsApi::OpenGl,
            GraphicsApi::LegacyGl,
        ];
        const LINUX: [GraphicsApi; 4] = [
            GraphicsApi::Auto,
            GraphicsApi::Vulkan,
            GraphicsApi::OpenGl,
            GraphicsApi::LegacyGl,
        ];
        const MACOS: [GraphicsApi; 1] = [GraphicsApi::Auto];
        if cfg!(windows) {
            &WINDOWS
        } else if cfg!(target_os = "macos") {
            &MACOS
        } else {
            &LINUX
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            GraphicsApi::Auto => "auto",
            GraphicsApi::Vulkan => "vulkan",
            GraphicsApi::Dx12 => "dx12",
            GraphicsApi::OpenGl => "opengl",
            GraphicsApi::LegacyGl => "legacygl",
        }
    }

    /// Translation keys, looked up through `lang::get`; no hardcoded UI text.
    pub fn label(self) -> &'static str {
        match self {
            GraphicsApi::Auto => "options.graphicsApi.auto",
            GraphicsApi::Vulkan => "options.graphicsApi.vulkan",
            GraphicsApi::Dx12 => "options.graphicsApi.dx12",
            GraphicsApi::OpenGl => "options.graphicsApi.opengl",
            GraphicsApi::LegacyGl => "options.graphicsApi.legacygl",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        let name = s.trim().to_ascii_lowercase();
        match name.as_str() {
            "legacygl" | "legacy-gl" | "legacy_gl" | "gl21" | "gl2.1" => {
                Some(GraphicsApi::LegacyGl)
            }
            _ => Self::ALL
                .into_iter()
                .find(|api| api.name().eq_ignore_ascii_case(&name)),
        }
    }

    /// `HORIZON_GRAPHICS_API` (auto, vulkan, dx12, opengl or legacygl).
    pub fn with_env(self) -> Self {
        std::env::var("HORIZON_GRAPHICS_API")
            .ok()
            .and_then(|v| Self::from_name(&v))
            .unwrap_or(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Graphics {
    pub api: GraphicsApi,
    pub texture_budget_mib: u32,
    pub render_distance: u32,
}

impl Default for Graphics {
    fn default() -> Self {
        Self {
            api: GraphicsApi::Auto,
            texture_budget_mib: 128,
            render_distance: 8,
        }
    }
}

impl Graphics {
    pub fn load(dir: &Path) -> Self {
        let text = fs::read_to_string(dir.join(FILE)).unwrap_or_default();
        Self::parse(&text).with_env()
    }

    pub fn with_env(mut self) -> Self {
        self.api = self.api.with_env();
        self
    }

    pub fn parse(text: &str) -> Self {
        let mut g = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "graphicsApi" => g.api = GraphicsApi::from_name(value).unwrap_or_default(),
                "textureBudgetMiB" => {
                    g.texture_budget_mib = value.parse().unwrap_or(g.texture_budget_mib)
                }
                "renderDistance" => g.render_distance = value.parse().unwrap_or(g.render_distance),
                _ => {}
            }
        }
        g
    }

    pub fn serialize(&self) -> String {
        format!(
            "graphicsApi:{}\ntextureBudgetMiB:{}\nrenderDistance:{}\n",
            self.api.name(),
            self.texture_budget_mib,
            self.render_distance
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacygl_round_trips_through_the_settings_file() {
        let g = Graphics::parse("graphicsApi:legacygl\ntextureBudgetMiB:48\nrenderDistance:4\n");
        assert_eq!(g.api, GraphicsApi::LegacyGl);
        assert_eq!(g.texture_budget_mib, 48);
        assert_eq!(g.render_distance, 4);
        assert_eq!(Graphics::parse(&g.serialize()), g);
        assert_eq!(GraphicsApi::from_name("gl21"), Some(GraphicsApi::LegacyGl));
        assert_eq!(GraphicsApi::from_name("metal"), None);
    }

    #[test]
    fn choices_offer_legacygl_on_linux_and_windows() {
        if cfg!(target_os = "macos") {
            assert_eq!(GraphicsApi::choices(), &[GraphicsApi::Auto]);
        } else {
            assert!(GraphicsApi::choices().contains(&GraphicsApi::LegacyGl));
        }
    }
}
