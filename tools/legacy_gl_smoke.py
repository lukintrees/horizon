#!/usr/bin/env python3
"""Build and execute the independent desktop GL probe against Linux llvmpipe.

No Minecraft resources or player data are used. Reports/builds go under target/.
Mesa and a Rust toolchain must already be installed; this never downloads them.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
REQUIRED_CHECKS = {
    "shader_compile_link", "vbo_ebo_draw", "clear_readback", "triangle_readback",
    "depth_readback", "invalid_shader_rejected", "no_gl_error",
}
REQUIRED_BACKEND_CHECKS = {
    "backend_rgba_texture", "backend_texture_update", "backend_depth", "backend_cutout",
    "backend_ordered_blend_no_depth_write", "backend_overlay_scissor_reset",
    "backend_lightmap", "backend_fog", "backend_large_coordinates", "backend_validation_budget",
    "backend_context_isolation", "backend_resource_reclamation", "backend_terrain_atlas_contract",
    "backend_resident_sections", "backend_gui_compositor", "backend_paged_gui", "backend_paged_entities", "backend_entity_layers", "backend_entity_glint", "backend_gui_glint",
}
MODES = (("gl20", "2.0", "110"), ("gl21", "2.1", "120"), ("native", None, "110"))


def mode_env(base: dict[str, str], gl: str | None, glsl: str) -> dict[str, str]:
    env = base.copy()
    env.update(EGL_PLATFORM="surfaceless", LIBGL_ALWAYS_SOFTWARE="1",
               GALLIUM_DRIVER="llvmpipe", MESA_SHADER_CACHE_DISABLE="true")
    for key in ("MESA_GL_VERSION_OVERRIDE", "MESA_GLSL_VERSION_OVERRIDE"):
        env.pop(key, None)
    if gl is not None:
        # Mesa 23.1 rejects the *reported* language version 110. Restrict to
        # 120, while the GL2.0 case still compiles source with #version 110.
        env.update(MESA_GL_VERSION_OVERRIDE=gl, MESA_GLSL_VERSION_OVERRIDE=str(max(120, int(glsl))))
    return env


def validate_report(data: dict, gl: str | None, glsl: str) -> None:
    if data.get("status") != "passed":
        raise ValueError("probe did not report success")
    if "llvmpipe" not in data.get("renderer", "").lower():
        raise ValueError("renderer is not llvmpipe")
    if not REQUIRED_CHECKS.issubset(data.get("checks", [])):
        raise ValueError("probe omitted mandatory draw/readback checks")
    if not REQUIRED_BACKEND_CHECKS.issubset(data.get("backend_checks", [])):
        raise ValueError("probe omitted reusable-backend rendering checks")
    version = data.get("gl_version", "").split()
    if gl is not None and (not version or version[0] != gl):
        raise ValueError(f"probe did not run GL {gl}")
    if data.get("shader_version") != glsl:
        raise ValueError(f"probe did not compile GLSL {glsl}")


def run_case(binary: Path, gl: str | None, glsl: str, timeout: float, context: str) -> dict:
    args = [str(binary), "--context", context, "--require-llvmpipe", "--shader-version", glsl]
    if gl:
        args += ["--expect-gl", gl]
    try:
        result = subprocess.run(args, env=mode_env(dict(os.environ), gl, glsl),
                                text=True, capture_output=True, timeout=timeout)
        if result.returncode:
            return {"status": "failed", "returncode": result.returncode,
                    "stderr": result.stderr, "stdout": result.stdout}
        data = json.loads(result.stdout)
        validate_report(data, gl, glsl)
        return {"status": "passed", "probe": data, "stderr": result.stderr}
    except (OSError, subprocess.TimeoutExpired, ValueError, TypeError, AttributeError, IndexError) as error:
        return {"status": "failed", "error": str(error)}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--context", choices=("egl", "osmesa"), default="egl")
    parser.add_argument("--binary", type=Path, help="use an already-built probe instead of Cargo")
    parser.add_argument("--output", type=Path, default=ROOT / "target/legacy-gl-probe/report.json")
    parser.add_argument("--timeout", type=float, default=30, help="seconds per runtime case")
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    report = {"schema": 2, "platform": platform.platform(), "cases": {},
              "context": args.context,
              "note": "Overrides do not emulate X1650/Win7. GL2.0 compiles GLSL110 source; Mesa reports GLSL120."}
    if sys.platform != "linux":
        report.update(status="blocked", error="this initial smoke runner requires Linux")
    else:
        binary = args.binary
        if binary is None:
            cargo = shutil.which("cargo")
            if not cargo:
                report.update(status="blocked", error="Cargo not found; install a Rust toolchain")
            else:
                build = subprocess.run(
                    [cargo, "build", "--offline", "--manifest-path",
                     str(ROOT / "tools/legacy-gl-probe/Cargo.toml"),
                     "--target-dir", str(ROOT / "target/legacy-gl-probe")],
                    text=True, capture_output=True,
                )
                if build.returncode:
                    report.update(status="failed", error="probe build failed", build_log=build.stderr)
                else:
                    binary = ROOT / "target/legacy-gl-probe/debug/horizon-legacy-gl-probe"
        if binary is not None:
            binary = binary.resolve()
            if not binary.is_file():
                report.update(status="blocked", error=f"probe binary not found: {binary}")
            else:
                report["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
                for name, gl, glsl in MODES:
                    report["cases"][name] = run_case(binary, gl, glsl, args.timeout, args.context)
                report["status"] = "passed" if all(
                    case["status"] == "passed" for case in report["cases"].values()
                ) else "failed"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"legacy GL smoke: {report['status']}; report: {args.output}")
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
