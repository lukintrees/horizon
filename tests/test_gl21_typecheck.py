"""Tests for tools/gl21_typecheck.py: the offline stand-in for `cargo check`.

The checker exists because the machine the GL 2.1 crate was written on has no rustc
(docs/gl21-dev-env.md). It is a lint over tree-sitter, so the way to trust it is to hold it
against fixtures it must flag and fixtures it must not: a false positive would teach the
reader to ignore it, and a false negative would let a wrong glow call reach CI.
"""

import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / "tools" / "gl21_typecheck.py"
CRATE = ROOT / "crates" / "horizon-client-gl"


def have_tree_sitter() -> bool:
    """As the tool itself finds them: the gates can live in .local/gl21-env, not in the
    interpreter's site-packages (tools/gl21_preflight.sh --install)."""
    import os

    env = dict(os.environ)
    env["PYTHONPATH"] = os.pathsep.join(
        [str(ROOT / ".local" / "gl21-env")]
        + ([env["PYTHONPATH"]] if env.get("PYTHONPATH") else [])
    )
    return (
        subprocess.run(
            [sys.executable, "-c", "import tree_sitter, tree_sitter_rust"],
            env=env,
            capture_output=True,
        ).returncode
        == 0
    )


def run(*args: str) -> subprocess.CompletedProcess:
    # The gates live in .local/gl21-env when tools/gl21_preflight.sh --install put them there;
    # CI installs them into the runner's Python instead.
    env_path = [str(ROOT / ".local" / "gl21-env"), str(Path(__file__).resolve().parents[0])]
    import os

    env = dict(os.environ)
    env["PYTHONPATH"] = os.pathsep.join(
        env_path + ([env["PYTHONPATH"]] if env.get("PYTHONPATH") else [])
    )
    return subprocess.run(
        [sys.executable, str(TOOL), *args],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        timeout=300,
    )


@unittest.skipUnless(have_tree_sitter(), "the tree-sitter Rust grammar is not installed")
class TypecheckTests(unittest.TestCase):
    def test_the_checker_flags_what_it_must_and_nothing_else(self):
        result = run("--self-test")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("0 false positives", result.stdout)

    def test_the_crate_is_checked_against_a_glow_checkout_when_one_is_present(self):
        # A vendored coupon of glow is not in the repository (it would be a copy of a
        # dependency), so this reads one only when the developer's environment has it:
        # tools/gl21_preflight.sh --glow puts it in .local/glow.
        glow = ROOT / ".local" / "glow"
        if not (glow / "src" / "lib.rs").is_file():
            self.skipTest("no glow sources in .local/glow (tools/gl21_preflight.sh --glow)")
        result = run("--glow", str(glow), "--verbose")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("0 findings", result.stdout)

    def test_a_missing_glow_directory_is_a_usage_error_not_a_pass(self):
        result = run("--glow", str(ROOT / "does-not-exist"))
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("no glow sources", result.stderr)

    def test_no_glow_option_at_all_says_which_two_ways_there_are(self):
        result = run()
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("--glow-cargo", result.stderr)
        self.assertIn("--self-test", result.stderr)


class LayoutTests(unittest.TestCase):
    """The tests that do not need tree-sitter: the crate's own shape."""

    def test_the_crate_is_a_workspace_member(self):
        cargo = (ROOT / "Cargo.toml").read_text()
        self.assertIn('"crates/horizon-client-gl"', cargo)

    def test_the_shipped_shaders_are_the_ones_the_gate_measures(self):
        shaders = CRATE / "src" / "shaders" / "terrain"
        for name in ("terrain.vert", "terrain.frag"):
            self.assertTrue((shaders / name).is_file(), name)
        info = (CRATE / "src" / "bin" / "horizon-gl21-info.rs").read_text()
        # The binary embeds them, so `--check-shaders` measures the files that ship and not a
        # second copy that could drift.
        self.assertIn('include_str!("../shaders/terrain/terrain.vert")', info)
        self.assertIn('include_str!("../shaders/terrain/terrain.frag")', info)

    def test_ci_compiles_the_crate(self):
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
        self.assertIn("\n  gl21:", workflow)
        self.assertIn("cargo +stable test -p horizon-client-gl", workflow)
        self.assertIn("cargo +stable clippy -p horizon-client-gl", workflow)
        self.assertIn("gl21_typecheck.py --glow-cargo", workflow)


if __name__ == "__main__":
    unittest.main()
