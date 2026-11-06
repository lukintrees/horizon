#!/usr/bin/env python3
"""Normal-resource real-mesher -> native GPU integration, NOT Game acceptance.

Requires a legacy-gl horizon-client-render test executable and prepared resources.
Never downloads assets, creates resource fixtures, or treats ignored tests as a pass.
Exit: 0 observed regression passed, 1 failed, 2 missing prerequisites.
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

TEST = "renderer_owner::native_streaming::worker_tests::prepared_resources_real_mesher_rejects_inflight_replaced_columns"


def validate_report(report: dict) -> None:
    expected = {"scenario": "native-prepared-real-mesher", "normal_resources": True,
                "pending_limit": 1, "nonempty_draw": True,
                "replacement_draw_changed": True, "forgotten_draw_empty": True,
                "game_acceptance": False, "terrain_generation_tested": False,
                "save_reopen_tested": False}
    for key, value in expected.items():
        if type(report.get(key)) is not type(value) or report[key] != value:
            raise ValueError(f"not an observed real-worker regression: {key}={report.get(key)!r}")
    for key in ("worker_completions", "obsolete_completions", "received_columns", "old_revision", "new_revision", "resource_generation"):
        if type(report.get(key)) is not int or report[key] < 1:
            raise ValueError(f"invalid observed {key}: {report.get(key)!r}")
    if report["old_revision"] == report["new_revision"]:
        raise ValueError("replacement reused an obsolete owner ticket")


def command(tests: Path) -> list[str]:
    return [str(tests), TEST, "--exact", "--ignored", "--nocapture"]


def environment(home: TestHome, resources: Path, report: Path) -> dict[str, str]:
    env = {k: v for k, v in os.environ.items() if not k.startswith("HORIZON_")}
    # An explicit native library override is a loader setting, not a command or
    # a resource substitute. Preserve it for non-system OSMesa installations.
    if "HORIZON_OSMESA_LIBRARY" in os.environ:
        env["HORIZON_OSMESA_LIBRARY"] = os.environ["HORIZON_OSMESA_LIBRARY"]
    env.update({"HOME": str(home.root), "XDG_CONFIG_HOME": str(home.root / "config"),
                "XDG_DATA_HOME": str(home.root / "data"), "XDG_CACHE_HOME": str(home.root / "cache"),
                "XDG_STATE_HOME": str(home.root / "state"),
                "HORIZON_TEST_RESOURCES": str(resources), "HORIZON_NATIVE_WORKER_REPORT": str(report),
                "LIBGL_ALWAYS_SOFTWARE": "1", "GALLIUM_DRIVER": "llvmpipe",
                "MESA_GL_VERSION_OVERRIDE": "2.0", "MESA_GLSL_VERSION_OVERRIDE": "120"})
    return env


def run(tests: Path, resources: Path, timeout: int) -> tuple[int, dict]:
    tests, resources = tests.resolve(), resources.resolve()
    evidence = {"scenario": "normal-resource-native-real-worker", "state": "blocked",
                "game_acceptance": False, "win7_qualification": False,
                "tests": str(tests), "resources": str(resources)}
    manifest = resources / "horizon-resources.json"
    if not tests.is_file() or not manifest.is_file():
        evidence["error"] = "requires a legacy-gl test executable and normal prepared resource directory"
        return 2, evidence
    if not sys.platform.startswith("linux"):
        evidence["error"] = "this regression uses Linux OSMesa, not a WGL/hardware scenario"
        return 2, evidence
    evidence["binary_sha256"] = sha256(tests)
    evidence["resource_manifest_sha256"] = sha256(manifest)
    with tempfile.TemporaryDirectory(prefix="horizon-native-worker-") as tmp:
        home = TestHome.create(Path(tmp), reused=False)
        report = home.artifacts / "worker.json"
        stdout, stderr = home.artifacts / "stdout.log", home.artifacts / "stderr.log"
        process = None
        evidence["command"] = command(tests)
        try:
            process = LocalProcess(command(tests), home.game_dir, environment(home, resources, report), stdout, stderr)
            code = process.wait(timeout)
            evidence["exit_code"] = code
            if code is None:
                raise ValueError("real-worker regression timed out")
            if code != 0:
                raise ValueError(f"real-worker regression exited with {code}")
            observed = json.loads(report.read_text(encoding="utf-8"))
            if not isinstance(observed, dict):
                raise ValueError("worker report is not an object")
            validate_report(observed)
            evidence["worker_report"] = observed
            evidence["state"] = "passed"
        except (OSError, ValueError) as error:
            evidence["state"] = "failed"
            evidence["error"] = str(error)
        finally:
            if process is not None:
                process.stop()
            evidence["stdout_tail"] = tail_text(stdout)
            evidence["stderr_tail"] = tail_text(stderr)
    return (0 if evidence["state"] == "passed" else 1), evidence


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tests", type=Path, required=True)
    parser.add_argument("--resources", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("--report", type=Path, default=Path(".local/legacy-gl/native-worker.json"))
    args = parser.parse_args()
    if args.timeout < 1:
        parser.error("timeout must be positive")
    code, evidence = run(args.tests, args.resources, args.timeout)
    write_json(args.report, evidence)
    print(f"{evidence['state']}: {args.report}")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
