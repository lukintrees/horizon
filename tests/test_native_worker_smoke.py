"""Runner accounting only; these tests do NOT execute the real mesher or Game."""
import copy
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import native_worker_smoke as smoke
from tools.legacy_game_e2e import TestHome


class NativeWorkerSmoke(unittest.TestCase):
    def report(self):
        return dict(scenario="native-prepared-real-mesher", normal_resources=True,
                    pending_limit=1, nonempty_draw=True, replacement_draw_changed=True,
                    forgotten_draw_empty=True, game_acceptance=False,
                    terrain_generation_tested=False, save_reopen_tested=False,
                    worker_completions=20, obsolete_completions=1, retired_queued_meshes=0, received_columns=26, old_revision=2,
                    new_revision=3, resource_generation=1)

    def test_report_requires_observed_results_and_distinct_tickets(self):
        smoke.validate_report(self.report())
        canceled = self.report()
        canceled.update(obsolete_completions=0, retired_queued_meshes=1)
        smoke.validate_report(canceled)
        canceled["retired_queued_meshes"] = 0
        with self.assertRaises(ValueError):
            smoke.validate_report(canceled)
        for key in self.report():
            bad = self.report()
            del bad[key]
            with self.assertRaises(ValueError, msg=key):
                smoke.validate_report(bad)
        for key, value in [("game_acceptance", True), ("worker_completions", True),
                           ("resource_generation", 0), ("pending_limit", True),
                           ("new_revision", 2), ("forgotten_draw_empty", False)]:
            bad = copy.deepcopy(self.report())
            bad[key] = value
            with self.assertRaises(ValueError, msg=key):
                smoke.validate_report(bad)

    def test_exact_opt_in_test_no_zero_matching_tests_success(self):
        self.assertEqual(smoke.command(Path("/test-binary")),
                         ["/test-binary", smoke.TEST, "--exact", "--ignored", "--nocapture"])
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            resources = root / "resources"
            resources.mkdir()
            (resources / "horizon-resources.json").write_text("{}")
            with patch.object(smoke.sys, "platform", "linux"), patch.object(smoke, "LocalProcess") as process:
                process.return_value.wait.return_value = 0
                code, evidence = smoke.run(Path(__file__), resources, 1)
            self.assertEqual(code, 1)
            self.assertEqual(evidence["state"], "failed")
            self.assertNotIn("worker_report", evidence)
            process.return_value.stop.assert_called_once()

    def test_missing_prerequisites_are_blocked_not_passed(self):
        code, evidence = smoke.run(Path("/missing/native-tests"), Path("/missing/resources"), 1)
        self.assertEqual(code, 2)
        self.assertEqual(evidence["state"], "blocked")
        self.assertFalse(evidence["game_acceptance"])
        self.assertFalse(evidence["win7_qualification"])

    def test_environment_isolated_and_only_explicit_loader_override_survives(self):
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ, {
                "HORIZON_CONTROL": "/existing/socket", "HORIZON_FAKE_RESOURCES": "yes",
                "HORIZON_OSMESA_LIBRARY": "/native/libOSMesa.so"}):
            home = TestHome.create(Path(tmp), reused=False)
            env = smoke.environment(home, Path("/real/resources"), home.artifacts / "report.json")
        self.assertNotIn("HORIZON_CONTROL", env)
        self.assertNotIn("HORIZON_FAKE_RESOURCES", env)
        self.assertEqual(env["HORIZON_TEST_RESOURCES"], "/real/resources")
        self.assertEqual(env["HORIZON_OSMESA_LIBRARY"], "/native/libOSMesa.so")
        self.assertEqual(env["HOME"], str(home.root))
        self.assertEqual(env["MESA_GL_VERSION_OVERRIDE"], "2.0")
