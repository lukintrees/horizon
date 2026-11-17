#!/usr/bin/env python3
"""Run the game on the legacy desktop-GL2 backend and check it is really on GL2.

Starts `horizon-game` with `--renderer native-gl2`, waits for the menu, asks the
control socket for `status`, and requires the GL strings the context itself
reported. A run that fell back to the modern wgpu renderer fails: `status.legacy`
is the confirmation, and a log line or a screenshot is never taken as proof.

Everything a run writes goes into a fresh scratch root (game folder, home,
launcher home, XDG folders, saves). The real player's folders are never read or
written. Only the process group this script started is ever stopped.

    python3 tools/legacy_game_mvp.py                      # menu, then quit
    python3 tools/legacy_game_mvp.py --open-world T01     # menu, then a world
    python3 tools/legacy_game_mvp.py --keep                # keep the artifacts

Exit status: 0 passed, 1 failed, 3 usage. Anything else means the script broke.
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

try:
    from tools import lab
except ImportError:  # running as a script: tools/ is on sys.path
    import lab

# `status.legacy` must carry all of these before a run counts as legacy at all.
REQUIRED_LEGACY_FIELDS = ("backend", "gl_version", "glsl_version", "vendor", "renderer")
LEGACY_BACKENDS = ("legacy-gl", "LegacyGl", "legacy-gl2")
# The driver strings a software Mesa context reports. A run that claims legacy
# cannot also be llvmpipe's absence of GL: llvmpipe is the only software GL 2.1
# here, but the check below is about the *strings being real*, not about Mesa.
WANT_VERSION_PREFIX = "2."


class Failure(Exception):
    pass


def mesa_env(base: dict[str, str], gl_version: str, glsl_version: str) -> dict[str, str]:
    """Force a desktop GL 2.1 context on Mesa's llvmpipe, as the probe does.

    Nothing here changes the game; it tells Mesa which context to give us, so a
    machine without a GL 2.1 GPU can still run and be checked.
    """
    env = base.copy()
    env.update(
        LIBGL_ALWAYS_SOFTWARE="1",
        GALLIUM_DRIVER="llvmpipe",
        MESA_SHADER_CACHE_DISABLE="true",
        MESA_GL_VERSION_OVERRIDE=gl_version,
        MESA_GLSL_VERSION_OVERRIDE=glsl_version,
    )
    vendor = Path("/usr/share/glvnd/egl_vendor.d/50_mesa.json")
    if vendor.is_file():
        env["__EGL_VENDOR_LIBRARY_FILENAMES"] = str(vendor)
    return env


def last_json(text: str) -> dict[str, Any] | None:
    """The control socket answers one JSON value per line; take the last."""
    for line in reversed(text.splitlines()):
        line = line.strip()
        if line.startswith("{"):
            try:
                return json.loads(line)
            except json.JSONDecodeError:
                return None
    return None


class Ctl:
    """One `horizon-ctl` against a running game's socket."""

    def __init__(self, ctl: Path, socket: Path, env: dict[str, str], timeout: float):
        self.ctl, self.socket, self.env, self.timeout = ctl, socket, env, timeout

    def command(self, *args: str) -> dict[str, Any]:
        result = subprocess.run(
            [str(self.ctl), "--socket", str(self.socket), *args],
            capture_output=True,
            text=True,
            env=self.env,
            timeout=self.timeout,
        )
        reply = last_json(result.stdout + result.stderr)
        if reply is None:
            raise Failure(
                f"horizon-ctl {' '.join(args)}: no JSON reply (exit {result.returncode})\n"
                f"{result.stdout[-2000:]}{result.stderr[-2000:]}"
            )
        return reply

    def wait_socket(self, timeout: float) -> dict[str, Any]:
        """Retry until the game answers: the socket file appears before it listens."""
        deadline = time.monotonic() + timeout
        last: dict[str, Any] | str = ""
        while time.monotonic() < deadline:
            try:
                return self.command("hello")
            except (Failure, subprocess.TimeoutExpired) as error:
                last = str(error)
            time.sleep(0.25)
        raise Failure(f"the control socket never answered within {timeout}s: {last}")

    def wait_for(self, condition: str, value: str | None, timeout: float) -> dict[str, Any]:
        args = ["wait-for", condition] + ([value] if value is not None else [])
        args += ["--timeout", str(max(1, int(timeout)))]
        return self.command(*args)


