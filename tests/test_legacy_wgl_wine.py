"""Report validation and isolation rules for tools/legacy_wgl_wine.py.

The real Wine run happens in the harness; these tests feed it fabricated JSON so
the pass/fail rules and the environment isolation are pinned without Wine.
"""
import importlib.util
from pathlib import Path
import unittest

HARNESS = Path(__file__).resolve().parents[1] / "tools/legacy_wgl_wine.py"
spec = importlib.util.spec_from_file_location("legacy_wgl_wine", HARNESS)
wine = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wine)


def probe_report(**overrides):
    """A genuine GL 2.1 WGL report, as the probe prints it under Wine."""
    data = {
        "status": "passed",
        "buffer_swap": True,
        "pointer_bits": 64,
        "target_os": "windows",
        "context": "WGL desktop OpenGL window",
        "checks": sorted(wine.REQUIRED_CHECKS),
        "backend_checks": sorted(wine.WGL_REQUIRED_BACKEND_CHECKS),
        "vendor": "Mesa",
        "renderer": "llvmpipe (LLVM 23.1.4, 256 bits)",
        "gl_version": "2.1 Mesa 26.2.6-arch3.1",
        "glsl_version": "1.20",
        "shader_version": "120",
    }
    data.update(overrides)
    return data


class ValidateReportTests(unittest.TestCase):
    def test_a_good_gl21_wgl_report_passes(self):
        wine.validate_report(probe_report(), "2.1", "120", pointer_bits=64)

    def test_a_gl20_glsl110_report_passes(self):
        wine.validate_report(
            probe_report(gl_version="2.0 Mesa", shader_version="110"),
            "2.0", "110",
        )

    def test_a_modern_gl_context_is_not_a_pass(self):
        # Wine without the Mesa overrides forwards the host's GL 4.6; that is not
        # the legacy backend's contract, so it must fail rather than pass quietly.
        with self.assertRaises(ValueError):
            wine.validate_report(
                probe_report(gl_version="4.6 (Compatibility Profile) Mesa",
                             glsl_version="4.60"),
                None, "120",
            )

    def test_a_wgpu_or_non_windows_report_is_not_a_pass(self):
        for key, value in [
            ("target_os", "linux"),
            ("context", "EGL pbuffer"),
            ("status", "failed"),
            ("buffer_swap", False),
        ]:
            with self.subTest(key=key, value=value):
                with self.assertRaises(ValueError):
                    wine.validate_report(probe_report(**{key: value}), "2.1", "120")

    def test_missing_identity_strings_fail(self):
        for key, value in [("vendor", ""), ("renderer", "")]:
            with self.subTest(key=key):
                with self.assertRaises(ValueError):
                    wine.validate_report(probe_report(**{key: value}), "2.1", "120")

    def test_a_report_missing_backend_outcomes_fails(self):
        data = probe_report()
        data["backend_checks"].remove("backend_world_weather")
        with self.assertRaises(ValueError):
            wine.validate_report(data, "2.1", "120")

    def test_the_only_skipped_outcome_is_context_isolation(self):
        # The probe creates its second, unrelated context only for the offscreen
        # backends. If a future probe emits it on WGL, it is no longer skipped and
        # an extra outcome does not make a report fail; if a new outcome appears,
        # the runner's set must grow with it.
        self.assertEqual(wine.WGL_SKIPPED_BACKEND_CHECKS, {"backend_context_isolation"})
        self.assertEqual(
            wine.WGL_REQUIRED_BACKEND_CHECKS | wine.WGL_SKIPPED_BACKEND_CHECKS,
            wine.REQUIRED_BACKEND_CHECKS,
        )
        wine.validate_report(probe_report(), "2.1", "120")

    def test_a_triangle_only_report_fails(self):
        data = probe_report()
        del data["backend_checks"]
        with self.assertRaises(ValueError):
            wine.validate_report(data, "2.1", "120")

    def test_the_requested_version_and_pointer_width_are_enforced(self):
        with self.assertRaises(ValueError):
            wine.validate_report(probe_report(), "2.0", "120")
        with self.assertRaises(ValueError):
            wine.validate_report(probe_report(pointer_bits=32), "2.1", "120",
                                 pointer_bits=64)

    def test_a_glsl_120_ceiling_still_accepts_110_source(self):
        # The GL2.0 case compiles #version 110 source while Mesa reports 1.20:
        # the check is on the reported language version, not the source string.
        wine.validate_report(probe_report(glsl_version="1.20", shader_version="110"),
                             "2.1", "110")


class EnvironmentTests(unittest.TestCase):
    def test_the_prefix_owns_every_writable_folder(self):
        env = wine.isolated_env(Path("/scratch/prefix"))
        for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME",
                    "XDG_CONFIG_HOME", "WINEPREFIX"):
            with self.subTest(key=key):
                self.assertTrue(env[key].startswith("/scratch/prefix"), env[key])
        self.assertEqual(env["HORIZON_LAUNCHER_NO_SELF_UPDATE"], "1")
        self.assertEqual(env["HORIZON_NO_AUDIO"], "1")

    def test_mesa_overrides_replace_inherited_ones(self):
        env = wine.mesa_env({"MESA_GL_VERSION_OVERRIDE": "4.6",
                             "MESA_GLSL_VERSION_OVERRIDE": "460"}, "2.1", "120")
        self.assertEqual(env["MESA_GL_VERSION_OVERRIDE"], "2.1")
        self.assertEqual(env["MESA_GLSL_VERSION_OVERRIDE"], "120")
        self.assertEqual(env["GALLIUM_DRIVER"], "llvmpipe")

    def test_a_gl20_case_uses_a_mesa_supported_language_ceiling(self):
        env = wine.mesa_env({}, "2.0", "110")
        self.assertEqual(env["MESA_GL_VERSION_OVERRIDE"], "2.0")
        self.assertEqual(env["MESA_GLSL_VERSION_OVERRIDE"], "120")

    def test_the_win7_registry_pin_uses_the_key_wine_honours(self):
        # Wine 11 keeps the reported Windows version under HKCU\Software\Wine as
        # "Version"; `cmd /c ver` then reports 6.1.7601. Pin the exact commands so
        # a future Wine that changes the key is caught rather than silently ignored.
        add, query = wine.version_pin_commands()
        self.assertEqual(add[:4], ["wine", "reg", "add", r"HKCU\Software\Wine"])
        self.assertIn("Version", add)
        self.assertIn("win7", add)
        self.assertEqual(query[:4], ["wine", "reg", "query", r"HKCU\Software\Wine"])


if __name__ == "__main__":
    unittest.main()
