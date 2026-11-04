"""CPU-only checks for the legacy GL developer probe's result validation."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "legacy_gl_smoke", Path(__file__).resolve().parents[1] / "tools/legacy_gl_smoke.py"
)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class SmokeValidationTest(unittest.TestCase):
    def test_versions(self):
        self.assertTrue(probe.version_at_least("2.1 Mesa 24.0", 2, 1))
        self.assertTrue(probe.version_at_least("1.20", 1, 20))
        self.assertFalse(probe.version_at_least("2.0", 2, 1))
        self.assertFalse(probe.version_at_least("OpenGL ES 3.0", 2, 1))
        self.assertFalse(probe.version_at_least("", 2, 1))

    def test_pixel_validation(self):
        self.assertTrue(probe.check_pixels([100, 20, 10, 191], [0, 0, 0, 255]))
        self.assertTrue(probe.check_pixels([103, 17, 10, 0], [3, 0, 2, 0]))
        self.assertFalse(probe.check_pixels([104, 20, 10, 255], [0, 0, 0, 255]))
        self.assertFalse(probe.check_pixels([100, 20, 10, 255], [4, 0, 0, 255]))
        self.assertFalse(probe.check_pixels([], []))


if __name__ == "__main__":
    unittest.main()
