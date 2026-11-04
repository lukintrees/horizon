"""Report validation tests; real driver execution lives in legacy_gl_smoke.py."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "legacy_gl_smoke", Path(__file__).resolve().parents[1] / "tools/legacy_gl_smoke.py"
)
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


class LegacyGlSmokeTests(unittest.TestCase):
    def report(self):
        return {
            "status": "passed", "renderer": "llvmpipe (LLVM 15.0.6, 256 bits)",
            "gl_version": "2.0 Mesa 22.3.6", "shader_version": "110",
            "backend_checks": ["backend_rgba_texture", "backend_texture_update", "backend_depth",
                               "backend_cutout", "backend_ordered_blend_no_depth_write",
                               "backend_overlay_scissor_reset", "backend_lightmap", "backend_fog",
                               "backend_large_coordinates", "backend_validation_budget",
                               "backend_context_isolation", "backend_resource_reclamation",
                               "backend_terrain_atlas_contract", "backend_resident_sections", "backend_gui_compositor"],
            "checks": ["shader_compile_link", "vbo_ebo_draw", "clear_readback",
                       "triangle_readback", "depth_readback", "invalid_shader_rejected",
                       "no_gl_error"],
        }

    def test_valid_gl20_readback_report(self):
        smoke.validate_report(self.report(), "2.0", "110")

    def test_unrestricted_control_removes_inherited_overrides(self):
        base = {"MESA_GL_VERSION_OVERRIDE": "4.6", "MESA_GLSL_VERSION_OVERRIDE": "460",
                "LD_LIBRARY_PATH": "/test/mesa"}
        env = smoke.mode_env(base, None, "110")
        self.assertNotIn("MESA_GL_VERSION_OVERRIDE", env)
        self.assertNotIn("MESA_GLSL_VERSION_OVERRIDE", env)
        self.assertEqual(env["GALLIUM_DRIVER"], "llvmpipe")
        self.assertEqual(env["LD_LIBRARY_PATH"], "/test/mesa")
        self.assertEqual(base["MESA_GL_VERSION_OVERRIDE"], "4.6")

    def test_baseline_modes_replace_inherited_modern_settings(self):
        env = smoke.mode_env({"MESA_GL_VERSION_OVERRIDE": "4.6"}, "2.1", "120")
        self.assertEqual(env["MESA_GL_VERSION_OVERRIDE"], "2.1")
        self.assertEqual(env["MESA_GLSL_VERSION_OVERRIDE"], "120")
        self.assertEqual(env["EGL_PLATFORM"], "surfaceless")

    def test_gl20_uses_glsl110_source_with_mesa_supported_version_ceiling(self):
        env = smoke.mode_env({}, "2.0", "110")
        self.assertEqual(env["MESA_GL_VERSION_OVERRIDE"], "2.0")
        self.assertEqual(env["MESA_GLSL_VERSION_OVERRIDE"], "120")

    def test_old_triangle_only_report_is_not_backend_verification(self):
        data = self.report()
        del data["backend_checks"]
        with self.assertRaises(ValueError):
            smoke.validate_report(data, "2.0", "110")

    def test_not_a_pass_without_real_driver_and_checks(self):
        for key, value in [("status", "failed"), ("renderer", "softpipe"),
                           ("renderer", "Radeon X1650"), ("checks", []),
                           ("gl_version", "4.5 Mesa 22.3.6"), ("shader_version", "120")]:
            with self.subTest(key=key, value=value):
                data = self.report()
                data[key] = value
                with self.assertRaises(ValueError):
                    smoke.validate_report(data, "2.0", "110")


if __name__ == "__main__":
    unittest.main()
