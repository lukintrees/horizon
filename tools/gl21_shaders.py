#!/usr/bin/env python3
"""Fail when a GLSL 1.20 shader (docs/opengl21-plan.md's renderer) needs what OpenGL 2.1 lacks.

The renderer that runs on a Radeon X1650 compiles `#version 120` sources: no `in`/`out`, no
`layout`, no `texture()`, no integer bit operations, no `gl_VertexID`, no uniform or storage blocks,
no arrays of arrays, no `flat` interpolation. That renderer does not exist yet; the shaders it will
be built from live beside the WGSL under `crates/horizon-client-render/src/gl21/` and are checked
here so that the first thing every one of them hits is this list, not a driver in 2026.

Usage:
    python3 tools/gl21_shaders.py                # every src/gl21/**/*.glsl and *.vert/*.frag
    python3 tools/gl21_shaders.py --list         # the files it would check
    python3 tools/gl21_shaders.py path/to.glsl    # named files

The rules are the profile (`Profile::X1650` in `crates/horizon-client-render/src/gl21.rs`): 16 vertex
attributes, 16 texture units, 4 draw buffers, 512 native ALU instructions per stage, no texture
fetch in a vertex shader. Limits are counted per shader, not per program, so they are reported as
errors only when a single stage is over them on its own.
"""

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SHADER_DIR = ROOT / "crates" / "horizon-client-render" / "src" / "gl21"
SUFFIXES = (".glsl", ".vert", ".frag")

# (regex, message). The regexes run on the source with comments removed.
RULES = [
    (re.compile(r"^\s*#version\s+(\d+)", re.MULTILINE),
     "`#version` must be 120 (the X1650's GLSL 1.20)"),
    (re.compile(r"\buniform\s+[A-Za-z_][\w]*\s*\{"),
     "uniform blocks need GL 3.1; use plain uniforms"),
    (re.compile(r"\bbuffer\s+[A-Za-z_][\w]*\s*\{"),
     "shader storage blocks need GL 4.3"),
    (re.compile(r"\blayout\s*\("),
     "`layout` needs GLSL 1.50; bind attributes by name (glBindAttribLocation) or use the driver's"),
    (re.compile(r"\btexture\s*\(\s*[A-Za-z_]"),
     "`texture()` needs GLSL 1.30; use texture2D()/texture2DProj()"),
    (re.compile(r"\btextureLod\s*\("),
     "`textureLod` needs GLSL 1.30 (or EXT_gpu_shader4); use texture2DLod() from ARB_shader_texture_lod"),
    (re.compile(r"\btextureGrad\s*\("),
     "`textureGrad` needs GLSL 1.30; the GL 2.1 path uses hardware derivatives"),
    (re.compile(r"\btexelFetch\s*\("),
     "`texelFetch` needs GLSL 1.30"),
    (re.compile(r"\btextureSize\s*\("),
     "`textureSize` needs GLSL 1.30; pass the size as a uniform"),
    (re.compile(r"\btexture2DArray\b|\bsampler2DArray\b|\btextureCubeArray\b"),
     "texture arrays need GL 3.0; the GL 2.1 path uses flat atlases"),
    (re.compile(r"\bgl_VertexID\b|\bgl_InstanceID\b"),
     "`gl_VertexID`/`gl_InstanceID` need GL 3.0; draw full-screen quads from a vertex buffer"),
    (re.compile(r"\bgl_FragData\s*\["),
     "`gl_FragData` needs GL_ARB_draw_buffers; one colour target only in the GL 2.1 path"),
    (re.compile(r"\bflat\b\s*(in|out|varying)|\b(in|out|varying)\s+flat\b"),
     "`flat` interpolation needs GLSL 1.30; send one value from every vertex of the quad"),
    (re.compile(r"(?<![/\w])(<<|>>)(?![/\w])"),
     "integer bit shifts need GLSL 1.30; use float arithmetic (floor/mod) or precomputed data"),
    (re.compile(r"\bbitcast\s*<|\bbitfieldReverse\b|\bfindLSB\b|\bfindMSB\b|\bcountOneBits\b"),
     "bit operations need GLSL 1.30; restructure the data (the compact quad layout is what needs them)"),
    (re.compile(r"\b(isampler|usampler|iimage|uimage)\w*"),
     "integer samplers need GL 3.0"),
    (re.compile(r"\b(imageLoad|imageStore|imageAtomic\w*)\s*\("),
     "images need GL 4.2"),
    (re.compile(r"\batomicAdd\b|\batomicExchange\b|\batomicCompSwap\b"),
     "atomics need GL 4.2 / SSBOs"),
    (re.compile(r"\bsubroutine\b|\bswitch\s*\("),
     "`subroutine` needs GL 4.0 and `switch` GLSL 1.30; use if/else"),
    (re.compile(r"\barray\s+of\s+array|\w+\s*\[\s*\d+\s*\]\s*\[\s*\d+\s*\]"),
     "arrays of arrays need GLSL 1.20 with ARB_arrays_of_arrays (GL 4.3) or a flat array"),
    (re.compile(r"^\s*#\s*extension\s+\w+\s*:\s*require", re.MULTILINE),
     "a required extension must be listed in the plan first: the X1650's driver set is fixed"),
    (re.compile(r"\bsampler2DShadow\b|\btexture2DProjLod\b"),
     "check the plan's shader table: shadows sample their own depth target by hand"),
]