def legacy_evidence(status: dict[str, Any]) -> dict[str, Any]:
    """The context's own confirmation, or why this run is not a legacy run.

    A wgpu backend is not a legacy confirmation: wgpu's GL API is a different
    renderer from this one and may be GL 3+, so `backend` must say legacy and
    every GL string must be present and real.
    """
    legacy = status.get("legacy")
    renderer = status.get("renderer") or {}
    if not isinstance(legacy, dict):
        raise Failure("status.legacy is missing: the game did not report its backend")
    if legacy.get("confirmed") is not True:
        raise Failure(f"status.legacy is not confirmed: {legacy}")
    if legacy.get("backend") not in LEGACY_BACKENDS:
        raise Failure(f"status.legacy names backend {legacy.get('backend')!r}")
    missing = [key for key in REQUIRED_LEGACY_FIELDS if not legacy.get(key)]
    if missing:
        raise Failure("status.legacy is missing " + ", ".join(missing))
    wgpu_backend = renderer.get("backend") if isinstance(renderer, dict) else None
    if wgpu_backend in {"Vulkan", "Dx12", "Metal"}:
        raise Failure(f"legacy was requested but wgpu backend {wgpu_backend} is drawing")
    version = str(legacy["gl_version"])
    if not version.startswith(WANT_VERSION_PREFIX):
        raise Failure(f"the context reports GL {version}, not a GL 2.x context")
    return legacy


