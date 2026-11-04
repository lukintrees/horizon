import json
import re
import tempfile
import unittest
from pathlib import Path

from tools import gl21_caps as caps

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tools/gl21_caps_fixtures/x1650-catalyst93.glxinfo.txt"


class ParseTests(unittest.TestCase):
    def test_parse_versions(self):
        self.assertEqual(caps.parse_version("2.1.8545 Release"), [2, 1])
        self.assertEqual(caps.parse_version("2.1 Mesa 23.2"), [2, 1])
        self.assertEqual(caps.parse_glsl("1.20"), 120)
        self.assertEqual(caps.parse_glsl("1.10"), 110)
        self.assertEqual(caps.parse_glsl("4.50"), 450)
        self.assertIsNone(caps.parse_glsl(None))

    def test_wrapped_extension_lines_are_parsed(self):
        text = """OpenGL vendor string: Example
OpenGL version string: 2.1 Mesa
OpenGL shading language version string: 1.20
OpenGL extensions:
    GL_ARB_multitexture, GL_ARB_vertex_buffer_object,
    GL_ARB_texture_non_power_of_two, GL_EXT_framebuffer_object
"""
        parsed = caps.parse_glxinfo(text)
        for name in caps.REQUIRED_BASE:
            self.assertIn(name, parsed["extensions"])
        self.assertIn("GL_EXT_framebuffer_object", parsed["extensions"])
        self.assertEqual(parsed["version"], [2, 1])
        self.assertEqual(parsed["glsl"], 120)

    def test_core_only_report_is_noted(self):
        text = """OpenGL vendor string: Example
OpenGL core profile version string: 4.5 (Core Profile) Mesa 23.2
OpenGL core profile shading language version string: 4.50
OpenGL extensions:
    GL_ARB_multitexture, GL_ARB_vertex_buffer_object,
    GL_ARB_texture_non_power_of_two, GL_EXT_framebuffer_object
"""
        report = caps.build_report(text, label="core", source_note="test")
        self.assertTrue(any("core profile" in note for note in report["notes"]))
        self.assertEqual(report["glsl"], 450)


class TierTests(unittest.TestCase):
    def test_reference_fixture_is_l1(self):
        report = caps.build_report(FIXTURE.read_text(encoding="utf-8"), label="x1650", source_note="test")
        self.assertEqual(report["tier"], "L1")
        self.assertEqual(report["version"], [2, 1])
        self.assertEqual(report["glsl"], 120)
        self.assertEqual(report["required_missing"], [])
        self.assertEqual(report["optional_missing"], [])
        self.assertEqual(caps.check_tier(report), [])
        self.assertIn("GL_EXT_multi_draw_arrays", report["optional_present"])
        self.assertIn("GL_ARB_shader_texture_lod", report["extensions"])
        self.assertEqual(report["limits"]["GL_MAX_TEXTURE_SIZE"], 4096)
        self.assertEqual(report["limits"]["GL_MAX_TEXTURE_IMAGE_UNITS"], 8)
        self.assertEqual(report["limits"]["GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS"], 0)
        self.assertEqual(report["limits"]["GL_MAX_DRAW_BUFFERS"], 4)

    def test_gl_20_with_glsl_110_is_l0(self):
        text = (FIXTURE.read_text(encoding="utf-8")
                .replace("OpenGL version string: 2.1.8545 Release", "OpenGL version string: 2.0.0 Release")
                .replace("OpenGL shading language version string: 1.20",
                         "OpenGL shading language version string: 1.10")
                .replace("Max compat profile version: 2.1", "Max compat profile version: 2.0"))
        report = caps.build_report(text, label="gl20", source_note="test")
        self.assertEqual(report["tier"], "L0")
        self.assertEqual(report["version"], [2, 0])
        self.assertEqual(report["glsl"], 110)
        self.assertEqual(caps.check_tier(report), [])

    def test_missing_fbo_is_unsupported(self):
        text = re.sub(r"\bGL_(?:EXT|ARB)_framebuffer_object\b,?", "",
                      FIXTURE.read_text(encoding="utf-8"))
        report = caps.build_report(text, label="no-fbo", source_note="test")
        self.assertEqual(report["tier"], "unsupported")
        self.assertIn(caps.FBO_REQUIREMENT, report["required_missing"])
        problems = caps.check_tier(report)
        self.assertTrue(any("tier" in problem for problem in problems))
        self.assertTrue(any(caps.FBO_REQUIREMENT in problem for problem in problems))

    def test_no_usable_glsl_falls_back_to_f0(self):
        text = re.sub(r"OpenGL shading language version string:.*\n", "",
                      FIXTURE.read_text(encoding="utf-8"))
        text = text.replace("OpenGL version string: 2.1.8545 Release",
                            "OpenGL version string: 1.5 Mesa")
        report = caps.build_report(text, label="f0", source_note="test")
        self.assertEqual(report["tier"], "F0")
        self.assertIsNone(report["glsl"])
        self.assertEqual(caps.check_tier(report), [])


class ReportTests(unittest.TestCase):
    def test_write_report_roundtrip(self):
        report = caps.build_report(FIXTURE.read_text(encoding="utf-8"),
                                   label="roundtrip", source_note="test")
        with tempfile.TemporaryDirectory() as tmp:
            json_path, text_path = caps.write_report(report, Path(tmp), "roundtrip")
            loaded = json.loads(json_path.read_text(encoding="utf-8"))
            self.assertEqual(loaded["tier"], "L1")
            self.assertEqual(loaded["label"], "roundtrip")
            self.assertEqual(loaded["schema"], caps.SCHEMA)
            self.assertIn("tier            L1", text_path.read_text(encoding="utf-8"))
            self.assertIn("GL_ARB_shader_texture_lod", text_path.read_text(encoding="utf-8"))

    def test_reference_report_on_disk_is_labelled_as_a_reference(self):
        path = ROOT / "benchmarks/graphics/x1650-reference-caps.json"
        report = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual(report["tier"], "L1")
        self.assertIsNone(report["measured_on"])
        self.assertIn("NOT measured in this sandbox", report["source"])


if __name__ == "__main__":
    unittest.main()
