#!/usr/bin/env python3
"""Run the WGL bootstrap probe under Wine, in a fresh isolated Windows prefix.

Proves the Windows path of the legacy desktop-GL2 backend actually executes: a
real `wglCreateContext` from `horizon-legacy-render`'s own `wgl.rs`, GL entry
points resolved through its own `driver.rs`/`wgl_address.rs` table, a drawn and
swapped frame, and a pixel read back. Wine's GL comes from the host's Mesa (a
software llvmpipe context when there is no GPU): that is the expected, accepted
renderer here, not a failure. The strings Wine hands the process are the ones
Mesa produced, so a GL 2.x / GLSL 1.1x report is about the context, not about
the driver being the X1650.

Everything a run writes goes into a fresh scratch root: a new `mktemp -d` Wine
prefix per run (never `~/.wine`), plus fresh `HORIZON_HOME`, launcher home and
XDG folders. The real player's data is never read or written. The build output
goes to a target directory; no binary is committed.

    python3 tools/legacy_wgl_wine.py                 # 64-bit Win7 target
    python3 tools/legacy_wgl_wine.py --arch x86      # 32-bit, the real Win7 shape
    python3 tools/legacy_wgl_wine.py --keep          # keep the prefix and exe
    python3 tools/legacy_wgl_wine.py --exe PATH      # run an already-built exe

Exit status: 0 passed, 1 failed, 3 usage. A missing wine/mingw/Rust toolchain or
an unbuildable probe is `blocked`, not `passed`.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# The reusable backend's own mandatory outcomes. A report missing any of these
# did not exercise the renderer, even if the triangle-only checks passed.
REQUIRED_CHECKS = {
    "shader_compile_link", "vbo_ebo_draw", "clear_readback", "triangle_readback",
    "depth_readback", "invalid_shader_rejected", "no_gl_error",
}
REQUIRED_BACKEND_CHECKS = {
    "backend_rgba_texture", "backend_texture_update", "backend_depth", "backend_cutout",
    "backend_ordered_blend_no_depth_write", "backend_overlay_scissor_reset",
    "backend_lightmap", "backend_fog", "backend_game_frame_inputs",
    "backend_screen_sampling", "backend_world_additive", "backend_world_weather",
    "backend_sky_background", "backend_large_coordinates", "backend_validation_budget",
    "backend_context_isolation", "backend_resource_reclamation",
    "backend_terrain_atlas_contract", "backend_resident_sections",
    "backend_gui_compositor", "backend_paged_gui", "backend_paged_entities",
    "backend_entity_layers", "backend_entity_glint", "backend_gui_glint",
    "backend_surface_lifecycle",
}

# Outcomes the probe cannot produce on WGL. `render_smoke.rs` only creates the
# second, unrelated context for the offscreen backends (`osmesa`/`egl`); a WGL
# run therefore never emits `backend_context_isolation`. Every other outcome is
# mandatory, and the total is asserted below so a silently skipped check fails.
WGL_SKIPPED_BACKEND_CHECKS = {"backend_context_isolation"}
WGL_REQUIRED_BACKEND_CHECKS = REQUIRED_BACKEND_CHECKS - WGL_SKIPPED_BACKEND_CHECKS

# Win7 targets that build with `-Zbuild-std`: the real shape of the X1650 machine.
# Both use a win64 prefix: Wine 11 is built in wow64 mode, where WINEARCH=win32 is
# rejected, and a 32-bit PE runs inside a 64-bit prefix just as it does on the
# x64 Windows hosts the project also ships. The exe's own bitness is what the
# probe reports (`pointer_bits`), and the harness checks it.
TARGETS = {
    "x86_64": "x86_64-win7-windows-gnu",
    "x86": "i686-win7-windows-gnu",
}

# Each case is (name, reported GL, GLSL source version). Wine does not emulate an
# X1650; the Mesa overrides only pin the *reported* version so the probe takes its
# GL2-only code path. GLSL 110 source is compiled in both, and Mesa reports 1.20.
MODES = (("gl21", "2.1", "120"), ("gl20", "2.0", "110"))


def mesa_env(base: dict[str, str], gl: str | None, glsl: str) -> dict[str, str]:
    """Force Mesa's llvmpipe at a GL 2.x ceiling, for the reasons the Linux runner does.

    Wine's `opengl32` forwards to the host's GLX, so these host variables reach the
    same Mesa the native Linux smoke test uses. `LIBGL_ALWAYS_SOFTWARE`/`GALLIUM_DRIVER`
    are kept even on a machine with a GPU: the point is a reproducible GL 2 context.
    """
    env = base.copy()
    env.update(
        LIBGL_ALWAYS_SOFTWARE="1",
        GALLIUM_DRIVER="llvmpipe",
        MESA_SHADER_CACHE_DISABLE="true",
    )
    for key in ("MESA_GL_VERSION_OVERRIDE", "MESA_GLSL_VERSION_OVERRIDE"):
        env.pop(key, None)
    if gl is not None:
        # Mesa rejects a reported language version of 110; 120 still compiles
        # explicit `#version 110` source, so the GL2.0 case stays honest.
        env["MESA_GL_VERSION_OVERRIDE"] = gl
        env["MESA_GLSL_VERSION_OVERRIDE"] = str(max(120, int(glsl)))
    return env


def isolated_env(prefix: Path) -> dict[str, str]:
    """A process environment that cannot touch the real player's folders."""
    env = os.environ.copy()
    env.update(
        WINEPREFIX=str(prefix),
        WINEDEBUG="-all",
        HORIZON_LAUNCHER_NO_SELF_UPDATE="1",
        HORIZON_NO_AUDIO="1",
        # Keep the prefix and everything a Windows process writes inside the
        # scratch root, never in the user's home.
        HORIZON_HOME=str(prefix / "horizon-home"),
        HORIZON_LAUNCHER_HOME=str(prefix / "horizon-launcher"),
        XDG_DATA_HOME=str(prefix / "xdg-data"),
        XDG_CONFIG_HOME=str(prefix / "xdg-config"),
    )
    return env


