"""Contract tests for the full PR 8 `legacy-gl` game integration.

These are static/source checks: the sandbox cannot resolve the full crates.io
dependency graph, and the real GL frames are exercised by the own-branch CI
workflow under Xvfb. The tests make sure the default wgpu build, the
standalone GL2 build, UI/localization and the real renderer stay wired.
"""

import json
import re
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RENDER_LIB = ROOT / "crates/horizon-client-render/src/lib.rs"
CLIENT_LIB = ROOT / "crates/horizon-client/src/client_lib.rs"
CLIENT_ROOT = ROOT / "crates/horizon-client/src/lib.rs"
MAIN_RS = ROOT / "crates/horizon-client/src/main.rs"
GRAPHICS_RS = ROOT / "crates/horizon-client-render/src/graphics.rs"
GRAPHICS_LEGACY_RS = ROOT / "crates/horizon-client-render/src/graphics_legacy.rs"
OPTIONS_UI = ROOT / "crates/horizon-client-render/src/options_ui.rs"
ENTRY_RS = ROOT / "crates/horizon-client/src/entry.rs"
ENTRY_LEGACY_RS = ROOT / "crates/horizon-client/src/entry_legacy.rs"
GAME_LEGACY_RS = ROOT / "crates/horizon-client/src/game_legacy.rs"
LEGACY_GL_RS = ROOT / "crates/horizon-client/src/legacy_gl.rs"
CARGO_RENDER = ROOT / "crates/horizon-client-render/Cargo.toml"
CARGO_CLIENT_LIB = ROOT / "crates/horizon-client-lib/Cargo.toml"
CARGO_CLIENT = ROOT / "crates/horizon-client/Cargo.toml"
WORKFLOW = ROOT / ".github/workflows/arena-legacy-gl.yml"
CONTRACT = ROOT / "crates/horizon-client-render/src/gfx21/CONTRACT.md"
DESIGN = ROOT / "docs/design/legacy-gl21.md"
README = ROOT / "README.md"
NATIVE = ROOT / "docs/native-client.md"
NOTICES = ROOT / "THIRD_PARTY_NOTICES.md"
LANG_DIR = ROOT / "crates/horizon-client/assets/horizon/lang"

UI_KEYS = (
    "options.graphicsApi.auto",
    "options.graphicsApi.vulkan",
    "options.graphicsApi.dx12",
    "options.graphicsApi.opengl",
    "options.graphicsApi.legacygl",
    "options.graphicsApi.default",
    "options.graphicsApi.legacygl.desc",
    "options.graphicsApi.tooltip",
    "options.graphicsApi.tooltip.vulkan",
    "options.graphicsApi.tooltip.legacygl",
    "options.needsRestart",
    "options.legacy.unavailable",
    "horizon.legacyGl.windowTitle",
    "horizon.legacyGl.startup",
    "horizon.legacyGl.startupError",
    "horizon.legacyGl.frameDrawn",
    "horizon.legacyGl.help",
    "horizon.legacyGl.launchApi",
    "horizon.legacyGl.unavailableMethods",
    "horizon.legacyGl.unknownArgument",
    "horizon.legacyGl.contextError",
    "horizon.legacyGl.glError",
    "horizon.legacyGl.unavailable",
    "horizon.legacyGl.game.network",
    "horizon.legacyGl.game.networkError",
    "horizon.legacyGl.game.chat",
    "horizon.legacyGl.game.offline",
    "horizon.legacyGl.game.integrated",
    "horizon.legacyGl.game.remote",
    "horizon.legacyGl.game.integratedStarted",
    "horizon.legacyGl.game.remoteStarted",
    "horizon.legacyGl.game.lanStarted",
    "horizon.legacyGl.game.chatPrompt",
    "horizon.legacyGl.game.chatOpen",
    "horizon.legacyGl.game.chatLine",
    "horizon.legacyGl.game.playerList",
    "horizon.legacyGl.game.containerTitle",
    "horizon.legacyGl.game.containerSlot",
    "horizon.legacyGl.game.containerSlotLabel",
    "horizon.legacyGl.game.containerClick",
    "horizon.legacyGl.game.serverName",
    "horizon.legacyGl.game.playerJoined",
    "horizon.legacyGl.game.playerLeft",
    "horizon.legacyGl.game.commandHelp",
    "horizon.legacyGl.game.commandUnknown",
    "horizon.legacyGl.game.commandSeed",
    "horizon.legacyGl.game.commandTime",
    "horizon.legacyGl.game.commandList",
    "horizon.legacyGl.game.commandSpawn",
    "horizon.legacyGl.game.commandHeal",
    "horizon.legacyGl.game.commandGive",
    "horizon.legacyGl.game.commandContainer",
    "horizon.legacyGl.game.commandSaved",
    "horizon.legacyGl.game.commandSaveError",
    "horizon.legacyGl.game.commandPosition",
    "horizon.legacyGl.game.commandTp",
    "horizon.legacyGl.game.commandUsage",
    "horizon.legacyGl.game.commandDay",
    "horizon.legacyGl.game.commandNight",
    "horizon.legacyGl.game.commandReset",
    "horizon.legacyGl.game.commandLoaded",
)


