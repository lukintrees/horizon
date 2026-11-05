#!/usr/bin/env python3
"""Real normal-resource native Game loading/menu smoke, NOT a world E2E.

Requires the user's prepared resources and feature-enabled client binary. Never
fetches assets or substitutes fixtures. All player data goes in a fresh temp home.
Exit 0=observed menu, 1=failed, 2=blocked (missing prerequisites).
"""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import sys
import tempfile

try:
    from tools.legacy_game_e2e import LocalProcess, TestHome, sha256, tail_text, write_json
except ImportError:
    from legacy_game_e2e import LocalProcess, TestHome, sha256, tail_text, write_json


def validate_report(report: dict, frames: int) -> None:
    expected = {"scenario": "native-game-menu-only", "backend": "native-gl2",
                "game_constructed": True, "loading_context_transferred": True,
                "outcome": "Presented", "presented": True, "playable": False,
                "world_opened": False}
    for key, value in expected.items():
        if type(report.get(key)) is not type(value) or report[key] != value:
            raise ValueError(f"not an observed native Game loading/menu: {key}={report.get(key)!r}")
    for key, minimum in (("loading_presentations", 1), ("menu_submissions", frames),
                         ("resource_generation", 1)):
        if type(report.get(key)) is not int or report[key] < minimum:
            raise ValueError(f"invalid {key}: {report.get(key)!r}")


def command(game: Path, resources: Path, home: TestHome, frames: int, timeout: int) -> list[str]:
    return [str(game), "--renderer", "native-gl2", "--resources", str(resources),
            "--game-dir", str(home.game_dir), "--player-dir", str(home.root / "player"),
            "--name", "NativeMenuSmoke", "--frames", str(frames), "--timeout-seconds", str(timeout),
            "--smoke-report", str(home.artifacts / "game.json")]


def environment(home: TestHome) -> dict[str, str]:
    # Keep display/ICD/library configuration, never inherited harness commands,
    # credentials, game data paths or HORIZON_* fixture switches.
    env = {k: v for k, v in os.environ.items() if not k.startswith("HORIZON_")}
    env.update(home.env(home.artifacts / "startup.json", home.root / "unused-control.sock"))
    if env.get("DISPLAY") and not env.get("XAUTHORITY"):
        authority = Path(os.environ.get("HOME", str(Path.home()))) / ".Xauthority"
        if authority.is_file():
            env["XAUTHORITY"] = str(authority)  # reference, never copy credentials
    env.pop("HORIZON_CONTROL", None)  # native menu intentionally has no control socket
    return env


def run(game: Path, resources: Path, frames: int, timeout: int) -> tuple[int, dict]:
    game, resources = game.resolve(), resources.resolve()
    evidence = {"scenario": "normal-resource-native-loading-menu", "state": "blocked",
                "playable": False, "world_acceptance": False,
                "game": str(game), "resources": str(resources)}
    if not game.is_file() or not resources.is_dir():
        evidence["error"] = "requires an existing client binary and normal prepared resource directory"
        return 2, evidence
    if sys.platform.startswith("linux") and not os.environ.get("DISPLAY"):
        evidence["error"] = "native GLX needs DISPLAY (X11/XWayland or xvfb-run), not offscreen"
        return 2, evidence
    evidence["binary_sha256"] = sha256(game)
    with tempfile.TemporaryDirectory(prefix="horizon-native-menu-") as tmp:
        home = TestHome.create(Path(tmp), reused=False)
        (home.root / "player").mkdir()
        cmd = command(game, resources, home, frames, timeout)
        evidence["command"] = cmd
        stdout, stderr = home.artifacts / "stdout.log", home.artifacts / "stderr.log"
        process = None
        try:
            process = LocalProcess(cmd, home.game_dir, environment(home), stdout, stderr)
            code = process.wait(timeout + 15)
            evidence["exit_code"] = code
            if code is None:
                raise ValueError("native Game loading/menu timed out")
            if code != 0:
                raise ValueError(f"native Game exited with {code}")
            report = json.loads((home.artifacts / "game.json").read_text(encoding="utf-8"))
            if not isinstance(report, dict):
                raise ValueError("native Game report is not an object")
            evidence["game_report"] = report
            validate_report(report, frames)
            evidence["state"] = "passed"
        except (OSError, ValueError) as error:
            evidence["state"] = "failed"
            evidence["error"] = str(error)
        finally:
            if process is not None:
                process.stop()  # only our own process/group, even on timeout
            evidence["stdout_tail"] = tail_text(stdout)
            evidence["stderr_tail"] = tail_text(stderr)
    return (0 if evidence["state"] == "passed" else 1), evidence


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game", required=True, type=Path)
    parser.add_argument("--resources", required=True, type=Path)
    parser.add_argument("--frames", type=int, default=180)
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("--report", type=Path, default=Path(".local/legacy-gl/native-menu.json"))
    args = parser.parse_args()
    if args.frames < 1 or args.timeout < 1:
        parser.error("frames and timeout must be positive")
    code, evidence = run(args.game, args.resources, args.frames, args.timeout)
    write_json(args.report, evidence)
    print(f"{evidence['state']}: {args.report}")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
