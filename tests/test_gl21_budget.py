import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from tools import gl21_budget as budget  # noqa: E402


class ComponentsTests(unittest.TestCase):
    def test_types_that_fit_a_uniform(self):
        self.assertEqual(budget.components("f32"), 1)
        self.assertEqual(budget.components("u32"), 1)
        self.assertEqual(budget.components("vec3<f32>"), 3)
        self.assertEqual(budget.components("mat4x4<f32>"), 16)
        self.assertEqual(budget.components("array<vec4<f32>, 16>"), 64)
        self.assertIsNone(budget.components("SomeType"))


class StructParsingTests(unittest.TestCase):
    def test_nested_angle_brackets_do_not_split_members(self):
        text = """
        struct Globals {
            view_proj: mat4x4<f32>,
            // a comment, with a comma
            lights: array<vec4<f32>, 4>,
            @location(0) uv: vec2<f32>,
        }
        """
        fields = budget.struct_fields(text, "Globals")
        self.assertEqual([budget.field_type(f) for f in fields],
                         ["mat4x4<f32>", "array<vec4<f32>, 4>", "vec2<f32>"])
        self.assertEqual(sum(budget.field_components(f) or 0 for f in fields), 16 + 16 + 2)

    def test_missing_struct_is_empty(self):
        self.assertEqual(budget.struct_fields("struct A { x: f32 }", "Missing"), [])


class ScanTests(unittest.TestCase):
    def setUp(self):
        self.shaders = budget.scan_shaders()

    def test_vertex_texture_fetch_is_found(self):
        # The X1650 reports 0 vertex texture image units; the renderer's one
        # vertex-side sample must stay visible to the check.
        hits = [s["path"] for s in self.shaders
                for st in s["stages"] if st["kind"] == "vertex" and st["samples_textures"]]
        self.assertIn("entity.wgsl", hits)

    def test_uniform_blocks_are_measured(self):
        terrain = next(s for s in self.shaders if s["path"] == "terrain.wgsl")
        self.assertGreater(terrain["uniform_structs"]["Globals"], 512)


class ProfileTests(unittest.TestCase):
    def report(self, profile):
        return budget.report(budget.scan_shaders(), budget.scan_rust(), profile)

    def test_gl21_report_flags_what_gl33_accepts(self):
        gl21 = self.report(budget.GL21)
        gl33 = self.report(budget.GL33)
        categories21 = {f["category"] for f in gl21["findings"]}
        categories33 = {f["category"] for f in gl33["findings"]}
        self.assertIn("uniform block", categories21)
        self.assertIn("compute shader", categories21)
        self.assertIn("storage buffer", categories21)
        self.assertIn("indirect draw", categories21)
        self.assertNotIn("uniform block", categories33)
        self.assertNotIn("indirect draw", categories33)

    def test_report_is_json_serialisable(self):
        json.dumps(self.report(budget.GL21))


    def test_shipped_gl21_shaders_fit_while_the_wgpu_path_does_not(self):
        """The premise of the port, as a test: the GLSL 1.20 shaders this crate ships are
        inside the target's budgets, and the WGSL path that made the target unreachable is
        not. If either half of that ever flips, the plan's premise has changed."""
        glsl = ROOT / "tools" / "gl21_glsl.py"
        result = subprocess.run(
            [sys.executable, str(glsl), "--check"],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("0 finding", result.stdout)

        result = subprocess.run(
            [sys.executable, str(ROOT / "tools" / "gl21_budget.py"), "--check"],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 1, "the wgpu path is inside the budget?!")
        self.assertIn("OVER BUDGET", result.stdout)

    def test_the_baseline_gate_fails_only_on_a_new_finding(self):
        """`--check --baseline` is what CI runs: 80 findings of known debt are not a failure,
        one new finding is. This builds a baseline from the current report and then runs it
        twice: against itself (passes) and against a report with a finding removed (which
        would make every current finding new)."""
        baseline = ROOT / "tools" / "gl21-budget-baseline.json"
        self.assertTrue(baseline.is_file(), "the committed baseline is what CI compares against")
        result = subprocess.run(
            [
                sys.executable,
                str(ROOT / "tools" / "gl21_budget.py"),
                "--check",
                "--baseline",
                str(baseline),
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("no new findings", result.stdout)

        saved = json.loads(baseline.read_text())
        self.assertGreater(len(saved["findings"]), 0)
        self.assertEqual(saved["profile"], "gl21-x1650")

        # A smaller baseline: every finding measured now is new, so the gate must fail.
        trimmed = dict(saved)
        trimmed["findings"] = saved["findings"][1:]
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(trimmed, handle)
            path = handle.name
        try:
            result = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "tools" / "gl21_budget.py"),
                    "--check",
                    "--baseline",
                    path,
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertIn("NEW:", result.stdout)
        finally:
            os.unlink(path)

if __name__ == "__main__":
    unittest.main()
