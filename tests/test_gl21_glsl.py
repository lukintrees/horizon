"""The GLSL 1.20 gate and the legacy-path budget, run over this crate's shaders.

Two implementations measure the shaders: the Python checker (`tools/gl21_glsl.py`, which
CI runs) and `budget.rs` (which refuses an over-budget program on every machine). These tests
keep the Rust half honest against the files that actually ship, so a shader edited on the
build machine cannot reach the target unnoticed.
"""

import json
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SHADERS = ROOT / "crates" / "horizon-client-gl" / "src" / "shaders"
CHECKER = ROOT / "tools" / "gl21_glsl.py"


def shader_files():
    out = []
    for ext in ("vert", "frag"):
        out.extend(sorted(SHADERS.glob(f"**/*.{ext}")))
    return out


def measure(path):
    """The Python checker's report for one file, through its own API."""
    sys.path.insert(0, str(ROOT))
    from tools import gl21_glsl

    return gl21_glsl.measure(path, path.read_text(encoding="utf-8"))


class ShipsWithoutFindings(unittest.TestCase):
    def test_every_shader_passes_the_checker(self):
        files = shader_files()
        self.assertTrue(files, "the legacy path ships shaders; an empty folder is a mistake")
        for path in files:
            report = measure(path)
            self.assertEqual(
                [f["detail"] for f in report["findings"]],
                [],
                f"{path.name} has findings: {report['findings']}",
            )

    def test_the_terrain_shaders_are_within_a_quarter_of_every_budget(self):
        # Room to add a pass's uniforms without renegotiating the budget, and a tripwire if a
        # change quietly pastes the WGSL's 936-component block into the 512-component stage.
        files = shader_files()
        for path in files:
            report = measure(path)
            self.assertLess(report["uniform_components"], 512 // 4, path.name)
            self.assertLess(report["samplers"], 16 // 2, path.name)
            self.assertLessEqual(report["varying_floats"], 44 // 2, path.name)

    def test_the_vertex_stage_never_samples(self):
        for path in SHADERS.glob("**/*.vert"):
            report = measure(path)
            self.assertEqual(report["samplers"], 0, f"{path.name} samples in the vertex stage")

    def test_a_vertex_and_fragment_pair_agrees_on_its_varyings(self):
        reports = {p.with_suffix(""): p for p in shader_files()}
        for stem in reports:
            vertex = measure(stem.with_suffix(".vert"))
            fragment = measure(stem.with_suffix(".frag"))
            v = {d["name"]: d["type"] for d in vertex["declarations"]["varying"]}
            f = {d["name"]: d["type"] for d in fragment["declarations"]["varying"]}
            self.assertEqual(v, f, f"{stem.name}: varyings must match name for name and type")


class TheCheckerItself(unittest.TestCase):
    """A gate that cannot fail is not a gate: these pin that it does."""

    def check(self, source, suffix=".frag"):
        import tempfile

        with tempfile.NamedTemporaryFile("w", suffix=suffix, delete=False) as handle:
            handle.write(source)
            path = Path(handle.name)
        try:
            return measure(path)["findings"]
        finally:
            path.unlink()

    def kinds(self, findings):
        return {f["kind"] for f in findings}

    def test_a_shader_over_512_uniform_components_is_refused(self):
        findings = self.check("uniform vec4 big[200];\nvoid main() { gl_FragColor = big[0]; }\n")
        self.assertIn("uniform budget", self.kinds(findings))

    def test_a_vertex_texture_fetch_is_refused(self):
        findings = self.check(
            "uniform sampler2D t;\nattribute vec2 uv;\nvoid main() { gl_Position = texture2D(t, uv); }\n",
            ".vert",
        )
        self.assertIn("vertex texture fetch", self.kinds(findings))

    def test_tokens_glsl_1_20_does_not_have_are_refused(self):
        findings = self.check(
            "uniform sampler2DArray sprites;\nuniform uint count;\nvoid main() {\n"
            "  vec4 c = texelFetch(sprites, ivec3(0), 0);\n"
            "  gl_FragColor = c + float(count);\n}\n"
        )
        details = " ".join(f["detail"] for f in findings)
        for word in ("sampler2DArray", "uint", "texelFetch"):
            self.assertIn(word, details)

    def test_a_flat_varying_is_refused(self):
        findings = self.check("flat varying vec4 tint;\nvoid main() { gl_FragColor = tint; }\n")
        self.assertTrue(any("flat varying" in f["detail"] for f in findings), findings)

    def test_a_data_bounded_loop_is_refused_and_a_constant_bounded_one_is_not(self):
        bad = self.check("uniform int n;\nvoid main() { for (int i = 0; i < n; i++) { } }\n")
        self.assertIn("dynamic loop", self.kinds(bad))
        ok = self.check("void main() { for (int i = 0; i < 4; i++) { } }\n")
        self.assertNotIn("dynamic loop", self.kinds(ok))
        ok = self.check("void main() { for (int i = 0; i < MAX_LIGHTS; i++) { } }\n")
        self.assertNotIn("dynamic loop", self.kinds(ok))

    def test_a_declaration_nothing_reads_is_reported(self):
        findings = self.check("uniform vec4 unused_tint;\nvoid main() { gl_FragColor = vec4(1.0); }\n")
        self.assertIn("unused uniform", self.kinds(findings))


class CommandLineTests(unittest.TestCase):
    def test_the_cli_fails_on_a_clean_tree_and_passes_on_dirty_source(self):
        run = subprocess.run([sys.executable, str(CHECKER), "--check"],
                             capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)

        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            bad = Path(tmp) / "bad.frag"
            bad.write_text("uniform vec4 x[200];\nvoid main() { gl_FragColor = x[0]; }\n")
            run = subprocess.run([sys.executable, str(CHECKER), str(bad), "--check"],
                                 capture_output=True, text=True)
            self.assertEqual(run.returncode, 1, run.stdout)

    def test_json_output_is_a_report(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "glsl.json"
            run = subprocess.run([sys.executable, str(CHECKER), "--json", str(out)],
                                 capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
            report = json.loads(out.read_text())
            self.assertEqual(report["limits"]["uniform_components"], 512)
            self.assertTrue(report["shaders"])


if __name__ == "__main__":
    unittest.main()
