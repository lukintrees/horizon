#!/usr/bin/env python3
"""Capture OpenGL 2.x capabilities for the legacy GL 2.1/2.0 renderer.

The game normally needs Vulkan/DX12 or OpenGL 3.3 through wgpu. The legacy tier
(docs/design/legacy-gl21.md) targets Radeon X1650 / R500, which tops out at
OpenGL 2.1 + GLSL 1.20. This tool turns `glxinfo` output into a small JSON/text
report: version, limits, extensions, missing requirements and the chosen tier
L1 (GL 2.1), L0 (GL 2.0) or F0 (no usable GLSL compiler).

Run it on the target machine (Linux + Mesa r300g + X11/GLX):

    glxinfo -B -l > /tmp/x1650.txt
    python3 tools/gl21_caps.py --from-file /tmp/x1650.txt --label x1650 \
        --output-dir benchmarks/graphics --check

or let the tool call glxinfo itself:

    python3 tools/gl21_caps.py --label x1650 --check

For CI, Mesa can *report* an older version (it does not actually remove the
driver's features):

    LIBGL_ALWAYS_SOFTWARE=1 MESA_GL_VERSION_OVERRIDE=2.1 \
    MESA_GLSL_VERSION_OVERRIDE=120 MESA_EXTENSION_MAX_YEAR=2006 \
    python3 tools/gl21_caps.py --label ci-gl21 --check --no-write

Everything players run stays Rust; this is a dev tool under tools/ (AGENTS.md).
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Iterable

SCHEMA = 1

# The absolute floor for the F0 fixed-function / ARB-program profile.
REQUIRED_BASE = (
    "GL_ARB_vertex_buffer_object",
    "GL_ARB_multitexture",
    "GL_ARB_texture_non_power_of_two",
)
# Either extension supplies offscreen colour+depth targets.
REQUIRED_FBO = ("GL_EXT_framebuffer_object", "GL_ARB_framebuffer_object")
FBO_REQUIREMENT = "GL_EXT_framebuffer_object|GL_ARB_framebuffer_object"

# Useful but optional: the renderer degrades when one is missing.
OPTIONAL = (
    "GL_ARB_depth_texture",
    "GL_ARB_framebuffer_object",
    "GL_ARB_map_buffer_range",
    "GL_ARB_occlusion_query",
    "GL_ARB_pixel_buffer_object",
    "GL_ARB_shader_texture_lod",
    "GL_ARB_shading_language_100",
    "GL_ARB_shadow",
    "GL_ARB_texture_cube_map",
    "GL_ARB_vertex_array_object",
    "GL_EXT_blend_func_separate",
    "GL_EXT_framebuffer_blit",
    "GL_EXT_framebuffer_multisample",
    "GL_EXT_multi_draw_arrays",
    "GL_EXT_packed_depth_stencil",
    "GL_EXT_texture_filter_anisotropic",
    "GL_EXT_texture_sRGB",
)

# glxinfo -l prints these as `GL_MAX_* = N`. A missing one is a note, not a
# hard failure: caps.rs in the Rust backend queries the same values at runtime.
EXPECTED_LIMITS = (
    "GL_MAX_TEXTURE_SIZE",
    "GL_MAX_TEXTURE_IMAGE_UNITS",
    "GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS",
    "GL_MAX_VERTEX_ATTRIBS",
    "GL_MAX_VERTEX_UNIFORM_COMPONENTS",
    "GL_MAX_VARYING_FLOATS",
    "GL_MAX_DRAW_BUFFERS",
    "GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS",
)

STRING_KEYS = {
    "OpenGL vendor string": "vendor",
    "OpenGL renderer string": "renderer",
    "OpenGL version string": "gl_version",
    "OpenGL shading language version string": "glsl_version",
    "OpenGL core profile version string": "core_version",
    "OpenGL core profile shading language version string": "core_glsl_version",
    "Max compat profile version": "compat_profile_version",
    "Max core profile version": "core_profile_version",
    "OpenGL ES profile version string": "gles_version",
}

# Extension names always carry a vendor prefix after GL_/GLX_/WGL_. GL_MAX_*,
# GL_VERSION and friends therefore never match.
EXTENSION_TOKEN = re.compile(
    r"\b(?:GL|GLX|WGL)_(?:ARB|EXT|ATI|NV|AMD|MESA|INTEL|SGIS|WIN|KTX|KHR|OES|"
    r"APPLE|IBM|SUN|HP|3DFX|INGR|PGI|REND|SGIX|SUNX|AMDX)_[A-Za-z0-9_]+\b"
)
LINE_VALUE = re.compile(r"^\s*([^:=]+?)\s*[:=]\s*(.*?)\s*$")
LIMIT_LINE = re.compile(r"^\s*(GL_[A-Z0-9_]+)\s*=\s*(\d+)\s*$", re.M)
VERSION_RE = re.compile(r"(\d+)\.(\d+)")


def parse_version(text: str | None) -> list[int] | None:
    """`2.1 Mesa 23.2` -> [2, 1]; `1.20` -> [1, 20]."""
    if not text:
        return None
    match = VERSION_RE.search(text)
    if not match:
        return None
    return [int(match.group(1)), int(match.group(2))]


def parse_glsl(text: str | None) -> int | None:
    """`1.20` -> 120, `4.50` -> 450."""
    pair = parse_version(text)
    if pair is None:
        return None
    return pair[0] * 100 + pair[1]


def parse_glxinfo(text: str) -> dict:
    """Parse the parts of glxinfo -B -l output the legacy tier needs."""
    strings: dict[str, str] = {}
    for line in text.splitlines():
        match = LINE_VALUE.match(line)
        if not match:
            continue
        key = match.group(1).strip()
        if key in STRING_KEYS:
            strings[STRING_KEYS[key]] = match.group(2).strip()

    limits = {
        name: int(value)
        for name, value in LIMIT_LINE.findall(text)
    }
    extensions = sorted(set(EXTENSION_TOKEN.findall(text)))

    gl_version_text = (
        strings.get("gl_version")
        or strings.get("compat_profile_version")
        or strings.get("core_version")
        or ""
    )
    glsl_text = strings.get("glsl_version") or strings.get("core_glsl_version") or ""
    glsl = parse_glsl(glsl_text)
    # A core-only report cannot be used to promise a GL 2.x compatibility
    # context; keep the number but say so in the notes.
    core_only = "gl_version" not in strings and "core_version" in strings

    return {
        "vendor": strings.get("vendor"),
        "renderer": strings.get("renderer"),
        "gl_version": gl_version_text or None,
        "glsl_version": glsl_text or None,
        "core_profile_version": strings.get("core_profile_version"),
        "compat_profile_version": strings.get("compat_profile_version"),
        "version": parse_version(gl_version_text),
        "glsl": glsl,
        "limits": limits,
        "extensions": extensions,
        "core_only": core_only,
    }


def choose_tier(parsed: dict) -> tuple[str, list[str], list[str]]:
    """(tier, required_missing, notes)."""
    extensions = set(parsed["extensions"])
    missing = [name for name in REQUIRED_BASE if name not in extensions]
    if not any(name in extensions for name in REQUIRED_FBO):
        missing.append(FBO_REQUIREMENT)

    notes: list[str] = []
    if parsed["core_only"]:
        notes.append("glxinfo reported only a core profile; a GL 2.x context has no core/compatibility split")
    if "GL_ARB_shading_language_100" not in extensions:
        notes.append("GL_ARB_shading_language_100 not listed; GLSL version string is authoritative")
    for name in EXPECTED_LIMITS:
        if name not in parsed["limits"]:
            notes.append(f"limit {name} not reported; query it at runtime")
    if parsed["version"] is None:
        notes.append("GL_VERSION could not be parsed")

    if missing:
        return "unsupported", missing, notes
    version = parsed["version"]
    glsl = parsed["glsl"]
    if version and version >= [2, 1] and glsl and glsl >= 120:
        return "L1", missing, notes
    if version and version >= [2, 0] and glsl and glsl >= 110:
        return "L0", missing, notes
    return "F0", missing, notes


def build_report(text: str, *, label: str, source_note: str) -> dict:
    parsed = parse_glxinfo(text)
    tier, missing, notes = choose_tier(parsed)
    extensions = set(parsed["extensions"])
    optional_present = sorted(name for name in OPTIONAL if name in extensions)
    optional_missing = sorted(name for name in OPTIONAL if name not in extensions)

    return {
        "schema": SCHEMA,
        "label": label,
        "source": source_note,
        "measured_on": None,  # never invent a hostname or a machine we did not touch
        "vendor": parsed["vendor"],
        "renderer": parsed["renderer"],
        "gl_version": parsed["gl_version"],
        "glsl_version": parsed["glsl_version"],
        "core_profile_version": parsed["core_profile_version"],
        "compat_profile_version": parsed["compat_profile_version"],
        "version": parsed["version"],
        "glsl": parsed["glsl"],
        "tier": tier,
        "required_missing": missing,
        "optional_present": optional_present,
        "optional_missing": optional_missing,
        "extension_count": len(parsed["extensions"]),
        "extensions": parsed["extensions"],
        "limits": parsed["limits"],
        "notes": notes,
    }


def check_tier(report: dict) -> list[str]:
    """Problems that make `--check` fail. Empty means the tier is usable."""
    problems: list[str] = []
    if report["tier"] not in ("L1", "L0", "F0"):
        problems.append(f"tier {report['tier']!r} is not usable")
    for name in report["required_missing"]:
        problems.append(f"missing required extension {name}")
    return problems


def format_text(report: dict) -> str:
    lines = [
        f"Legacy GL capability report: {report['label']}",
        f"  source          {report['source']}",
        f"  vendor          {report['vendor'] or '?'}",
        f"  renderer        {report['renderer'] or '?'}",
        f"  GL_VERSION      {report['gl_version'] or '?'}",
        f"  GLSL_VERSION    {report['glsl_version'] or '?'}",
        f"  tier            {report['tier']}",
        f"  extensions      {report['extension_count']}",
        "",
        "Required extensions:",
    ]
    for name in REQUIRED_BASE:
        lines.append(f"  {'OK ' if name in report['extensions'] else 'MISS'} {name}")
    lines.append(
        f"  {'OK ' if any(n in report['extensions'] for n in REQUIRED_FBO) else 'MISS'} {FBO_REQUIREMENT}"
    )
    lines.extend(["", "Optional extensions present:"])
    lines.extend(f"  {name}" for name in report["optional_present"] or ["(none)"])
    lines.extend(["", "Optional extensions missing:"])
    lines.extend(f"  {name}" for name in report["optional_missing"] or ["(none)"])
    lines.extend(["", "Limits:"])
    for name, value in sorted(report["limits"].items()):
        lines.append(f"  {name} = {value}")
    if not report["limits"]:
        lines.append("  (none reported; run glxinfo -l)")
    if report["required_missing"]:
        lines.extend(["", "Missing requirements:"])
        lines.extend(f"  {name}" for name in report["required_missing"])
    if report["notes"]:
        lines.extend(["", "Notes:"])
        lines.extend(f"  {note}" for note in report["notes"])
    lines.extend(["", "Extensions:"])
    # Wrap the long list so the text report stays readable in diffs.
    row: list[str] = []
    for name in report["extensions"]:
        row.append(name)
        if len(row) == 4:
            lines.append("  " + " ".join(row))
            row = []
    if row:
        lines.append("  " + " ".join(row))
    return "\n".join(lines) + "\n"


def write_report(report: dict, output_dir: Path, label: str) -> tuple[Path, Path]:
    output_dir.mkdir(parents=True, exist_ok=True)
    json_path = output_dir / f"{label}-caps.json"
    text_path = output_dir / f"{label}-caps.txt"
    json_path.write_text(
        json.dumps(report, indent=2, ensure_ascii=False, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    text_path.write_text(format_text(report), encoding="utf-8")
    return json_path, text_path


def run_glxinfo(args: Iterable[str]) -> str:
    command = ["glxinfo", *args]
    if shutil.which("glxinfo") is None:
        raise FileNotFoundError(
            "glxinfo is not installed (Debian/Ubuntu: mesa-utils). "
            "Capture it on the target machine and use --from-file instead."
        )
    result = subprocess.run(command, capture_output=True, text=True, check=True)
    return result.stdout


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--from-file", type=Path, help="parse an existing glxinfo dump")
    parser.add_argument("--label", default="x1650", help="report label / output basename")
    parser.add_argument(
        "--output-dir", type=Path, default=Path("benchmarks/graphics"),
        help="where <label>-caps.json and <label>-caps.txt are written",
    )
    parser.add_argument(
        "--glxinfo-args", nargs="*", default=["-B", "-l"],
        help="arguments passed to glxinfo (default: -B -l)",
    )
    parser.add_argument("--source-note", default=None, help="override the report's source line")
    parser.add_argument("--no-write", action="store_true", help="do not write report files")
    parser.add_argument("--stdout", action="store_true", help="print the text report")
    parser.add_argument(
        "--check", action="store_true",
        help="exit 1 when the tier is unsupported or a required extension is missing",
    )
    args = parser.parse_args(argv)

    if args.from_file:
        text = args.from_file.read_text(encoding="utf-8", errors="replace")
        source_note = args.source_note or f"file:{args.from_file}"
    else:
        try:
            text = run_glxinfo(args.glxinfo_args)
        except (FileNotFoundError, subprocess.CalledProcessError) as error:
            print(f"gl21_caps: {error}", file=sys.stderr)
            return 2
        source_note = args.source_note or "glxinfo " + " ".join(args.glxinfo_args)

    report = build_report(text, label=args.label, source_note=source_note)
    if not args.no_write:
        json_path, text_path = write_report(report, args.output_dir, args.label)
        print(f"gl21_caps: wrote {json_path} and {text_path}")
    if args.stdout:
        sys.stdout.write(format_text(report))

    problems = check_tier(report)
    print(
        f"gl21_caps: {report['label']} -> {report['tier']} "
        f"(GL {report['gl_version'] or '?'}, GLSL {report['glsl_version'] or '?'})",
        file=sys.stderr,
    )
    if args.check and problems:
        for problem in problems:
            print(f"gl21_caps: {problem}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
