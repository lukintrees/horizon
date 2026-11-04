import re
import tomllib
import unittest
from pathlib import Path

from tools import gl21_caps as caps

ROOT = Path(__file__).resolve().parents[1]
GAME = ROOT / "crates/horizon-client/src/game.rs"
GAME_DIR = ROOT / "crates/horizon-client/src/game"
GFX21 = ROOT / "crates/horizon-client-render/src/gfx21"
SHADER_DIR = GFX21 / "shaders"
CONTRACT = GFX21 / "CONTRACT.md"
LIB_RS = ROOT / "crates/horizon-client-render/src/lib.rs"
CARGO_RENDER = ROOT / "crates/horizon-client-render/Cargo.toml"
CARGO_CLIENT = ROOT / "crates/horizon-client/Cargo.toml"
WORKFLOW = ROOT / ".github/workflows/arena-legacy-gl.yml"
EXAMPLE = ROOT / "crates/horizon-client-render/examples/gl21_clear.rs"
NOTICES = ROOT / "THIRD_PARTY_NOTICES.md"

# These are Option<Renderer> methods, not members of Renderer itself.
OPTION_METHODS = {
    "as_ref", "as_mut", "is_some", "is_none", "is_some_and", "take",
    "and_then", "map", "unwrap", "as_deref",
}

# Every Rust file in the additive legacy scaffold must stay free of the modern
# renderer stack. The real GL loader lives in the gated example instead.
GFX21_SOURCES = (
    "caps.rs",
    "mod.rs",
    "context.rs",
    "program.rs",
    "state.rs",
    "smoke.rs",
)
SHADER_SOURCES = (
    "smoke_120.vert",
    "smoke_120.frag",
    "smoke_110.vert",
    "smoke_110.frag",
)


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
        for name in ("context", "program", "state", "smoke"):
            self.assertIn(f"pub mod {name};", module)
        for name in GFX21_SOURCES:
            source = (GFX21 / name).read_text(encoding="utf-8")
            self.assertNotRegex(source, r"\bwgpu::", f"{name} references wgpu")
            self.assertNotRegex(source, r"\bnaga::", f"{name} references naga")
            self.assertNotRegex(source, r"\bunsafe\b", f"{name} uses unsafe")

    def test_shader_sources_stay_in_the_110_120_dialect(self):
        for name in SHADER_SOURCES:
            text = (SHADER_DIR / name).read_text(encoding="utf-8")
            self.assertTrue(
                text.startswith("#version 110") or text.startswith("#version 120"),
                f"{name} has no GL2 #version",
            )
            for forbidden in (
                "#version 330",
                "layout(",
                "in vec2",
                "out vec4",
                "texelFetch",
                "gl_VertexID",
                "gl_InstanceID",
            ):
                self.assertNotIn(forbidden, text, f"{name} contains {forbidden}")
            if name.endswith(".vert"):
                self.assertIn("attribute", text)
                self.assertIn("gl_Position", text)
            else:
                self.assertIn("gl_FragColor", text)

    def test_legacy_gl_feature_is_wired_in_both_crates(self):
        render = tomllib.loads(CARGO_RENDER.read_text(encoding="utf-8"))
        client = tomllib.loads(CARGO_CLIENT.read_text(encoding="utf-8"))
        self.assertEqual(render["features"]["default"], ["modern-gl"])
        self.assertIn("modern-gl", render["features"]["legacy-gl"])
        for dep in ("wgpu", "naga", "pollster"):
            self.assertIn(f"dep:{dep}", render["features"]["modern-gl"])
            self.assertTrue(render["dependencies"][dep].get("optional"), f"{dep} is not optional")
        for dep in ("glow", "glutin", "glutin-winit", "raw-window-handle"):
            self.assertIn(f"dep:{dep}", render["features"]["legacy-gl"])
            self.assertTrue(render["dependencies"][dep].get("optional"), f"{dep} is not optional")
        examples = render.get("example", [])
        self.assertTrue(
            any(
                item.get("name") == "gl21_clear"
                and item.get("required-features") == ["legacy-gl"]
                for item in examples
            ),
            "gl21_clear example is not gated behind legacy-gl",
        )
        self.assertTrue(EXAMPLE.exists())
        self.assertIn(
            "horizon-client-render/legacy-gl",
            client["features"]["legacy-gl"],
        )

    def test_new_gl_dependencies_have_notices(self):
        notices = NOTICES.read_text(encoding="utf-8")
        for name in ("glow", "glutin", "glutin-winit"):
            self.assertIn(name, notices, f"{name} is missing from THIRD_PARTY_NOTICES.md")

    def test_crate_root_alias_is_cfg_gated(self):
        lib = LIB_RS.read_text(encoding="utf-8")
        self.assertIn('#[cfg(feature = "legacy-gl")]', lib)
        self.assertIn("pub type Renderer = gfx21::Renderer;", lib)
        self.assertIn("pub use gfx21::Frame;", lib)
        contract = CONTRACT.read_text(encoding="utf-8")
        self.assertIn("PR 2b", contract)
        self.assertIn("PR 3 stage 2", contract)
        self.assertIn("PR 4 stage 1", contract)

    def test_own_branch_workflow_runs_the_gate_and_gl_smoke(self):
        # Our own workflow only; existing CI files are deliberately untouched.
        self.assertTrue(WORKFLOW.exists())
        source = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("arena/81644160-horizon", source)
        self.assertIn(
            "cargo check -p horizon-client --features legacy-gl --no-default-features",
            source,
        )
        self.assertIn("gl21_clear", source)
        self.assertIn("xvfb-run", source)
        self.assertIn("HORIZON_GLSL_DIALECT", source)
        self.assertIn("HORIZON_GLSL_DIALECT", EXAMPLE.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
