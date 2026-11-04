"""Independent checks for the PR 9 additive smoke report."""

import unittest

from tools import legacy_gl_golden as golden
from tools import legacy_gl_smoke as smoke

ROOT = golden.ROOT
CAPS_PATH = smoke.CAPS_PATH
STATS_PATH = smoke.STATS_PATH
REPORT_PATH = smoke.REPORT_PATH


class SmokeReportTests(unittest.TestCase):
    def test_committed_stats_match_the_independent_frame(self):
        stats = smoke.load_json(STATS_PATH)
        self.assertEqual(stats, golden.STATS)
        self.assertEqual(stats["draw_calls"], 5)
        self.assertEqual(stats["samplers"], 4)
        self.assertEqual(stats["vertices"], 12)
        self.assertEqual(stats["indices"], 18)
        self.assertEqual(stats["full_frame_draws"], 16)
        self.assertEqual(stats["vram_mib"], 48)
        self.assertEqual(stats["gl_errors"], 0)
        self.assertIsNone(stats["measured_on"])
        self.assertIsNone(stats["fps"])

    def test_reference_caps_and_stats_pass_the_budget(self):
        caps = smoke.load_json(CAPS_PATH)
        stats = smoke.load_json(STATS_PATH)
        self.assertTrue(smoke.validate(caps, stats))
        self.assertEqual(caps["tier"], "L1")
        self.assertEqual(caps["glsl"], 120)
        self.assertEqual(caps["extension_count"], 85)
        self.assertIsNone(caps["measured_on"])

    def test_committed_report_matches_the_generated_report(self):
        caps = smoke.load_json(CAPS_PATH)
        stats = smoke.load_json(STATS_PATH)
        self.assertEqual(REPORT_PATH.read_text(encoding="utf-8"), smoke.build_report(caps, stats))

    def test_report_lines_have_independent_expected_values(self):
        caps = smoke.load_json(CAPS_PATH)
        stats = smoke.load_json(STATS_PATH)
        lines = smoke.build_lines(caps, stats)
        self.assertEqual(len(lines), 17)
        self.assertEqual(lines[0], "Legacy GL startup report (PR 9 diagnostics; no GL context)")
        self.assertEqual(lines[4], "  GL_VERSION           2.1.8545 Release")
        self.assertEqual(lines[9], "  draw calls           5/200")
        self.assertEqual(lines[10], "  samplers             4/4")
        self.assertEqual(lines[11], "  VRAM estimate        48/160 MiB")
        self.assertEqual(lines[15], "  wgpu parity          NOT MEASURED")
        self.assertEqual(lines[16], "  real X1650 run       NOT MEASURED")

    def test_budget_rejects_excess_draws_and_gl_errors(self):
        caps = smoke.load_json(CAPS_PATH)
        stats = dict(smoke.load_json(STATS_PATH))
        stats["draw_calls"] = 201
        self.assertFalse(smoke.validate(caps, stats))
        stats["draw_calls"] = 5
        stats["gl_errors"] = 1
        self.assertFalse(smoke.validate(caps, stats))
        stats["gl_errors"] = 0
        stats["samplers"] = 5
        self.assertFalse(smoke.validate(caps, stats))

    def test_gl_error_names_are_explicit(self):
        self.assertEqual(smoke.gl_error_name(0x0000), "GL_NO_ERROR")
        self.assertEqual(smoke.gl_error_name(0x0502), "GL_INVALID_OPERATION")
        self.assertEqual(smoke.gl_error_name(0xDEAD), "UNKNOWN_GL_ERROR")

    def test_json_report_marks_unmeasured_acceptance(self):
        caps = smoke.load_json(CAPS_PATH)
        stats = smoke.load_json(STATS_PATH)
        report = smoke.build_json(caps, stats)
        self.assertTrue(report["budget_ok"])
        self.assertEqual(report["wgpu_parity"], "NOT MEASURED")
        self.assertEqual(report["real_x1650"], "NOT MEASURED")
        self.assertEqual(report["startup_lines"], smoke.build_lines(caps, stats))


if __name__ == "__main__":
    unittest.main()