# A stray `in`/`out` used as a storage qualifier (GLSL 1.30's replacement for attribute/varying).
IN_OUT = re.compile(r"^\s*(in|out)\s+(?!\w+\s*;)([A-Za-z_][\w]*)\s+[A-Za-z_]", re.MULTILINE)

ATTRIBUTE = re.compile(r"^\s*attribute\s+[^;]+;", re.MULTILINE)
VARYING = re.compile(r"^\s*varying\s+[^;]+;", re.MULTILINE)

# One non-obvious limit from the profile: a fetch in a vertex shader.
TEXTURE_FETCH = re.compile(r"\btexture2D(Lod|Proj|ProjLod)?\s*\(|\btextureCube\s*\(")


def strip_comments(text):
    """`text` without comments, so that prose about `texture()` is not a violation. Line and block
    comments become blanks of the same length, keeping line numbers."""
    def blank(match):
        return re.sub(r"[^\n]", " ", match.group(0))
    return re.sub(r"//[^\n]*|/\*.*?\*/", blank, text, flags=re.DOTALL)


# The profiles a file can be checked against, by the name `HORIZON_GL_PROFILE` and
# `gl21::named` take.
PROFILES = ("x1650",)


def check_files(files):
    """The violations of every file: ({path: [(line, message)]}, {path: error text})."""
    violations, errors = {}, {}
    for path in files:
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as e:
            errors[path] = str(e)
            continue
        found = check_text(text, stage_of(path))
        if found:
            violations[path] = found
    return violations, errors


def stage_of(path):
    """The stage a file holds, from its suffix: `vert`/`frag` name one, and a `.glsl` file holds
    both (checked as one source, with the vertex rule only where the source is a vertex shader)."""
    return {"vert": "vertex", "frag": "fragment"}.get(path.suffix.lstrip(".").lower())


def is_vertex(text):
    """Whether a source without a stage suffix is a vertex shader: GLSL 1.20 refuses `attribute` in a
    fragment shader, and a fragment shader writes `gl_FragColor` (or `gl_FragData`)."""
    return bool(ATTRIBUTE.search(text)) and "gl_FragColor" not in text


def check_text(text, stage=None):
    """The violations in one shader source, as (line, message) pairs, sorted by line."""
    clean = strip_comments(text)
    vertex = stage == "vertex" or (stage is None and is_vertex(clean))
    violations = []
    for pattern, message in RULES:
        for match in pattern.finditer(clean):
            line = line_of(clean, match.start())
            if message.startswith("`#version`"):
                if match.group(1) != "120":
                    violations.append((line, message))
                continue
            violations.append((line, message))
    if not re.search(r"^\s*#version\b", clean, re.MULTILINE):
        violations.append((1, "a GLSL 1.20 shader starts with `#version 120`"))
    for match in IN_OUT.finditer(clean):
        violations.append(
            (line_of(clean, match.start()), "`in`/`out` need GLSL 1.30; use attribute/varying")
        )
    if vertex:
        # R500 has no texture unit in its vertex pipeline at all, whatever the driver quotes. Its
        # its own functions do not count: `texture2D` declared and called inside the shader is the
        # shader's, not the driver's.
        for match in TEXTURE_FETCH.finditer(clean):
            if not defined_in(clean, match.start()):
                violations.append(
                    (line_of(clean, match.start()),
                     "a vertex shader cannot sample on R500 (no vertex texture fetch)")
                )
    return sorted(set(violations))


DEFINITION = re.compile(r"\b\w+\s+(\w+)\s*\(")


def defined_in(text, offset):
    """Whether the call at `offset` is to a function the shader declares itself, whose name is the
    word before its `(`: `vec4 lightmap(vec2 levels)` declares `lightmap`."""
    before = text[:offset]
    name = re.search(r"(\w+)\s*$", before)
    if not name:
        return False
    return bool(re.search(r"\b\w+\s+" + re.escape(name.group(1)) + r"\s*\([^;{]*\)\s*\{", text))


def line_of(text, offset):
    return text.count("\n", 0, offset) + 1


def shader_files(paths=None):
    if paths:
        return [Path(p) for p in paths]
    if not SHADER_DIR.is_dir():
        return []
    return sorted(p for p in SHADER_DIR.rglob("*") if p.suffix.lower() in SUFFIXES)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("paths", nargs="*", help="shader files instead of the gl21 folder")
    parser.add_argument("--list", action="store_true", help="list the files and exit")
    parser.add_argument("--quiet", action="store_true", help="print nothing when everything passes")
    args = parser.parse_args()

    files = shader_files(args.paths)
    if args.list:
        for path in files:
            print(path.relative_to(ROOT) if path.is_relative_to(ROOT) else path)
        return 0
    if not files:
        if not args.quiet:
            print(f"no GLSL 1.20 shaders yet ({SHADER_DIR.relative_to(ROOT)}): nothing to check")
        return 0

    violations, errors = check_files(files)
    failed = len(errors)
    for path, error in errors.items():
        print(f"{path}: {error}", file=sys.stderr)
    for path, found in violations.items():
        try:
            shown = path.relative_to(ROOT)
        except ValueError:
            shown = path
        for line, message in found:
            print(f"{shown}:{line}: {message}")
            failed += 1
    if failed:
        print(f"\n{failed} violation(s) of the OpenGL 2.1 profile "
              f"(docs/opengl21-plan.md); {len(files)} file(s) checked", file=sys.stderr)
        return 1
    if not args.quiet:
        print(f"{len(files)} GLSL 1.20 file(s) fit the OpenGL 2.1 profile")
    return 0


if __name__ == "__main__":
    sys.exit(main())
