import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("inventory", Path(__file__).resolve().parents[1] / "tools/ci/legacy_renderer_inventory.py")
inventory = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inventory)


class InventoryTests(unittest.TestCase):
    def test_marks_alias_candidates_without_claiming_name_resolution(self):
        code = "// renderer.device\nself.renderer.as_mut()\nr.device.create_buffer()\nold.hud\nother.device\nRenderer::new()\n"
        rows = inventory.scan(code, {"device", "hud"})
        self.assertEqual([row["line"] for row in rows], [2, 3, 4, 6])
        self.assertEqual(rows[1]["kinds"], ["candidate-member-access"])
