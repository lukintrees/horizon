"""Static contract tests for PR 11: interactive offline GL2 single-player."""

import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GAME_DIR = ROOT / "crates/horizon-client/src/legacy_game"
CLIENT_LIB = ROOT / "crates/horizon-client/src/lib.rs"
LEGACY_GL = ROOT / "crates/horizon-client/src/legacy_gl.rs"
GAME_LEGACY = ROOT / "crates/horizon-client/src/game_legacy.rs"
FACADE = ROOT / "crates/horizon-client-render/src/gfx21/facade.rs"
WORKFLOW = ROOT / ".github/workflows/arena-legacy-gl.yml"
CONTRACT = ROOT / "crates/horizon-client-render/src/gfx21/CONTRACT.md"
DESIGN = ROOT / "docs/design/legacy-gl21.md"
LANG_DIR = ROOT / "crates/horizon-client/assets/horizon/lang"

GAME_KEYS = (
    "horizon.legacyGl.game.title",
    "horizon.legacyGl.game.newWorld",
    "horizon.legacyGl.game.loadWorld",
    "horizon.legacyGl.game.options",
    "horizon.legacyGl.game.quit",
    "horizon.legacyGl.game.controls",
    "horizon.legacyGl.game.paused",
    "horizon.legacyGl.game.resume",
    "horizon.legacyGl.game.inventory",
    "horizon.legacyGl.game.saveQuit",
    "horizon.legacyGl.game.back",
    "horizon.legacyGl.game.minus",
    "horizon.legacyGl.game.plus",
    "horizon.legacyGl.game.renderDistance",
    "horizon.legacyGl.game.fov",
    "horizon.legacyGl.game.sensitivity",
    "horizon.legacyGl.game.fps",
    "horizon.legacyGl.game.position",
    "horizon.legacyGl.game.chunks",
    "horizon.legacyGl.game.draws",
    "horizon.legacyGl.game.day",
    "horizon.legacyGl.game.seed",
    "horizon.legacyGl.game.health",
    "horizon.legacyGl.game.hunger",
    "horizon.legacyGl.game.count",
    "horizon.legacyGl.game.block.air",
    "horizon.legacyGl.game.block.grass",
    "horizon.legacyGl.game.block.dirt",
    "horizon.legacyGl.game.block.stone",
    "horizon.legacyGl.game.block.sand",
    "horizon.legacyGl.game.block.water",
    "horizon.legacyGl.game.block.wood",
    "horizon.legacyGl.game.block.leaves",
    "horizon.legacyGl.game.block.planks",
    "horizon.legacyGl.game.block.cobble",
    "horizon.legacyGl.game.started",
    "horizon.legacyGl.game.frame",
    "horizon.legacyGl.saveError",
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


def source(name):
    return (GAME_DIR / name).read_text(encoding="utf-8")


class LegacyGameSourceTests(unittest.TestCase):
    def test_all_pr11_modules_exist_and_are_wired(self):
        for name in ("mod.rs", "world.rs", "player.rs", "save.rs", "ui.rs", "text.rs", "scene.rs"):
            self.assertTrue((GAME_DIR / name).is_file(), name)
        self.assertIn("pub mod legacy_game;", CLIENT_LIB.read_text(encoding="utf-8"))
        self.assertIn("renderer.run(&self.options.settings)?;", GAME_LEGACY.read_text(encoding="utf-8"))

    def test_world_streaming_and_meshing_are_present(self):
        text = source("world.rs")
        for marker in ("CHUNK_SIZE", "WORLD_HEIGHT", "stream_around", "mesh_chunk", "saved_edits"):
            self.assertIn(marker, text)
        self.assertNotIn("wgpu::", text)
        self.assertNotIn("unsafe", text)

    def test_player_physics_editing_and_raycast_are_present(self):
        text = source("player.rs")
        for marker in ("fn update(", "fn raycast(", "fn break_block(", "fn place_block(", "fn collides("):
            self.assertIn(marker, text)
        self.assertNotIn("wgpu::", text)

    def test_save_and_restore_are_present(self):
        text = source("save.rs")
        for marker in ("pub fn load(", "pub fn save(", "SaveState", "serde_json"):
            self.assertIn(marker, text)

    def test_ui_text_and_scene_are_localized_and_dynamic(self):
        ui = source("ui.rs")
        for marker in ("enum Screen", "Title", "Playing", "Pause", "Inventory", "Options", "lang::get", "lang::tr"):
            self.assertIn(marker, ui)
        for literal in ('"New world"', '"Options"', '"Quit"', '"Inventory"'):
            self.assertNotIn(literal, ui)
        text = source("text.rs")
        for marker in ("ab_glyph", "horizon_theme::fonts::GEIST_MONO", "TextAtlas::build"):
            self.assertIn(marker, text)
        scene = source("scene.rs")
        for marker in ("Mat4::look_at_rh", "impl RenderScene", "pub fn build(", "TextAtlas::build", "ChunkMesh"):
            self.assertIn(marker, scene)

    def test_game_state_handles_input_menus_and_saves(self):
        text = source("mod.rs")
        for marker in ("pub struct Game", "fn key_down", "fn mouse_click", "fn handle_action", "fn save(", "Screen::Title"):
            self.assertIn(marker, text)

    def test_gl_backend_has_an_interactive_game_mode(self):
        text = LEGACY_GL.read_text(encoding="utf-8")
        for marker in (
            "Mode::Game",
            "fn draw_game",
            "chunk_buffers",
            "window.set_cursor_grab",
            "DeviceEvent::MouseMotion",
            "ControlFlow::WaitUntil",
            "MouseWheel",
            "legacy-gl game: OK",
            "legacy-gl integrated: OK",
            "legacy-gl network: OK",
            "legacy-gl multiplayer: OK",
            "event.text.as_deref()",
            "horizon.legacyGl.saveError",
        ):
            self.assertIn(marker, text)
        self.assertIn("FullFramePlan::reference", text, "reference smoke mode must remain")

    def test_settings_support_the_game_flags(self):
        text = FACADE.read_text(encoding="utf-8")
        for marker in ("--game", "--frames", "--seed", "--world", "--fov", "--sensitivity", "--integrated", "--lan-port", "--join", "--name", "pub game: bool", "pub seed: u64"):
            self.assertIn(marker, text)

    def test_every_locale_has_the_pr11_keys(self):
        for path in sorted(LANG_DIR.glob("*.json")):
            data = json.loads(path.read_text(encoding="utf-8"))
            for key in GAME_KEYS:
                self.assertIn(key, data, f"{path.name} is missing {key}")
            for flag in ("--game", "--integrated", "--lan-port", "--join", "--name"):
                self.assertIn(flag, data["horizon.legacyGl.help"])


class LegacyGameWorkflowTests(unittest.TestCase):
    def test_workflow_runs_the_interactive_game_and_its_unit_tests(self):
        text = WORKFLOW.read_text(encoding="utf-8")
        for marker in (
            "tests.test_legacy_game",
            "cargo test -p horizon-client --lib legacy_game --no-default-features --features legacy-gl",
            "--game --frames 2",
            "legacy-gl game: OK",
            "legacy_game/*.rs",
        ):
            self.assertIn(marker, text)
        self.assertIn("rustfmt", text)
        self.assertEqual(text.count("\n      - name:"), 14)

    def test_contract_and_design_describe_pr11(self):
        contract = CONTRACT.read_text(encoding="utf-8")
        design = DESIGN.read_text(encoding="utf-8")
        for marker in ("PR 11", "single-player", "networking"):
            self.assertIn(marker, contract)
        for marker in ("PR 11", "одиночн", "сеть", "14 шагов"):
            self.assertIn(marker, design)


if __name__ == "__main__":
    unittest.main()
