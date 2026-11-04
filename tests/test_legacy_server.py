"""Static contract tests for PR 12: integrated server and LAN link."""

import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GAME_DIR = ROOT / "crates/horizon-client/src/legacy_game"
MOD = GAME_DIR / "mod.rs"
SERVER = GAME_DIR / "server.rs"
NET = GAME_DIR / "net.rs"
FACADE = ROOT / "crates/horizon-client-render/src/gfx21/facade.rs"
LEGACY_GL = ROOT / "crates/horizon-client/src/legacy_gl.rs"
UI = GAME_DIR / "ui.rs"
WORKFLOW = ROOT / ".github/workflows/arena-legacy-gl.yml"
CONTRACT = ROOT / "crates/horizon-client-render/src/gfx21/CONTRACT.md"
DESIGN = ROOT / "docs/design/legacy-gl21.md"
README = ROOT / "README.md"
NATIVE = ROOT / "docs/native-client.md"
NOTICES = ROOT / "THIRD_PARTY_NOTICES.md"
LANG_DIR = ROOT / "crates/horizon-client/assets/horizon/lang"

PR12_KEYS = (
    "horizon.legacyGl.game.network",
    "horizon.legacyGl.game.networkError",
    "horizon.legacyGl.game.chat",
    "horizon.legacyGl.game.offline",
    "horizon.legacyGl.game.integrated",
    "horizon.legacyGl.game.remote",
    "horizon.legacyGl.game.integratedStarted",
    "horizon.legacyGl.game.remoteStarted",
    "horizon.legacyGl.game.lanStarted",
)


class IntegratedServerSourceTests(unittest.TestCase):
    def test_server_and_net_modules_are_wired(self):
        for marker in ("pub mod net;", "pub mod server;", "Link::open(settings)", "apply_snapshot"):
            self.assertIn(marker, MOD.read_text(encoding="utf-8"))
        self.assertTrue(SERVER.is_file())
        self.assertTrue(NET.is_file())

    def test_authoritative_server_has_commands_events_and_tests(self):
        text = SERVER.read_text(encoding="utf-8")
        for marker in (
            "IntegratedState",
            "ServerCommand",
            "ServerEvent",
            "ServerSnapshot",
            "LinkMode",
            "NetServer",
            "NetClient",
            "fn integrated_server_steps_and_answers_a_ping",
            "fn integrated_and_remote_links_exchange_a_ping",
        ):
            self.assertIn(marker, text)
        self.assertNotIn("wgpu::", text)
        self.assertNotIn("unsafe", text)

    def test_lan_protocol_uses_std_tcp_and_json_lines(self):
        text = NET.read_text(encoding="utf-8")
        for marker in (
            "TcpListener",
            "TcpStream",
            "write_message",
            "parse_message",
            "NetMessage",
            "Hello",
            "Event(ServerEvent)",
            "Bye",
            "fn a_client_reads_a_loopback_welcome_and_event",
        ):
            self.assertIn(marker, text)
        self.assertNotIn("wgpu::", text)
        self.assertNotIn("unsafe", text)

    def test_game_routes_input_and_hud_shows_network_status(self):
        text = MOD.read_text(encoding="utf-8")
        for marker in (
            "ServerCommand::Input(self.input.clone())",
            "ServerCommand::Pause",
            "ServerCommand::Break",
            "ServerCommand::Place",
            "ServerCommand::SetHotbar",
            "network_mode",
        ):
            self.assertIn(marker, text)
        ui = UI.read_text(encoding="utf-8")
        for marker in ("network_mode", "server_tick", "server_tps", "network_error", "lang::get(mode_key)"):
            self.assertIn(marker, ui)

    def test_settings_and_backend_expose_the_network_flags(self):
        facade = FACADE.read_text(encoding="utf-8")
        for marker in ("--integrated", "--lan-port", "--join", "--name", "pub lan_port: Option<u16>"):
            self.assertIn(marker, facade)
        backend = LEGACY_GL.read_text(encoding="utf-8")
        for marker in (
            "legacy-gl integrated: OK",
            "legacy-gl network: OK",
            "LinkMode::Integrated",
            "LinkMode::Remote",
            "self.game.shutdown()",
        ):
            self.assertIn(marker, backend)

    def test_every_locale_has_the_pr12_keys_and_help(self):
        for path in sorted(LANG_DIR.glob("*.json")):
            data = json.loads(path.read_text(encoding="utf-8"))
            for key in PR12_KEYS:
                self.assertIn(key, data, f"{path.name} is missing {key}")
            help_text = data["horizon.legacyGl.help"]
            for flag in ("--game", "--integrated", "--join", "--lan-port", "--name"):
                self.assertIn(flag, help_text)


class IntegratedServerWorkflowTests(unittest.TestCase):
    def test_workflow_runs_new_tests_and_integrated_frames(self):
        text = WORKFLOW.read_text(encoding="utf-8")
        for marker in (
            "tests.test_legacy_server",
            "--integrated --frames 2",
            "--lan-port 0",
            "legacy-gl integrated: OK",
            "cargo test -p horizon-client --lib legacy_game --no-default-features --features legacy-gl",
        ):
            self.assertIn(marker, text)
        self.assertEqual(text.count("\n      - name:"), 14)

    def test_docs_describe_pr12_and_its_limitations(self):
        contract = CONTRACT.read_text(encoding="utf-8")
        design = DESIGN.read_text(encoding="utf-8")
        for marker in ("PR 12", "integrated server", "LAN", "Minecraft Java protocol"):
            self.assertIn(marker, contract)
        for marker in ("PR 12", "интегрированный сервер", "LAN", "14 шагов"):
            self.assertIn(marker, design)
        self.assertIn("integrated server", README.read_text(encoding="utf-8"))
        self.assertIn("PR 12", NATIVE.read_text(encoding="utf-8"))
        self.assertIn("PR 12", NOTICES.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