def validate_report(data: dict, gl: str | None, glsl: str, pointer_bits: int | None = None) -> None:
    """The pass/fail rules, independent of any driver. Raises ValueError."""
    if data.get("status") != "passed":
        raise ValueError("probe did not report success")
    if data.get("target_os") != "windows":
        raise ValueError("probe did not run as a Windows process")
    if pointer_bits is not None and data.get("pointer_bits") != pointer_bits:
        raise ValueError(
            f"probe is {data.get('pointer_bits')}-bit, expected {pointer_bits}"
        )
    if data.get("context") != "WGL desktop OpenGL window":
        raise ValueError("probe did not create the WGL context")
    if not data.get("buffer_swap"):
        raise ValueError("WGL window never presented a frame")
    if not REQUIRED_CHECKS.issubset(data.get("checks", [])):
        raise ValueError("probe omitted mandatory draw/readback checks")
    if not WGL_REQUIRED_BACKEND_CHECKS.issubset(data.get("backend_checks", [])):
        raise ValueError("probe omitted reusable-backend rendering checks")
    version = data.get("gl_version", "").split()
    if not version or not version[0].startswith("2."):
        raise ValueError(f"GL version is not 2.x: {data.get('gl_version')!r}")
    if gl is not None and version[0] != gl:
        raise ValueError(f"probe did not run GL {gl}")
    glsl_reported = data.get("glsl_version", "")
    # The GL2-era language versions are 1.10 and 1.20; anything else (a modern
    # "4.60", or GLSL ES) means Wine never gave us a GL2 context.
    if not glsl_reported.startswith(("1.10", "1.20")):
        raise ValueError(f"GLSL is not 1.1x: {glsl_reported!r}")
    if not data.get("vendor"):
        raise ValueError("empty GL vendor")
    if not data.get("renderer"):
        raise ValueError("empty GL renderer")
    if data.get("shader_version") != glsl:
        raise ValueError(f"probe did not compile GLSL {glsl}")


def run_case(exe: Path, gl: str | None, glsl: str, timeout: float, prefix: Path,
             pointer_bits: int | None = None) -> dict:
    args = [str(exe), "--context", "wgl", "--require-llvmpipe", "--shader-version", glsl]
    if gl is not None:
        args += ["--expect-gl", gl]
    try:
        result = subprocess.run(
            ["wine", *args],
            cwd=prefix,
            env=mesa_env(isolated_env(prefix), gl, glsl),
            text=True, capture_output=True, timeout=timeout,
        )
        if result.returncode:
            return {"status": "failed", "returncode": result.returncode,
                    "stderr": result.stderr[-4000:], "stdout": result.stdout[-4000:]}
        line = next((l for l in reversed(result.stdout.splitlines())
                     if l.strip().startswith("{")), None)
        if line is None:
            return {"status": "failed", "error": "no JSON report on stdout",
                    "stdout": result.stdout[-4000:], "stderr": result.stderr[-4000:]}
        data = json.loads(line)
        try:
            validate_report(data, gl, glsl, pointer_bits)
        except ValueError as error:
            return {"status": "failed", "error": str(error), "probe": data}
        return {"status": "passed", "probe": data}
    except (OSError, subprocess.TimeoutExpired, ValueError, TypeError, AttributeError) as error:
        return {"status": "failed", "error": str(error)}


