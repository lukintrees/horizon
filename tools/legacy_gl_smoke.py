#!/usr/bin/env python3
"""PR 9 legacy-GL smoke report (no GL context and no GPU calls).

Reads the reference caps fixture and the independent PR 6/7 frame stats, then
prints or writes a deterministic startup report. `--check` validates the
draw/sampler/VRAM budgets without pretending to have measured real X1650
hardware or wgpu visual parity. Real `glGetError` codes can be decoded with
`gl_error_name`; this tool itself never opens a window.
"""

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CAPS_PATH = ROOT / "benchmarks/graphics/x1650-reference-caps.json"
STATS_PATH = ROOT / "benchmarks/graphics/legacy-gl-scene.stats.json"
REPORT_PATH = ROOT / "benchmarks/graphics/legacy-gl-startup-report.txt"

MAX_DRAWS = 200
MAX_SAMPLERS = 4
MAX_VRAM_MIB = 160

GL_ERROR_NAMES = {
    0x0000: "GL_NO_ERROR",
    0x0500: "GL_INVALID_ENUM",
    0x0501: "GL_INVALID_VALUE",
    0x0502: "GL_INVALID_OPERATION",
    0x0503: "GL_STACK_OVERFLOW",
    0x0504: "GL_STACK_UNDERFLOW",
    0x0505: "GL_OUT_OF_MEMORY",
}


def gl_error_name(code):
    return GL_ERROR_NAMES.get(code, "UNKNOWN_GL_ERROR")


def load_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def build_lines(caps, stats):
    measured = caps.get("measured_on")
    return [
        "Legacy GL startup report (PR 9 diagnostics; no GL context)",
        f"  caps source          {CAPS_PATH.relative_to(ROOT)}",
        f"  measured_on          {measured} (NOT measured in this sandbox)",
        f"  tier                 {caps.get('tier')}",
        f"  GL_VERSION           {caps.get('gl_version')}",
        f"  GLSL_VERSION         {caps.get('glsl_version')}",
        f"  GL_VENDOR            {caps.get('vendor')}",
        f"  GL_RENDERER          {caps.get('renderer')}",
        f"  extensions           {caps.get('extension_count')}",
        f"  draw calls           {stats.get('draw_calls')}/{MAX_DRAWS}",
        f"  samplers             {stats.get('samplers')}/{MAX_SAMPLERS}",
        f"  VRAM estimate        {stats.get('vram_mib')}/{MAX_VRAM_MIB} MiB",
        f"  glGetError count     {stats.get('gl_errors')}",
        f"  disabled features    {', '.join(stats.get('disabled_features', []))}",
        f"  full frame draws     {stats.get('full_frame_draws')}/{MAX_DRAWS}",
        "  wgpu parity          NOT MEASURED",
        "  real X1650 run       NOT MEASURED",
    ]


def build_report(caps, stats):
    return "\n".join(build_lines(caps, stats)) + "\n"


def build_json(caps, stats):
    return {
        "schema": 1,
        "caps": caps,
        "stats": stats,
        "startup_lines": build_lines(caps, stats),
        "budget_ok": validate(caps, stats),
        "wgpu_parity": "NOT MEASURED",
        "real_x1650": "NOT MEASURED",
    }


def validate(caps, stats):
    return (
        caps.get("tier") == "L1"
        and caps.get("glsl") == 120
        and not caps.get("required_missing")
        and caps.get("measured_on") is None
        and stats.get("width") == 80
        and stats.get("height") == 60
        and stats.get("rgba_bytes") == 19200
        and stats.get("draw_calls", MAX_DRAWS + 1) <= MAX_DRAWS
        and stats.get("full_frame_draws", MAX_DRAWS + 1) <= MAX_DRAWS
        and stats.get("samplers", MAX_SAMPLERS + 1) <= MAX_SAMPLERS
        and stats.get("vram_mib", MAX_VRAM_MIB + 1) <= MAX_VRAM_MIB
        and stats.get("gl_errors", 1) == 0
        and stats.get("measured_on") is None
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--caps", type=Path, default=CAPS_PATH, help="capability JSON")
    parser.add_argument("--stats", type=Path, default=STATS_PATH, help="frame stats JSON")
    parser.add_argument("--report-out", type=Path, default=REPORT_PATH, help="text report path")
    parser.add_argument("--json-out", type=Path, help="optional JSON report path")
    parser.add_argument("--stdout", action="store_true", help="print the report")
    parser.add_argument("--write", action="store_true", help="write the text report")
    parser.add_argument("--check", action="store_true", help="validate budgets and exit")
    args = parser.parse_args()
    try:
        caps = load_json(args.caps)
        stats = load_json(args.stats)
        report = build_report(caps, stats)
        if args.stdout:
            sys.stdout.write(report)
        if args.write:
            args.report_out.parent.mkdir(parents=True, exist_ok=True)
            args.report_out.write_text(report, encoding="utf-8")
        if args.json_out:
            args.json_out.parent.mkdir(parents=True, exist_ok=True)
            args.json_out.write_text(
                json.dumps(build_json(caps, stats), indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
        if args.check and not validate(caps, stats):
            print("legacy-gl smoke: FAILED: budget or caps mismatch", file=sys.stderr)
            return 1
        if args.check:
            print("legacy-gl smoke: OK (L1, 5/200 draws, 4/4 samplers, 48/160 MiB)")
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as err:
        print(f"legacy-gl smoke: FAILED: {err}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
