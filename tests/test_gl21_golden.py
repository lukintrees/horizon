import tempfile
import unittest
from pathlib import Path

from tools import legacy_gl_golden as golden

ROOT = Path(__file__).resolve().parents[1]
RGBA_PATH = ROOT / "benchmarks/graphics/legacy-gl-scene.rgba"
PNG_PATH = ROOT / "benchmarks/graphics/legacy-gl-scene.png"


def pixel(rgba, x, y):
    at = (y * golden.WIDTH + x) * 4
    return tuple(rgba[at:at + 4])


class GoldenFrameTests(unittest.TestCase):
    def test_committed_fixtures_round_trip(self):
        rgba = RGBA_PATH.read_bytes()
        self.assertEqual(len(rgba), golden.WIDTH * golden.HEIGHT * 4)
        self.assertTrue(PNG_PATH.exists())
        width, height, decoded = golden.read_png_rgba(PNG_PATH)
        self.assertEqual((width, height), (80, 60))
        self.assertEqual(decoded, rgba)

    def test_known_pixels_have_independent_expected_values(self):
        rgba = RGBA_PATH.read_bytes()
        expected = {
            (0, 0): (24, 36, 56, 255),          # sky top
            (0, 59): (72, 96, 120, 255),        # sky bottom
            (8, 20): (114, 94, 74, 255),        # opaque quad with fog=51
            (20, 16): (36, 52, 73, 255),        # cutout checker hole, sky at y=16
            (21, 16): (153, 145, 122, 255),     # cutout quad with fog=76
            (5, 5): (220, 220, 220, 255),       # HUD
            (5, 56): (240, 200, 80, 255),       # line
            (39, 59): (72, 96, 120, 255),       # untouched sky below the line
        }
        for (x, y), want in expected.items():
            with self.subTest(x=x, y=y):
                self.assertEqual(pixel(rgba, x, y), want)

    def test_integer_helpers_have_independent_expected_values(self):
        self.assertEqual(golden.apply_fog((120, 90, 60, 255), 51), (114, 94, 74, 255))
        self.assertEqual(golden.blend_channel(255, 0, 128), 127)
        self.assertEqual(golden.sky_pixel(0), (24, 36, 56))
        self.assertEqual(golden.sky_pixel(59), (72, 96, 120))

    def test_check_accepts_the_committed_frame(self):
        self.assertEqual(golden.check(str(RGBA_PATH)), 0)

    def test_check_rejects_a_single_wrong_byte(self):
        rgba = bytearray(RGBA_PATH.read_bytes())
        rgba[0] ^= 0xFF
        with tempfile.NamedTemporaryFile(delete=False) as handle:
            handle.write(rgba)
            path = handle.name
        try:
            with self.assertRaises(ValueError):
                golden.check(path)
        finally:
            Path(path).unlink(missing_ok=True)


if __name__ == "__main__":
    unittest.main()
