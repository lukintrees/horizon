"""tools/gl21_shaders.py: the OpenGL 2.1 shader gate (docs/opengl21-plan.md).

The examples are what a 2020 GPU's shader and a 2006 GPU's shader differ by; the gate is what keeps
the second kind from drifting back into the first.
"""

import tempfile
from pathlib import Path
import unittest

from tools import gl21_shaders


def violations(text, stage=None):
    return [message for _line, message in gl21_shaders.check_text(text, stage)]


HEADER = "#version 120\n"


class AcceptsTests(unittest.TestCase):
    def test_a_plain_glsl_120_shader_passes(self):
        source = HEADER + """
attribute vec3 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
uniform vec4 u_tint;
uniform sampler2D u_atlas;
void main() {
    v_uv = a_uv;
    gl_Position = vec4(a_pos, 1.0);
}
"""
        self.assertEqual(violations(source), [])

    def test_a_fragment_shader_may_sample_with_an_explicit_lod(self):
        source = HEADER + """
uniform sampler2D u_atlas;
varying vec2 v_uv;
void main() {
    vec4 c = texture2DLod(u_atlas, v_uv, 0.0);
    if (c.a < 0.5) { discard; }
    gl_FragColor = c;
}
"""
        self.assertEqual(violations(source), [])

    def test_comments_about_newer_features_are_not_violations(self):
        source = HEADER + """
// textureLod() and layout(location = 0) need GLSL 1.30, so this file does not use them.
attribute vec2 a_pos;
void main() { gl_Position = vec4(a_pos, 0.0, 1.0); }
"""
        self.assertEqual(violations(source), [])

    def test_the_shaders_in_the_tree_pass(self):
        root = Path(gl21_shaders.__file__).resolve().parent.parent
        files = gl21_shaders.shader_files()
        if not files:
            self.skipTest("no GLSL 1.20 shaders yet")
        found, errors = gl21_shaders.check_files(files)
        self.assertEqual(errors, {})
        self.assertEqual(found, {})
        for path in files:
            self.assertTrue(path.is_relative_to(root), path)


class RejectsTests(unittest.TestCase):
    def check(self, source, fragment, stage=None):
        found = violations(source, stage)
        self.assertTrue(any(fragment in message for message in found), (fragment, found))

    def test_a_missing_version_line(self):
        self.check("attribute vec2 a_pos;\nvoid main() { gl_Position = vec4(a_pos, 0.0, 1.0); }\n",
                   "#version 120")

    def test_a_newer_version_line(self):
        self.check("#version 330 core\nin vec2 a_pos;\nvoid main() { gl_Position = vec4(a_pos, 0.0, 1.0); }\n",
                   "#version")

    def test_uniform_blocks_and_layout_qualifiers(self):
        self.check(HEADER + "uniform Globals { mat4 view_proj; } g;\n", "uniform blocks")
        self.check(HEADER + "layout(location = 0) in vec3 a_pos;\n", "`layout`")

    def test_texture_functions_from_glsl_130(self):
        for call in ["texture(atlas, uv)", "textureLod(atlas, uv, 0.0)", "textureGrad(atlas, uv, dx, dy)",
                     "texelFetch(atlas, ivec2(0), 0)", "textureSize(atlas, 0)"]:
            with self.subTest(call=call):
                self.check(HEADER + f"uniform sampler2D atlas;\nvoid main() {{ vec4 c = {call}; }}\n",
                           "GLSL 1.30")

    def test_texture_arrays_and_vertex_ids(self):
        self.check(HEADER + "uniform sampler2DArray atlas;\n", "texture arrays")
        self.check(HEADER + "void main() { gl_Position = vec4(float(gl_VertexID)); }\n", "gl_VertexID")

    def test_integer_arithmetic(self):
        self.check(HEADER + "void main() { int a = 1 << 2; int b = a >> 1; }\n", "bit shifts")
        self.check(HEADER + "void main() { float f = bitcast<float>(1u); }\n", "bit operations")

    def test_in_and_out_storage_qualifiers(self):
        self.check(HEADER + "in vec3 a_pos;\nvoid main() { gl_Position = vec4(a_pos, 1.0); }\n",
                   "attribute/varying")

    def test_a_vertex_shader_may_call_its_own_helper(self):
        # A function the shader declares itself is not a sample, whatever its name: only the
        # driver's own texture functions are.
        source = HEADER + """
attribute vec2 a_uv;
vec4 helper(vec2 p) { return vec4(p, 0.0, 1.0); }
void main() { gl_Position = helper(a_uv); }
"""
        self.assertEqual(violations(source, "vertex"), [])

    def test_a_vertex_shader_cannot_sample(self):
        source = HEADER + """
attribute vec2 a_uv;
varying vec2 v_uv;
uniform sampler2D u_height;
void main() { v_uv = a_uv; gl_Position = vec4(0.0, 0.0, texture2D(u_height, a_uv).r, 1.0); }
"""
        self.check(source, "vertex texture fetch")
        self.check(source, "vertex texture fetch", stage="vertex")
        # The same sampling in a fragment shader is what GL 2.1 is for: no violation.
        self.assertEqual([v for v in violations(source, "fragment") if "vertex texture" in v], [])

    def test_a_files_suffix_names_its_stage(self):
        self.assertEqual(gl21_shaders.stage_of(Path("terrain.vert")), "vertex")
        self.assertEqual(gl21_shaders.stage_of(Path("terrain.frag")), "fragment")
        self.assertIsNone(gl21_shaders.stage_of(Path("terrain.glsl")))

    def test_line_numbers_point_at_the_violation(self):
        found = gl21_shaders.check_text(HEADER + "\n\nuniform sampler2DArray atlas;\n")
        self.assertEqual([line for line, _ in found], [4])


class FileTests(unittest.TestCase):
    def test_counts_an_unreadable_file_as_a_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory, "gone.frag")
            found, errors = gl21_shaders.check_files([missing])
            self.assertEqual(found, {})
            self.assertIn(missing, errors)

    def test_reports_every_file_that_breaks_a_rule(self):
        with tempfile.TemporaryDirectory() as directory:
            good = Path(directory, "good.frag")
            good.write_text(HEADER + "void main() { gl_FragColor = vec4(1.0); }\n", encoding="utf-8")
            bad = Path(directory, "bad.frag")
            bad.write_text(HEADER + "uniform sampler2DArray atlas;\n", encoding="utf-8")
            found, errors = gl21_shaders.check_files([good, bad])
            self.assertEqual(errors, {})
            self.assertEqual(list(found), [bad])


if __name__ == "__main__":
    unittest.main()
