"""Unit stubs for the harness; these are not evidence of a running game."""
import sys
import tempfile
import unittest
from pathlib import Path

from tools.legacy_game_e2e import run_game

GOOD = "import sys; print('HORIZON_E2E_BACKEND legacy-gl'); " \
       "[print('HORIZON_E2E_CHECKPOINT '+s) for s in ('startup','menu','world','save')]"


class LegacyGameE2ETests(unittest.TestCase):
    def test_stub_with_required_evidence_succeeds_only_as_process_harness(self):
        result = run_game([sys.executable, "-c", GOOD], timeout=3)
        self.assertEqual(result["status"], "success")
        self.assertEqual(result["evidence_kind"], "real_process_logs")
        self.assertEqual(result["backend"], "legacy-gl")

    def test_timeout_is_failure_and_child_is_stopped(self):
        result = run_game([sys.executable, "-c", "import time; time.sleep(30)"], timeout=.1)
        self.assertEqual(result["status"], "failure")
        self.assertEqual(result["reason"], "timeout")

    def test_crash_is_failure(self):
        result = run_game([sys.executable, "-c", "raise SystemExit(7)"], timeout=3)
        self.assertEqual(result["status"], "failure")
        self.assertEqual(result["exit_code"], 7)

    def test_missing_checkpoints_fails(self):
        result = run_game([sys.executable, "-c", "print('HORIZON_E2E_BACKEND legacy-gl')"], timeout=3)
        self.assertEqual(result["reason"], "checkpoints_missing")
        self.assertIn("world", result["missing_checkpoints"])

    def test_unconfirmed_backend_is_blocked(self):
        result = run_game([sys.executable, "-c", "print('startup')"], timeout=3)
        self.assertEqual(result["status"], "blocked")
        self.assertEqual(result["reason"], "legacy_backend_unconfirmed")

    def test_wrong_backend_and_fallback_fail(self):
        for backend, extra in (("wgpu", ""), ("legacy-gl", "print('fallback to wgpu')")):
            code = f"print('HORIZON_E2E_BACKEND {backend}'); " \
                   "[print('HORIZON_E2E_CHECKPOINT '+s) for s in ('startup','menu','world','save')]"
            if extra:
                code = f"print('HORIZON_E2E_BACKEND {backend}'); print('fallback to wgpu'); " \
                       "[print('HORIZON_E2E_CHECKPOINT '+s) for s in ('startup','menu','world','save')]"
            result = run_game([sys.executable, "-c", code], timeout=3)
            self.assertIn(result["status"], ("blocked", "failure"))
            self.assertIn(result["reason"], ("legacy_backend_unconfirmed", "unexpected_modern_fallback"))

    def test_reuse_home_for_same_save_and_refuse_unmarked_path(self):
        with tempfile.TemporaryDirectory() as temp:
            home = Path(temp) / "e2e"
            first = run_game([sys.executable, "-c", GOOD], timeout=3, home=home)
            second = run_game([sys.executable, "-c", GOOD], timeout=3, home=home)
            self.assertEqual(first["test_home"], second["test_home"])
            unsafe = Path(temp) / "user-data"
            unsafe.mkdir()
            with self.assertRaises(RuntimeError):
                run_game([sys.executable, "-c", GOOD], timeout=3, home=unsafe)


if __name__ == "__main__":
    unittest.main()
