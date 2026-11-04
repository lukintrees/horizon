import json
import sys
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


if __name__ == "__main__":
    unittest.main()
