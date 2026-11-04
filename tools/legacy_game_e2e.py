#!/usr/bin/env python3
"""Isolated process runner for future real-Game legacy GL E2E scenarios.

This harness does not claim a successful game run unless the child reports
explicit, machine-readable checkpoints and confirms the requested backend.
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Sequence

CHECKPOINTS = ("startup", "menu", "world", "save")
REQUIRED_HOMES = ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME")


class RunFailure(RuntimeError):
    pass


def _stop_child(proc: subprocess.Popen[str], grace: float = 2.0) -> None:
    """Stop only this child process tree (created in a dedicated session)."""
    if proc.poll() is not None:
        return
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        proc.wait(timeout=grace)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        proc.wait()


def run_game(command: Sequence[str], *, timeout: float, home: Path | None = None,
             keep_home: bool = False, expected_backend: str = "legacy-gl") -> dict:
    """Run command in a fresh test home, or reuse an explicitly supplied test home.

    Reusing ``home`` supports save/reopen scenarios. Such a directory must carry
    the harness marker created on first use; arbitrary paths are rejected.
    """
    if not command:
        raise ValueError("command must not be empty")
    owned_temp = home is None
    root = Path(tempfile.gettempdir()) / f"horizon-legacy-e2e-{os.urandom(8).hex()}" if home is None else home.resolve()
    marker = root / ".horizon-legacy-e2e-home"
    try:
        existed = root.exists()
        if existed:
            if not marker.is_file():
                raise RunFailure(f"refusing to use existing unmarked test home: {root}")
        else:
            root.mkdir(parents=True, exist_ok=False)
            marker.write_text("isolated legacy game E2E home v1\n", encoding="utf-8")
        env = os.environ.copy()
        home_vars = {
            "HORIZON_HOME": root / "game",
            "HORIZON_LAUNCHER_HOME": root / "launcher",
            "XDG_DATA_HOME": root / "xdg-data",
            "XDG_CONFIG_HOME": root / "xdg-config",
        }
        for key, path in home_vars.items():
            path.mkdir(parents=True, exist_ok=True)
            env[key] = str(path)
        env["HORIZON_LAUNCHER_NO_SELF_UPDATE"] = "1"
        env["HORIZON_NO_AUDIO"] = "1"
        started = time.monotonic()
        try:
            proc = subprocess.Popen(list(command), env=env, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                    start_new_session=True)
            try:
                stdout, stderr = proc.communicate(timeout=timeout)
                timed_out = False
            except subprocess.TimeoutExpired:
                timed_out = True
                _stop_child(proc)
                stdout, stderr = proc.communicate()
        except OSError as exc:
            raise RunFailure(f"could not start game process: {exc}") from exc
        log = stdout + "\n" + stderr
        checkpoints = sorted({line.partition(" ")[2].strip() for line in log.splitlines()
                              if line.startswith("HORIZON_E2E_CHECKPOINT ")})
        backend_lines = [line for line in log.splitlines() if line.startswith("HORIZON_E2E_BACKEND ")]
        fallbacks = [line for line in log.splitlines() if "fallback" in line.lower() and "wgpu" in line.lower()]
        result = {
            "status": "blocked" if not backend_lines else "failure",
            "command": list(command), "exit_code": proc.returncode,
            "timed_out": timed_out, "duration_seconds": round(time.monotonic() - started, 3),
            "backend": backend_lines[-1].partition(" ")[2].strip() if backend_lines else None,
            "checkpoints": checkpoints, "stdout": stdout, "stderr": stderr,
            "test_home": str(root), "evidence_kind": "real_process_logs",
        }
        missing = [stage for stage in CHECKPOINTS if stage not in checkpoints]
        result["missing_checkpoints"] = missing
        if timed_out:
            result["status"] = "failure"
            result["reason"] = "timeout"
        elif proc.returncode != 0:
            result["status"] = "failure"
            result["reason"] = "process_crash_or_nonzero_exit"
        elif not backend_lines or result["backend"] != expected_backend:
            result["status"] = "blocked"
            result["reason"] = "legacy_backend_unconfirmed"
        elif fallbacks:
            result["status"] = "failure"
            result["reason"] = "unexpected_modern_fallback"
        elif missing:
            result["status"] = "failure"
            result["reason"] = "checkpoints_missing"
        else:
            result["status"] = "success"
        return result
    finally:
        if owned_temp and not keep_home:
            shutil.rmtree(root, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--home", type=Path, help="marked test home to reuse for save/reopen")
    parser.add_argument("--keep-home", action="store_true")
    parser.add_argument("--expected-backend", default="legacy-gl")
    parser.add_argument("--result", type=Path, help="write machine-readable JSON result")
    parser.add_argument("command", nargs=argparse.REMAINDER, help="game command after --")
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    result = run_game(command, timeout=args.timeout, home=args.home,
                      keep_home=args.keep_home, expected_backend=args.expected_backend)
    payload = json.dumps(result, indent=2)
    print(payload)
    if args.result:
        args.result.write_text(payload + "\n", encoding="utf-8")
    return 0 if result["status"] == "success" else 1


if __name__ == "__main__":
    raise SystemExit(main())