def build(exe_target: str, target_dir: Path) -> tuple[Path | None, str]:
    """Build the probe for the Win7 GNU target. Returns (exe, log)."""
    cargo = Path.home() / ".cargo/bin/cargo"
    toolchain = os.environ.get("HORIZON_WINE_RUSTUP_TOOLCHAIN", "nightly")
    if not cargo.is_file():
        cargo = Path(shutil.which("cargo") or "")
    if not cargo.is_file():
        return None, "Cargo not found; install a Rust toolchain"
    env = os.environ.copy()
    env["RUSTUP_TOOLCHAIN"] = toolchain
    result = subprocess.run(
        [str(cargo), "-Zbuild-std=std,panic_abort", "build",
         "--target", exe_target,
         "--manifest-path", str(ROOT / "tools/legacy-gl-probe/Cargo.toml"),
         "--target-dir", str(target_dir)],
        text=True, capture_output=True, env=env,
    )
    if result.returncode:
        return None, "probe build failed:\n" + result.stderr[-8000:]
    name = "horizon-legacy-gl-probe.exe"
    return target_dir / exe_target / "debug" / name, ""


def wine_prefix() -> Path:
    """A brand-new Wine prefix on a filesystem with room for it.

    `/tmp` is often a small tmpfs that another build can fill; prefer it only if
    it has space, else fall back to the system temp root chosen by `mktemp`.
    """
    base = os.environ.get("HORIZON_WINE_SCRATCH")
    if base:
        Path(base).mkdir(parents=True, exist_ok=True)
        return Path(tempfile.mkdtemp(prefix="legacy-wgl-wine-", dir=base))
    return Path(tempfile.mkdtemp(prefix="legacy-wgl-wine-"))


def version_pin_commands() -> tuple[list[str], list[str]]:
    """The registry commands that make Wine report Windows 7.

    Wine 11 keeps the reported version under `HKCU\\Software\\Wine` as `Version`;
    with `win7`, `cmd /c ver` reports 6.1.7601. Kept in one place so the runner
    tests the exact commands it executes.
    """
    key = r"HKCU\Software\Wine"
    return (["wine", "reg", "add", key, "/v", "Version", "/d", "win7", "/f"],
            ["wine", "reg", "query", key, "/v", "Version"])


def isolate_desktop_commands() -> list[list[str]]:
    """The registry settings that put Wine's windows on its own virtual desktop.

    The probe resizes its window down to 1x1. A host window manager (KWin here,
    over Xwayland) clamps a real window to its own minimum size asynchronously,
    so the client rect comes back as something like 111x30 and the resize check
    fails for a reason that has nothing to do with WGL. Wine's virtual desktop is
    a single toplevel the host WM sees; inside it Wine is the only window manager,
    so the requested client sizes are honoured. Set via the registry (what Wine
    reads at start); the persistent `explorer /desktop` is what actually owns it.
    """
    explorer = r"HKCU\Software\Wine\Explorer"
    return [
        ["wine", "reg", "add", explorer + r"\Desktops", "/v", "Default",
         "/d", "800x600", "/f"],
        ["wine", "reg", "add", explorer, "/v", "Desktop", "/d", "Default", "/f"],
    ]


DESKTOP_NAME = "horizon-wgl"
DESKTOP_SIZE = "800x600"


