import re
import tomllib
import unittest
from pathlib import Path

from tools import gl21_caps as caps

ROOT = Path(__file__).resolve().parents[1]
GAME = ROOT / "crates/horizon-client/src/game.rs"
GAME_DIR = ROOT / "crates/horizon-client/src/game"
GFX21 = ROOT / "crates/horizon-client-render/src/gfx21"
CONTRACT = GFX21 / "CONTRACT.md"
LIB_RS = ROOT / "crates/horizon-client-render/src/lib.rs"
CARGO_RENDER = ROOT / "crates/horizon-client-render/Cargo.toml"
CARGO_CLIENT = ROOT / "crates/horizon-client/Cargo.toml"
WORKFLOW = ROOT / ".github/workflows/arena-legacy-gl.yml"

# These are Option<Renderer> methods, not members of Renderer itself.
OPTION_METHODS = {
    "as_ref", "as_mut", "is_some", "is_none", "is_some_and", "take",
    "and_then", "map", "unwrap", "as_deref",
}

# Every Rust file in the additive legacy scaffold must stay free of the modern
# renderer stack; the real GL loader arrives in a later PR.
GFX21_SOURCES = ("caps.rs", "mod.rs", "context.rs", "state.rs", "smoke.rs")


def game_sources():
    return "\n".join(
        path.read_text(encoding="utf-8", errors="replace")
        for path in [GAME, *sorted(GAME_DIR.glob("*.rs"))]
    )


class ContractTests(unittest.TestCase):
    def test_contract_lists_every_renderer_access(self):
        names = set(re.findall(r"\brenderer\.([A-Za-z_][A-Za-z0-9_]*)", game_sources()))
        names -= OPTION_METHODS
        contract = CONTRACT.read_text(encoding="utf-8")
        missing = sorted(name for name in names if f"`{name}`" not in contract)
        self.assertEqual(missing, [], f"CONTRACT.md is missing: {missing}")

    def test_contract_lists_associated_functions(self):
        names = set(re.findall(r"\bRenderer::([A-Za-z_][A-Za-z0-9_]*)", game_sources()))
        contract = CONTRACT.read_text(encoding="utf-8")
        self.assertTrue(names)
        missing = sorted(name for name in names if f"`{name}`" not in contract)
        self.assertEqual(missing, [], f"CONTRACT.md is missing: {missing}")

    def test_python_and_rust_required_constants_agree(self):
        rust = (GFX21 / "caps.rs").read_text(encoding="utf-8")
        for name in (*caps.REQUIRED_BASE, *caps.REQUIRED_FBO, caps.FBO_REQUIREMENT):
            self.assertIn(name, rust, f"caps.rs is missing {name}")
        for name in caps.OPTIONAL:
            self.assertIn(name, rust, f"caps.rs is missing optional {name}")
        for name in caps.EXPECTED_LIMITS:
            self.assertIn(name, rust, f"caps.rs is missing limit {name}")

    def test_scaffold_is_additive_and_wgpu_free(self):
        self.assertIn("pub mod gfx21;", LIB_RS.read_text(encoding="utf-8"))
        module = (GFX21 / "mod.rs").read_text(encoding="utf-8")
        for name in ("context", "state", "smoke"):
            self.assertIn(f"pub mod {name};", module)
        for name in GFX21_SOURCES:
            source = (GFX21 / name).read_text(encoding="utf-8")
            self.assertNotRegex(source, r"\bwgpu::", f"{name} references wgpu")
            self.assertNotRegex(source, r"\bnaga::", f"{name} references naga")
            self.assertNotRegex(source, r"\bunsafe\b", f"{name} uses unsafe")

    def test_legacy_gl_feature_is_wired_in_both_crates(self):
        render = tomllib.loads(CARGO_RENDER.read_text(encoding="utf-8"))
        client = tomllib.loads(CARGO_CLIENT.read_text(encoding="utf-8"))
        self.assertEqual(render["features"]["default"], ["modern-gl"])
        self.assertIn("modern-gl", render["features"]["legacy-gl"])
        for dep in ("wgpu", "naga", "pollster"):
            self.assertIn(f"dep:{dep}", render["features"]["modern-gl"])
            self.assertTrue(render["dependencies"][dep].get("optional"), f"{dep} is not optional")
        self.assertIn(
            "horizon-client-render/legacy-gl",
            client["features"]["legacy-gl"],
        )

    def test_crate_root_alias_is_cfg_gated(self):
        lib = LIB_RS.read_text(encoding="utf-8")
        self.assertIn('#[cfg(feature = "legacy-gl")]', lib)
        self.assertIn("pub type Renderer = gfx21::Renderer;", lib)
        self.assertIn("pub use gfx21::Frame;", lib)
        contract = CONTRACT.read_text(encoding="utf-8")
        self.assertIn("PR 2b", contract)

    def test_own_branch_workflow_runs_the_gate(self):
        # Our own workflow only; existing CI files are deliberately untouched.
        self.assertTrue(WORKFLOW.exists())
        source = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("arena/81644160-horizon", source)
        self.assertIn(
            "cargo check -p horizon-client --features legacy-gl --no-default-features",
            source,
        )


if __name__ == "__main__":
    unittest.main()