def cargo(path):
    return tomllib.loads(path.read_text(encoding="utf-8"))


class FeatureSplitTests(unittest.TestCase):
    def test_all_three_crates_split_modern_and_legacy_features(self):
        render = cargo(CARGO_RENDER)
        client_lib = cargo(CARGO_CLIENT_LIB)
        client = cargo(CARGO_CLIENT)
        for data in (render, client_lib, client):
            self.assertEqual(data["features"]["default"][-1], "modern-gl")
            self.assertIn("modern-gl", data["features"])
            self.assertIn("legacy-gl", data["features"])
            self.assertNotIn("modern-gl", data["features"]["legacy-gl"])
        for dep in ("wgpu", "naga", "pollster", "horizon-hdr", "horizon-shader-api", "horizon-shaders"):
            self.assertTrue(render["dependencies"].get(dep, {}).get("optional"), dep)
            self.assertIn(f"dep:{dep}", render["features"]["modern-gl"])
        for dep in ("glow", "glutin", "glutin-winit", "raw-window-handle"):
            self.assertTrue(render["dependencies"][dep].get("optional"), dep)
            self.assertTrue(client["dependencies"][dep].get("optional"), dep)
            self.assertIn(f"dep:{dep}", client["features"]["legacy-gl"])
        for data in (client_lib, client):
            self.assertFalse(data["dependencies"]["horizon-client-render"]["default-features"])
            self.assertNotIn(
                "index",
                data["dependencies"]["horizon-mods"].get("features", []),
                "horizon-mods/index must not be enabled outside modern-gl",
            )
            self.assertIn("horizon-mods/index", data["features"]["modern-gl"])
            self.assertNotIn("horizon-mods/index", data["features"]["legacy-gl"])
        self.assertIn("horizon-client-render/legacy-gl", client["features"]["legacy-gl"])
        self.assertIn("horizon-client-lib/legacy-gl", client["features"]["legacy-gl"])
        self.assertIn("horizon-client-render/legacy-gl", client_lib["features"]["legacy-gl"])

    def test_render_crate_root_cfg_gates_all_modern_modules(self):
        source = RENDER_LIB.read_text(encoding="utf-8")
        self.assertIn("pub mod gfx21;", source)
        self.assertIn('#[cfg(feature = "modern-gl")]\npub mod gfx;', source)
        self.assertIn('#[cfg(feature = "modern-gl")]\npub mod graphics;', source)
        self.assertIn('#[cfg(all(feature = "legacy-gl", not(feature = "modern-gl")))]', source)
        self.assertIn('#[path = "graphics_legacy.rs"]', source)
        self.assertIn('#[cfg(feature = "legacy-gl")]\npub use gfx21::Frame;', source)
        self.assertIn('#[cfg(feature = "legacy-gl")]\npub type Renderer = gfx21::Renderer;', source)

    def test_client_library_and_binary_select_legacy_modules(self):
        client_lib = CLIENT_LIB.read_text(encoding="utf-8")
        self.assertIn('#[cfg(feature = "modern-gl")]\npub use horizon_client_render::{', client_lib)
        self.assertIn('#[cfg(all(feature = "legacy-gl", not(feature = "modern-gl")))]', client_lib)
        self.assertIn("pub use horizon_client_render::{gfx21, graphics};", client_lib)
        self.assertIn('#[cfg(feature = "modern-gl")]\npub mod attack_strength;', client_lib)

        root = CLIENT_ROOT.read_text(encoding="utf-8")
        self.assertIn('#![cfg_attr(not(feature = "legacy-gl"), forbid(unsafe_code))]', root)
        self.assertIn('#[path = "entry_legacy.rs"]', root)
        self.assertIn('#[path = "game_legacy.rs"]', root)
        self.assertIn("pub mod legacy_gl;", root)
        self.assertIn('#[cfg(feature = "modern-gl")]\npub mod game;', root)
        self.assertIn('#[cfg(feature = "modern-gl")]\npub mod mod_api;', root)
        self.assertIn('#![cfg_attr(not(feature = "legacy-gl"), forbid(unsafe_code))]', MAIN_RS.read_text(encoding="utf-8"))