def start_desktop(prefix: Path, settle: float = 6.0) -> subprocess.Popen | None:
    """Start Wine's explorer owning a persistent virtual desktop.

    Without a live explorer the registry alone is not enough: the first window
    created on the desktop can fail with "module not found", and later ones get
    the host WM's size clamp. Returns the process, or None if it exited at once.
    """
    env = isolated_env(prefix)
    try:
        process = subprocess.Popen(
            ["wine", "explorer", f"/desktop={DESKTOP_NAME},{DESKTOP_SIZE}"],
            env=env, cwd=prefix,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
    except OSError:
        return None
    # explorer creates the toplevel asynchronously; a case started before it is
    # up is the one that fails with "module not found". Give it a fixed window.
    time.sleep(settle)
    return None if process.poll() is not None else process


def stop_desktop(process: subprocess.Popen) -> None:
    """End the desktop's explorer; its wineserver closes when the prefix is dropped."""
    try:
        process.terminate()
        process.wait(timeout=15)
    except (OSError, subprocess.TimeoutExpired):
        try:
            process.kill()
        except OSError:
            pass


def init_prefix(prefix: Path, timeout: float) -> str | None:
    """Create the prefix, pin Windows 7, and configure the virtual desktop."""
    env = isolated_env(prefix)
    env["WINEARCH"] = "win64"
    boot = subprocess.run(["wineboot", "--init"], env=env, text=True,
                          capture_output=True, timeout=timeout)
    if boot.returncode:
        return "wineboot --init failed: " + boot.stderr[-2000:]
    add, query = version_pin_commands()
    added = subprocess.run(add, env=env, text=True, capture_output=True, timeout=timeout)
    if added.returncode:
        return "wine reg add Version=win7 failed: " + added.stderr[-2000:]
    # Verify rather than assume: a Wine that ignored the key would still pass add.
    queried = subprocess.run(query, env=env, text=True, capture_output=True,
                             timeout=timeout)
    if queried.returncode or "win7" not in queried.stdout:
        return "Wine did not retain the win7 version setting: " + queried.stdout[-2000:]
    for command in isolate_desktop_commands():
        result = subprocess.run(command, env=env, text=True, capture_output=True,
                                timeout=timeout)
        if result.returncode:
            return "wine virtual desktop setup failed: " + result.stderr[-2000:]
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--arch", choices=tuple(TARGETS), default="x86_64")
    parser.add_argument("--exe", type=Path, help="run an already-built probe instead of Cargo")
    parser.add_argument("--output", type=Path,
                        default=ROOT / "target/legacy-wgl-wine/report.json")
    parser.add_argument("--timeout", type=float, default=120, help="seconds per case")
    parser.add_argument("--keep", action="store_true", help="keep the prefix and exe")
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    if shutil.which("wine") is None:
        parser.error("wine is not installed")

    exe_target = TARGETS[args.arch]
    report: dict = {
        "schema": 1,
        "platform": platform.platform(),
        "wine": subprocess.run(["wine", "--version"], text=True,
                               capture_output=True).stdout.strip(),
        "target": exe_target,
        "note": ("Wine's GL is the host's Mesa/llvmpipe, not an X1650. The Mesa "
                 "overrides pin the reported GL/GLSL version; this proves the WGL "
                 "and GL2 code paths execute, not ATI driver compatibility."),
        "cases": {},
    }

    prefix = wine_prefix()
    exe = args.exe
    desktop = None
    try:
        boot_error = init_prefix(prefix, args.timeout)
        if boot_error:
            report.update(status="blocked", error=boot_error)
        else:
            if exe is None:
                target_dir = ROOT / "target/legacy-gl-probe-wine"
                exe, log = build(exe_target, target_dir)
                if exe is None:
                    report.update(status="blocked", error=log)
            if exe is not None:
                exe = exe.resolve()
                if not exe.is_file():
                    report.update(status="blocked", error=f"probe exe not found: {exe}")
                else:
                    report["exe_sha256"] = hashlib.sha256(exe.read_bytes()).hexdigest()
                    bits = {"x86_64": 64, "x86": 32}[args.arch]
                    report["pointer_bits"] = bits
                    drive_c = prefix / "drive_c"
                    drive_c.mkdir(exist_ok=True)
                    local = drive_c / "horizon-legacy-gl-probe.exe"
                    shutil.copy2(exe, local)
                    desktop = start_desktop(prefix)
                    report["virtual_desktop"] = desktop is not None
                    for name, gl, glsl in MODES:
                        report["cases"][name] = run_case(
                            local, gl, glsl, args.timeout, prefix, bits)
                    report["status"] = "passed" if all(
                        case["status"] == "passed" for case in report["cases"].values()
                    ) else "failed"
    finally:
        if desktop is not None:
            stop_desktop(desktop)
        # A failed or blocked run keeps its prefix so the reason can be inspected;
        # a passing run that was not asked to keep it cleans up after itself.
        if not args.keep and report.get("status") == "passed":
            shutil.rmtree(prefix, ignore_errors=True)

    if report.get("status") is None:
        report.update(status="failed", error="no case ran")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"legacy WGL under Wine: {report['status']}; report: {args.output}")
    if report.get("status") == "blocked":
        return 1
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
