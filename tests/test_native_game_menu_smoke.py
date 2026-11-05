"""Smoke runner contracts only; these mocks are NOT actual Game runtime evidence."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from tools import native_game_menu_smoke as smoke


def report():
    return {"scenario": "native-game-menu-only", "backend": "native-gl2",
            "game_constructed": True, "loading_context_transferred": True,
            "loading_worker_joined": True,
            "loading_presentations": 2, "loading_gpu_checkpoints": 10,
            "loading_gpu_presentations": 1, "loading_gpu_elapsed_ms": 8, "menu_submissions": 3, "resource_generation": 1,
            "outcome": "Presented", "presented": True, "playable": False, "world_opened": False}


class SmokeTests(unittest.TestCase):
    def test_requires_actual_visible_game_menu_not_bootstrap_or_fixture(self):
        smoke.validate_report(report(), 3)
        for key, value in (("scenario", "native-game-bootstrap-only"), ("outcome", "Offscreen"),
                           ("backend", "modern"), ("game_constructed", False),
                           ("loading_context_transferred", False), ("loading_worker_joined", False),
                           ("loading_presentations", 0), ("loading_gpu_checkpoints", 8),
                           ("loading_gpu_checkpoints", True), ("loading_gpu_presentations", -1),
                           ("loading_gpu_presentations", 3), ("loading_gpu_elapsed_ms", -1),
                           ("loading_gpu_elapsed_ms", 1.5),
                           ("menu_submissions", 0), ("menu_submissions", True),
                           ("resource_generation", None), ("presented", 1),
                           ("playable", True), ("world_opened", True)):
            data = report()
            data[key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                smoke.validate_report(data, 3)
        for key in report():
            data = report()
            del data[key]
            with self.subTest(missing=key), self.assertRaises(ValueError):
                smoke.validate_report(data, 3)

    def test_minimized_gpu_frames_are_not_fabricated_or_required(self):
        data = report()
        data.update(loading_gpu_presentations=0, loading_gpu_elapsed_ms=0)
        smoke.validate_report(data, 3)
        data.update(loading_presentations=100, loading_gpu_presentations=11)
        with self.assertRaises(ValueError):
            smoke.validate_report(data, 3)

    def test_missing_prerequisites_block_without_starting_a_process(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(smoke, "LocalProcess") as process:
            code, evidence = smoke.run(Path(tmp) / "no-game", Path(tmp), 3, 10)
            self.assertEqual(code, 2)
            self.assertEqual(evidence["state"], "blocked")
            process.assert_not_called()

    def test_scratch_isolation_flags_and_inherited_fixture_removal(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = smoke.TestHome.create(Path(tmp), reused=False)
            with patch.dict(os.environ, {"DISPLAY": ":91", "HORIZON_COMMAND_SCRIPT": "real.txt",
                                         "HORIZON_SESSION": "must-not-inherit"}):
                env = smoke.environment(home)
            self.assertEqual(env["DISPLAY"], ":91")
            for key in ("HORIZON_CONTROL", "HORIZON_COMMAND_SCRIPT", "HORIZON_SESSION"):
                self.assertNotIn(key, env)
            for key in ("HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "HORIZON_HOME",
                        "XDG_DATA_HOME", "XDG_CONFIG_HOME", "HORIZON_LAUNCHER_HOME"):
                self.assertTrue(Path(env[key]).is_relative_to(Path(tmp)))
            self.assertEqual(env["HORIZON_NO_AUDIO"], "1")
            cmd = smoke.command(Path("/game"), Path("/resources"), home, 3, 20)
            self.assertNotIn("--control", cmd)
            self.assertNotIn("--offscreen", cmd)
            self.assertNotIn("--native-bootstrap-only", cmd)
            self.assertEqual(cmd[cmd.index("--timeout-seconds") + 1], "20")

    def test_timeout_stops_only_started_process_and_cannot_pass(self):
        class Process:
            stopped = False
            def __init__(self, command, cwd, env, stdout, stderr):
                stdout.write_text("")
                stderr.write_text("")
            def wait(self, timeout):
                return None
            def stop(self):
                Process.stopped = True
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ, {"DISPLAY": ":91"}):
            game = Path(tmp) / "game"
            game.write_bytes(b"unit-test-binary-not-executed")
            with patch.object(smoke, "LocalProcess", Process):
                code, evidence = smoke.run(game, Path(tmp), 3, 1)
            self.assertEqual(code, 1)
            self.assertEqual(evidence["state"], "failed")
            self.assertTrue(Process.stopped)
            self.assertFalse(evidence["world_acceptance"])


if __name__ == "__main__":
    unittest.main()