class IntegrationSourceTests(unittest.TestCase):
    def test_graphics_api_has_legacygl_and_translation_labels(self):
        for path in (GRAPHICS_RS, GRAPHICS_LEGACY_RS):
            source = path.read_text(encoding="utf-8")
            self.assertIn("LegacyGl,", source)
            self.assertIn("pub const ALL: [GraphicsApi; 5]", source)
            self.assertIn('"legacygl"', source)
            self.assertIn('"options.graphicsApi.legacygl"', source)
            self.assertIn("HORIZON_GRAPHICS_API", source)
            self.assertIn('"gl21"', source)
        ui = OPTIONS_UI.read_text(encoding="utf-8")
        self.assertIn("GraphicsApi::LegacyGl", ui)
        self.assertIn("[Auto, OpenGl, Vulkan, LegacyGl]", ui)
        self.assertIn("options.graphicsApi.tooltip.legacygl", ui)

    def test_legacy_settings_module_is_pure_rust(self):
        source = GRAPHICS_LEGACY_RS.read_text(encoding="utf-8")
        for forbidden in ("wgpu::", "naga::", "pollster::", "unsafe"):
            self.assertNotIn(forbidden, source)
        self.assertIn('pub const FILE: &str = "horizon-graphics.txt"', source)
        self.assertIn("GraphicsApi::from_name", source)

    def test_game_legacy_is_the_cfg_split_of_both_renderer_points(self):
        source = GAME_LEGACY_RS.read_text(encoding="utf-8")
        for marker in (
            '#[cfg(feature = "legacy-gl")]',
            "fn build_renderer",
            "fn loading_renderer",
            "fn loading_frame",
            "pub fn run_with",
            "LegacyArgs::parse",
            "Graphics::load",
            "Renderer::new",
        ):
            self.assertIn(marker, source)
        for forbidden in ("gfx::", "wgpu::", "naga::", "pollster::"):
            self.assertNotIn(forbidden, source)
        self.assertIn("horizon.legacyGl", GAME_LEGACY_RS.read_text(encoding="utf-8"))

    def test_real_gl_backend_links_and_draws_all_passes(self):
        source = LEGACY_GL_RS.read_text(encoding="utf-8")
        for marker in (
            "glow::Context::from_loader_function",
            "use glutin_winit::{DisplayBuilder, GlWindow};",
            "winit::application::ApplicationHandler",
            "FullFramePlan::reference",
            "all_programs(dialect)",
            "validate_all()",
            "MAX_DRAWS",
            "UNAVAILABLE_METHODS",
            "Pass::Terrain",
            "Pass::Entity",
            "Pass::Sky",
            "Pass::Hud",
            "Pass::Line",
            "gl.get_error()",
            ".swap_buffers(context)",
            "lang::get",
            "lang::tr",
        ):
            self.assertIn(marker, source)
        for forbidden in ("wgpu::", "naga::", "pollster::", "alpha_func", "gl.alpha_func"):
            self.assertNotIn(forbidden, source)

    def test_entry_points_resolve_language_and_diagnose_unavailable_backend(self):
        modern = ENTRY_RS.read_text(encoding="utf-8")
        self.assertIn("lang::set(lang::Language::load(&[], lang::DEFAULT));", modern)
        self.assertIn("fn legacy_requested", modern)
        self.assertIn('"--legacy-gl"', modern)
        self.assertIn("options.legacy.unavailable", modern)
        legacy = ENTRY_LEGACY_RS.read_text(encoding="utf-8")
        self.assertIn("lang::set(lang::Language::load(&[], lang::DEFAULT));", legacy)
        self.assertIn("game::run_with()", legacy)
        self.assertIn("horizon.legacyGl.startupError", legacy)

    def test_every_bundled_locale_has_the_new_ui_keys(self):
        languages = sorted(LANG_DIR.glob("*.json"))
        self.assertGreater(len(languages), 1)
        for path in languages:
            data = json.loads(path.read_text(encoding="utf-8"))
            for key in UI_KEYS:
                self.assertIn(key, data, f"{path.name} is missing {key}")
        en = json.loads((LANG_DIR / "en_us.json").read_text(encoding="utf-8"))
        self.assertNotEqual(en["options.graphicsApi.legacygl"], "options.graphicsApi.legacygl")

    def test_no_hardcoded_english_ui_labels_in_new_rust_sources(self):
        for path in (GRAPHICS_RS, GRAPHICS_LEGACY_RS, OPTIONS_UI, ENTRY_RS, ENTRY_LEGACY_RS, GAME_LEGACY_RS, LEGACY_GL_RS):
            source = path.read_text(encoding="utf-8")
            for literal in ('"Auto"', '"Vulkan"', '"DirectX 12"', '"OpenGL"', '"Default"'):
                self.assertNotIn(literal, source, f"{path.name} hardcodes {literal}")
        for path in (ENTRY_LEGACY_RS, GAME_LEGACY_RS, LEGACY_GL_RS):
            source = path.read_text(encoding="utf-8")
            self.assertIn("lang::", source)


