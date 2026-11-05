"""Tests for tools/legacy_material_audit.py (M01a).

The real-repository test ties docs/design/legacy-material-audit.md to the sources at
baseline 453966c. The synthetic tests check the checker itself on tiny Rust-like
texts whose expected answers are written here, not computed by the checker.
"""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("legacy_material_audit", ROOT / "tools/legacy_material_audit.py")
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)

SYNTHETIC_ENTITY = """
fn runs_view() {
    if additive && !matches!(code, 0 | 2 | 3 | 37) {
    let overlay = match code {
        2 | 3 | 37 if additive => Overlay::None,
        0 | 7 | 13 => Overlay::None, // 99 would be a comment, not an arm
        1 | 8 => Overlay::Hurt,
        16..=31 => Overlay::White((code - 16) as u8),
        _ => return Err(Error::UnsupportedMaterial(code)),
    };
    let glint = if matches!(code, 7 | 8) {
        let state = options.glint.ok_or(Error::UnsupportedMaterial(code))?;
    }
}
"""

SYNTHETIC_FRAME = """
fn unsupported(frame: &gfx::Frame<'_>) -> Result<(), Error> {
    for (requested, name) in [
        (frame.title, "title panorama"),
        (frame.clouds.is_some(), "clouds"),
    ] {
        if requested { return Err(Error::Unsupported(name)); }
    }
    Ok(())
}
"""


def synthetic_expectations(**overrides):
    base = {
        "entity_rs": "entity.rs",
        "frame_rs": "frame.rs",
        "const_files": ["consts.rs"],
        "legacy_entity": {"nonadditive": [0, 1, 7, 8, 13] + list(range(16, 32)), "additive": [0, 2, 3, 37], "glint_required": [7, 8]},
        "frame_blockers": ["title panorama", "clouds"],
        "const_values": {"DRAGON_RAYS_OVERLAY": 37},
        "producers": [],
        "present": [],
        "absent": [],
    }
    base.update(overrides)
    return base


class SyntheticRoot:
    def __init__(self, files):
        self.dir = tempfile.TemporaryDirectory()
        for name, text in files.items():
            (Path(self.dir.name) / name).write_text(text, encoding="utf-8")

    def run(self, expectations):
        results = audit.Audit(self.dir.name, expectations).run()
        return {r["id"]: r["ok"] for r in results}

    def close(self):
        self.dir.cleanup()


class ParserTests(unittest.TestCase):
    def test_arms_skip_comments_and_expand_ranges(self):
        sets = audit.legacy_entity_sets(SYNTHETIC_ENTITY)
        self.assertEqual(sets["nonadditive"], [0, 1, 7, 8, 13] + list(range(16, 32)))
        self.assertEqual(sets["additive"], [0, 2, 3, 37])
        self.assertEqual(sets["glint_required"], [7, 8])

    def test_frame_blockers_are_the_quoted_names_of_unsupported(self):
        self.assertEqual(audit.parse_frame_blockers(SYNTHETIC_FRAME), ["clouds", "title panorama"])

    def test_constants_accept_u8_u16_u32_only(self):
        table = audit.parse_consts(["pub const A: u32 = 34;\npub const B: u16 = 1;\npub const C: i32 = 9;"])
        self.assertEqual(table, {"A": 34, "B": 1})


class CheckerTests(unittest.TestCase):
    def test_matching_synthetic_sources_pass(self):
        root = SyntheticRoot({"entity.rs": SYNTHETIC_ENTITY, "frame.rs": SYNTHETIC_FRAME,
                              "consts.rs": "pub const DRAGON_RAYS_OVERLAY: u32 = 37;"})
        try:
            results = root.run(synthetic_expectations())
        finally:
            root.close()
        self.assertTrue(all(results.values()), results)

    def test_wrong_accept_set_fails(self):
        root = SyntheticRoot({"entity.rs": SYNTHETIC_ENTITY, "frame.rs": SYNTHETIC_FRAME,
                              "consts.rs": "pub const DRAGON_RAYS_OVERLAY: u32 = 37;"})
        try:
            expectations = synthetic_expectations()
            expectations["legacy_entity"]["nonadditive"] = [0, 1, 7, 8, 13]  # forgot 16..31
            results = root.run(expectations)
        finally:
            root.close()
        self.assertFalse(results["legacy.nonadditive_codes"])
        self.assertTrue(results["legacy.additive_codes"])

    def test_missing_producer_snippet_fails(self):
        root = SyntheticRoot({"entity.rs": SYNTHETIC_ENTITY, "frame.rs": SYNTHETIC_FRAME,
                              "consts.rs": "pub const DRAGON_RAYS_OVERLAY: u32 = 37;",
                              "game.rs": "let x = 1;"})
        try:
            expectations = synthetic_expectations(
                producers=[{"id": "p", "category": "c", "file": "game.rs", "snippet": "layer: 11,"}])
            results = root.run(expectations)
        finally:
            root.close()
        self.assertFalse(results["producer.p"])

    def test_absent_regex_catches_a_new_test_mentioning_the_code(self):
        text = "#[test] fn t() { for layer in [2, 11, 37] { } }\n"
        root = SyntheticRoot({"entity.rs": SYNTHETIC_ENTITY, "frame.rs": SYNTHETIC_FRAME,
                              "consts.rs": "pub const DRAGON_RAYS_OVERLAY: u32 = 37;",
                              "tests.rs": text})
        try:
            expectations = synthetic_expectations(
                absent=[{"id": "no11", "file": "tests.rs", "regex": "for layer in \\[[^\\]]*\\b1[12]\\b"}])
            results = root.run(expectations)
        finally:
            root.close()
        self.assertFalse(results["absent.no11"])

    def test_snippet_inside_a_line_comment_does_not_count(self):
        root = SyntheticRoot({"entity.rs": SYNTHETIC_ENTITY, "frame.rs": SYNTHETIC_FRAME,
                              "consts.rs": "pub const DRAGON_RAYS_OVERLAY: u32 = 37;",
                              "game.rs": "// layer: 11,\nlet y = 2;"})
        try:
            expectations = synthetic_expectations(
                producers=[{"id": "p", "category": "c", "file": "game.rs", "snippet": "layer: 11,"}])
            results = root.run(expectations)
        finally:
            root.close()
        self.assertFalse(results["producer.p"])


class RepositoryTests(unittest.TestCase):
    def test_catalog_sources_match_expectations(self):
        expectations = json.loads((ROOT / "tests/fixtures/legacy_material_audit/expectations.json").read_text(encoding="utf-8"))
        results = audit.Audit(ROOT, expectations).run()
        failed = [r for r in results if not r["ok"]]
        self.assertEqual(failed, [])
        self.assertGreater(len(results), 60)

    def test_report_is_written_with_static_limits(self):
        with tempfile.TemporaryDirectory() as tmp:
            report_path = Path(tmp) / "report.json"
            code = audit.main(["--report", str(report_path)])
            report = json.loads(report_path.read_text(encoding="utf-8"))
        self.assertEqual(code, 0)
        self.assertEqual(report["status"], "passed")
        self.assertIn("no Rust build", report["limits"])


if __name__ == "__main__":
    unittest.main()