def run(args: argparse.Namespace) -> dict[str, Any]:
    game = args.game or ROOT / lab.cargo_target() / "fast" / "horizon-game"
    ctl = args.ctl or game.with_name("horizon-ctl")
    for path, what in ((game, "game binary"), (ctl, "horizon-ctl")):
        if not Path(path).is_file():
            raise Failure(f"{what} not found at {path}: build it first "
                          f"(cargo build --profile fast -p horizon-game --features legacy-gl)")
    resources = Path(args.resources)
    if not (resources / "minecraft").is_dir():
        raise Failure(f"prepared resources not found at {resources} (tools/lab.py prepare)")

    root = Path(tempfile.mkdtemp(prefix="horizon-legacy-mvp-"))
    folders = {name: root / name for name in
               ("game", "home", "launcher", "data", "config", "runtime", "tmp", "artifacts")}
    for folder in folders.values():
        folder.mkdir(parents=True, exist_ok=True)
    env = lab.game_env(
        HORIZON_HOME=str(folders["home"]),
        HORIZON_LAUNCHER_HOME=str(folders["launcher"]),
        XDG_DATA_HOME=str(folders["data"]),
        XDG_CONFIG_HOME=str(folders["config"]),
        XDG_RUNTIME_DIR=str(folders["runtime"]),
        TMPDIR=str(folders["tmp"]),
        HORIZON_LAUNCHER_TRASH=str(folders["home"] / "trash"),
        HORIZON_LAUNCHER_CLIPBOARD=str(folders["home"] / "clipboard"),
        HORIZON_LAUNCHER_DESKTOP=str(folders["home"] / "desktop"),
        HORIZON_NO_DISCORD="1",
    )
    if args.software_gl:
        env = mesa_env(env, args.gl_version, args.glsl_version)
    socket = folders["runtime"] / "control.sock"
    command = [
        str(game),
        "--resources", str(resources),
        "--game-dir", str(folders["game"]),
        "--home", str(folders["home"]),
        "--renderer", "native-gl2",
        "--control",
        "--control-socket", str(socket),
    ] + (["--offscreen"] if args.offscreen else []) + args.extra

    log = folders["artifacts"] / "game.log"
    report: dict[str, Any] = {
        "command": command,
        "scratch": str(root),
        "software_gl": args.software_gl,
        "checkpoints": {},
        "log": str(log),
    }
    process = None
    try:
        with log.open("wb") as out:
            process = subprocess.Popen(
                command, cwd=folders["game"], env=env, stdout=out, stderr=subprocess.STDOUT,
                start_new_session=True,
            )
        control = Ctl(ctl, socket, env, args.timeout)
        control.wait_socket(args.timeout)
        report["checkpoints"]["socket"] = True

        status = control.command("status")
        report["checkpoints"]["status"] = True
        report["legacy"] = legacy_evidence(status)
        report["checkpoints"]["legacy"] = True

        control.wait_for("screen", "title", args.timeout)
        report["checkpoints"]["menu"] = True
        report["menu_screenshot"] = str(folders["artifacts"] / "menu.png")
        control.command("screenshot", report["menu_screenshot"])

        if args.open_world:
            control.command("open", args.open_world, "creative")
            control.wait_for("loaded", None, args.timeout)
            report["checkpoints"]["world_loaded"] = True
            report["world_screenshot"] = str(folders["artifacts"] / "world.png")
            control.command("screenshot", report["world_screenshot"])
            state = control.command("state")
            report["player"] = state.get("player")
            chunks = control.command("status").get("chunks")
            report["chunks"] = chunks

        control.command("quit")
        process.wait(timeout=60)
        report["checkpoints"]["quit"] = True
        if process.returncode not in (0, None):
            raise Failure(f"the game exited {process.returncode} after quit")
        report["status"] = "passed"
    except Failure as failure:
        report["status"] = "failed"
        report["error"] = str(failure)
    finally:
        if process is not None and process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=20)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        tail = log.read_text(errors="replace")[-4000:] if log.is_file() else ""
        report["log_tail"] = tail
        if not args.keep:
            # The artifacts go with the scratch root; copy the report out first.
            report["scratch_removed"] = True
            shutil.rmtree(root, ignore_errors=True)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--game", type=Path, help="the horizon-game binary")
    parser.add_argument("--ctl", type=Path, help="horizon-ctl beside it")
    parser.add_argument("--resources", default=str(ROOT / ".local" / "horizon-client" / "26.3"))
    parser.add_argument("--open-world", metavar="NAME",
                        help="also open this world and wait until it is loaded")
    parser.add_argument("--offscreen", action="store_true",
                        help="draw without a visible window")
    parser.add_argument("--no-software-gl", dest="software_gl", action="store_false",
                        help="use the machine's own GL driver instead of Mesa llvmpipe")
    parser.add_argument("--gl-version", default="2.1")
    parser.add_argument("--glsl-version", default="120")
    parser.add_argument("--timeout", type=float, default=180.0)
    parser.add_argument("--keep", action="store_true", help="keep the scratch root")
    parser.add_argument("--report", type=Path, default=ROOT / "target" / "legacy-game-mvp.json")
    parser.add_argument("extra", nargs="*", help="further game arguments")
    args = parser.parse_args()

    report = run(args)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    legacy = report.get("legacy")
    if report["status"] == "passed":
        print(f"passed: {legacy['backend']} GL {legacy['gl_version']} "
              f"GLSL {legacy['glsl_version']} on {legacy['renderer']}")
        for name in ("menu_screenshot", "world_screenshot"):
            if report.get(name):
                print(f"  {report[name]}")
    else:
        print(f"failed: {report.get('error')}")
        print(f"  log: {report.get('log')}")
        tail = report.get("log_tail", "").strip().splitlines()[-15:]
        for line in tail:
            print(f"  | {line}")
    print(f"report: {args.report}")
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