class OwnBranchWorkflowAndDocsTests(unittest.TestCase):
    def test_workflow_checks_the_full_legacy_build_and_game_frames(self):
        source = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("arena/81644160-horizon", source)
        self.assertNotIn("pull_request:", source)
        self.assertIn("tests.test_gl21_integration", source)
        self.assertIn("tests.test_legacy_game", source)
        self.assertIn("--game --frames 2", source)
        self.assertIn("legacy-gl game: OK", source)
        self.assertIn("--integrated --frames 2", source)
        self.assertIn("--lan-port 0", source)
        self.assertIn("legacy-gl integrated: OK", source)
        self.assertIn("tests.test_legacy_server", source)
        self.assertIn("tests.test_legacy_multiplayer", source)
        self.assertIn("legacy-gl multiplayer: OK", source)
        self.assertIn("cargo test -p horizon-client --lib legacy_game --no-default-features --features legacy-gl", source)
        self.assertIn("cargo check -p horizon-client --features legacy-gl --no-default-features", source)
        self.assertIn("cargo tree -p horizon-client --no-default-features --features legacy-gl", source)
        self.assertIn("--edges no-dev", source)
        self.assertIn("-e features,no-dev", source)
        self.assertIn("no wgpu/naga/pollster/modern-gl", source)
        self.assertIn("cargo test -p horizon-client-render --lib gfx21 --no-default-features --features legacy-gl", source)
        self.assertIn("--example gl21_clear --no-default-features --features legacy-gl", source)
        self.assertIn("cargo run -p horizon-client --no-default-features --features legacy-gl -- --legacy-gl --frames 1", source)
        self.assertIn("legacy-gl frame: OK", source)
        self.assertIn("actions/upload-artifact@v4", source)
        self.assertIn("legacy-gl-Cargo.lock", source)
        self.assertEqual(source.count("\n      - name:"), 14, "workflow step count changed")

    def test_contract_and_docs_describe_full_integration(self):
        contract = CONTRACT.read_text(encoding="utf-8")
        self.assertIn("## Full PR 8 integration (this branch)", contract)
        self.assertIn("no longer", contract)
        self.assertIn("enables `modern-gl`", contract)
        self.assertIn("crates/horizon-client/src/legacy_gl.rs", contract)
        design = DESIGN.read_text(encoding="utf-8")
        self.assertIn("полная интеграция", design)
        self.assertIn("14 шагов", design)
        self.assertNotIn("Пока игры под `legacy-gl` ещё нет", design)
        self.assertIn("Full PR 8 integration", CONTRACT.read_text(encoding="utf-8"))
        self.assertIn("PR 11", CONTRACT.read_text(encoding="utf-8"))
        self.assertIn("PR 12", CONTRACT.read_text(encoding="utf-8"))
        self.assertIn("PR 13", CONTRACT.read_text(encoding="utf-8"))
        self.assertIn("without `modern-gl`", README.read_text(encoding="utf-8"))
        self.assertIn("no longer", contract)
        self.assertIn("enables `modern-gl`", contract)
        self.assertIn("full integration", NATIVE.read_text(encoding="utf-8"))
        self.assertNotIn("experimental, additive", NATIVE.read_text(encoding="utf-8"))
        self.assertIn("legacy_gl.rs", NOTICES.read_text(encoding="utf-8"))
        self.assertNotIn("still includes `modern-gl`", README.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
