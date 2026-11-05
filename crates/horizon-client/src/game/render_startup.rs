//! Experimental startup selection. Native GL2 is NOT wgpu GraphicsApi::OpenGl.
//! Bootstrap-only is deliberately a bounded diagnostic, not a playable mode.
use super::{Options, Result};
#[cfg(any(feature = "legacy-gl", test))]
use horizon_client::renderer_facade::{MenuBackground, MenuFrame};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Renderer {
    #[default]
    Modern,
    NativeGl2,
}
impl Renderer {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "modern" => Ok(Self::Modern),
            "native-gl2" => Ok(Self::NativeGl2),
            _ => Err("expected modern or native-gl2 (wgpu opengl is not native-gl2)".into()),
        }
    }
    pub fn is_native(self) -> bool {
        self == Self::NativeGl2
    }
    #[cfg(any(feature = "legacy-gl", test))]
    pub fn menu_background(self) -> MenuBackground {
        match self {
            Self::Modern => MenuBackground::Panorama,
            Self::NativeGl2 => MenuBackground::Static([0.025, 0.025, 0.025, 1.]),
        }
    }
    #[cfg(any(feature = "legacy-gl", test))]
    pub fn menu<'a>(self, hud: &'a horizon_client::hud::HudBatch) -> MenuFrame<'a> {
        match self.menu_background() {
            MenuBackground::Panorama => super::loading_frame(hud),
            MenuBackground::Static(rgba) => MenuFrame::static_color(hud, rgba),
        }
    }
    pub fn load_settings(self, path: &std::path::Path) -> horizon_client::settings::Settings {
        if self.is_native() {
            horizon_client::settings::Settings::load_preserving_startup_marker(path)
        } else {
            horizon_client::settings::Settings::load(path)
        }
    }
    pub fn validate(self, o: &Options) -> Result<()> {
        if !self.is_native() {
            return if o.native_bootstrap_only {
                Err("--native-bootstrap-only requires --renderer native-gl2".into())
            } else {
                Ok(())
            };
        }
        if !cfg!(feature = "legacy-gl") {
            return Err(
                "native-gl2 requires a client built with --features legacy-gl; no wgpu fallback"
                    .into(),
            );
        }
        if !o.native_bootstrap_only {
            return Err("native Game event loop is not connected yet; only --native-bootstrap-only is available (not playable)".into());
        }
        if o.headless
            || !o.execution_role.is_client()
            || o.world.is_some()
            || o.quick_play.is_some()
            || o.server.is_some()
            || o.control.is_some()
            || o.benchmark.is_some()
            || o.test_terrain
            || o.world_frames.is_some()
            || o.screenshot_after.is_some()
            || o.frames.is_some_and(|n| n != 1)
            || o.fullscreen
            || !o.dev_mods.is_empty()
            || o.session_stdin
            || o.session_file.is_some()
        {
            return Err("native bootstrap is one menu submission only: no world/server/headless/control/benchmark/fullscreen/mods/session/screenshot; --frames may only be 1".into());
        }
        // Each context owns its own surface. Never manufacture a hidden winit
        // window or show a WGL window in response to an offscreen request.
        Self::context_kind(o.offscreen)?;
        Ok(())
    }
    pub fn context_kind(offscreen: bool) -> Result<&'static str> {
        match (std::env::consts::OS, offscreen) {
            ("linux", true) => Ok("osmesa"),
            ("windows", false) => Ok("wgl"),
            ("linux", false) => Err("visible native Linux window is not connected yet; --offscreen is bootstrap validation only".into()),
            ("windows", true) => Err("native WGL owns a visible window; offscreen native bootstrap is not supported on Windows".into()),
            _ => Err("native GL2 startup is not supported on this platform".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(args: &[&str]) -> Result<Options> {
        Options::parse(args.iter().map(|s| s.to_string()))?.ok_or_else(|| "no options".into())
    }
    #[test]
    fn default_and_explicit_modern_keep_the_existing_startup() {
        for args in [&[][..], &["--renderer", "modern"][..]] {
            let o = options(args).unwrap();
            assert_eq!(o.renderer, Renderer::Modern);
            assert!(!o.native_bootstrap_only);
            assert_eq!(o.renderer.menu_background(), MenuBackground::Panorama);
        }
    }
    #[test]
    fn wgpu_opengl_and_unknown_names_are_not_native_aliases() {
        for value in ["opengl", "gl2", "vulkan", "native", ""] {
            assert!(Renderer::parse(value).is_err());
        }
        assert_eq!(Renderer::parse("native-gl2").unwrap(), Renderer::NativeGl2);
    }
    #[test]
    fn bootstrap_cannot_be_requested_on_modern_or_claim_an_event_loop() {
        assert!(options(&["--native-bootstrap-only"]).is_err());
        assert!(options(&["--renderer", "native-gl2"]).is_err());
    }
    #[test]
    fn native_menu_policy_is_selected_before_submission() {
        let hud = horizon_client::hud::HudBatch::new([64; 2]);
        let menu = Renderer::NativeGl2.menu(&hud);
        assert!(matches!(menu.background, MenuBackground::Static(_)));
        assert!(menu.validate().is_ok());
    }
    #[test]
    fn native_context_selection_never_falls_back_to_wgpu_or_wrong_surface_kind() {
        let visible = Renderer::context_kind(false);
        let offscreen = Renderer::context_kind(true);
        match std::env::consts::OS {
            "linux" => {
                assert_eq!(offscreen.unwrap(), "osmesa");
                assert!(visible.is_err());
            }
            "windows" => {
                assert_eq!(visible.unwrap(), "wgl");
                assert!(offscreen.is_err());
            }
            _ => {
                assert!(visible.is_err());
                assert!(offscreen.is_err());
            }
        }
    }
    #[test]
    fn native_settings_read_preserves_modern_hdr_recovery_and_saved_options() {
        use horizon_client::graphics::{FILE, Graphics, HDR_MARKER};
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };
        let dir = std::env::temp_dir().join(format!(
            "horizon-native-startup-settings-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let options = dir.join("options.txt");
        let original = b"gamma:0.75\nresourcePacks:[\"file/kept.zip\"]\n";
        fs::write(&options, original).unwrap();
        let graphics = Graphics {
            hdr: true,
            shader_pack: "kept-shaders".into(),
            ..Default::default()
        };
        graphics.save(&dir);
        fs::write(dir.join(HDR_MARKER), b"pending").unwrap();
        let saved_graphics = fs::read(dir.join(FILE)).unwrap();
        let settings = Renderer::NativeGl2.load_settings(&options);
        assert!(settings.horizon.hdr);
        assert_eq!(settings.horizon.shader_pack, "kept-shaders");
        assert_eq!(settings.resource_packs, ["file/kept.zip"]);
        assert_eq!(settings.gamma, 0.75);
        assert_eq!(fs::read(&options).unwrap(), original);
        assert_eq!(fs::read(dir.join(FILE)).unwrap(), saved_graphics);
        assert_eq!(fs::read(dir.join(HDR_MARKER)).unwrap(), b"pending");
        // Default modern keeps its old recovery behavior; this change must not
        // turn off HDR crash recovery for ordinary players.
        assert!(!Renderer::Modern.load_settings(&options).horizon.hdr);
        assert!(!dir.join(HDR_MARKER).exists());
        fs::remove_dir_all(&dir).unwrap();
    }
    #[cfg(not(feature = "legacy-gl"))]
    #[test]
    fn native_selection_is_a_usage_error_when_not_built_in() {
        let e = options(&[
            "--renderer",
            "native-gl2",
            "--native-bootstrap-only",
            "--offscreen",
        ])
        .err()
        .unwrap()
        .to_string();
        assert!(e.contains("--features legacy-gl"));
    }
    #[cfg(all(feature = "legacy-gl", target_os = "linux"))]
    #[test]
    fn native_bootstrap_accepts_only_its_bounded_menu_scenario() {
        let base = [
            "--renderer",
            "native-gl2",
            "--native-bootstrap-only",
            "--offscreen",
        ];
        assert!(options(&base).is_ok());
        for extra in [
            vec!["--world", "saves/test"],
            vec!["--headless"],
            vec!["--server", "host"],
            vec!["--frames", "2"],
            vec!["--frames", "0"],
            vec!["--control"],
            vec!["--fullscreen"],
            vec!["--test-terrain"],
        ] {
            let args = base.iter().copied().chain(extra).collect::<Vec<_>>();
            assert!(options(&args).is_err(), "{args:?}");
        }
    }
}
