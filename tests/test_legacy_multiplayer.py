"""Static contract tests for PR 13: authoritative multiplayer state."""

import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GAME_DIR = ROOT / "crates/horizon-client/src/legacy_game"
MOD = GAME_DIR / "mod.rs"
SERVER = GAME_DIR / "server.rs"
NET = GAME_DIR / "net.rs"
UI = GAME_DIR / "ui.rs"
SCENE = GAME_DIR / "scene.rs"
LEGACY_GL = ROOT / "crates/horizon-client/src/legacy_gl.rs"
WORKFLOW = ROOT / ".github/workflows/arena-legacy-gl.yml"
CONTRACT = ROOT / "crates/horizon-client-render/src/gfx21/CONTRACT.md"
DESIGN = ROOT / "docs/design/legacy-gl21.md"
README = ROOT / "README.md"
NATIVE = ROOT / "docs/native-client.md"
NOTICES = ROOT / "THIRD_PARTY_NOTICES.md"
LANG_DIR = ROOT / "crates/horizon-client/assets/horizon/lang"

PR13_KEYS = (
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


class MultiplayerSourceTests(unittest.TestCase):
    def test_server_owns_players_chat_commands_and_container(self):
        text = SERVER.read_text(encoding="utf-8")
        for marker in (
            "ServerPlayer",
            "ClientCommand",
            "ContainerAction",
            "ServerCommand::Client",
            "ClientCommand::Chat",
            "ClientCommand::Container",
            "fn command<F: FnMut(ServerEvent)>",
            "fn chat_commands_and_container_are_authoritative",
            "horizon.legacyGl.game.commandHelp",
            "horizon.legacyGl.game.playerJoined",
        ):
            self.assertIn(marker, text)
        self.assertNotIn("wgpu::", text)
        self.assertNotIn("unsafe", text)

    def test_lan_protocol_assigns_names_and_routes_actor_commands(self):
        text = NET.read_text(encoding="utf-8")
        for marker in (
            "ServerEvent::Welcome",
            "ServerCommand::Client",
            "ServerCommand::Ping(id)",
            "ServerCommand::Leave {",
            "peer.port()",
            "name.clone()",
        ):
            self.assertIn(marker, text)
        self.assertNotIn("wgpu::", text)

    def test_game_routes_chat_container_and_remote_players(self):
        text = MOD.read_text(encoding="utf-8")
        for marker in (
            "Screen::Chat",
            "ClientCommand::Chat",
            "ClientCommand::Container",
            "ClientCommand::SetDistance",
            "ContainerAction",
            "remote_players",
            "container: [u8; 27]",
            "chat_input: String",
            "fn submit_chat",
            "fn append_chat_text",
            "fn container_action",
        ):
            self.assertIn(marker, text)

    def test_ui_and_scene_show_multiplayer_state(self):
        ui = UI.read_text(encoding="utf-8")
        for marker in ("Screen::Chat", "Container(u8)", "chatPrompt", "containerTitle", "playerList", "chatLine"):
            self.assertIn(marker, ui)
        scene = SCENE.read_text(encoding="utf-8")
        for marker in ("remote_players", "name_color", "ServerPlayer"):
            self.assertIn(marker, scene)
        backend = LEGACY_GL.read_text(encoding="utf-8")
        self.assertIn("event.text.as_deref()", backend)
        self.assertIn("legacy-gl multiplayer: OK", backend)

    def test_every_locale_has_the_pr13_keys(self):
        for path in sorted(LANG_DIR.glob("*.json")):
            data = json.loads(path.read_text(encoding="utf-8"))
            for key in PR13_KEYS:
                self.assertIn(key, data, f"{path.name} is missing {key}")
            self.assertIn("--join", data["horizon.legacyGl.help"])
            self.assertIn("T", data["horizon.legacyGl.game.controls"])


class MultiplayerWorkflowTests(unittest.TestCase):
    def test_workflow_runs_the_new_static_and_rust_tests(self):
        text = WORKFLOW.read_text(encoding="utf-8")
        for marker in (
            "tests.test_legacy_multiplayer",
            "legacy-gl multiplayer: OK",
            "cargo test -p horizon-client --lib legacy_game --no-default-features --features legacy-gl",
        ):
            self.assertIn(marker, text)
        self.assertEqual(text.count("\n      - name:"), 14)

    def test_docs_describe_pr13_and_its_limitations(self):
        contract = CONTRACT.read_text(encoding="utf-8")
        design = DESIGN.read_text(encoding="utf-8")
        for marker in ("PR 13", "multiplayer", "chat commands", "shared container", "visible remote players", "Minecraft Java protocol"):
            self.assertIn(marker, contract)
        for marker in ("PR 13", "мультиплеер", "чат", "контейнер", "14 шагов"):
            self.assertIn(marker, design)
        self.assertIn("chat commands", README.read_text(encoding="utf-8"))
        self.assertIn("PR 13", NATIVE.read_text(encoding="utf-8"))
        self.assertIn("PR 13", NOTICES.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
