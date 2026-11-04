#!/usr/bin/env python3
"""Check the legacy path's GLSL against the Radeon X1650's budgets, and against GLSL 1.20's
grammar as far as a lexical pass can see it.

    python3 tools/gl21_glsl.py                  # every shader under crates/horizon-client-gl
    python3 tools/gl21_glsl.py --check           # exit 1 on any finding
    python3 tools/gl21_glsl.py --json out.json
    python3 tools/gl21_glsl.py --verbose         # list every declaration

This is the independent half of a pair: `crates/horizon-client-gl/src/budget.rs` measures the
same things at run time, in Rust, and refuses to create a program over budget. Two
implementations of one rule are here on purpose — a bug in the Rust one would hide an
over-budget shader on every machine, and the numbers come from a GPU we do not have.

Budgets (the target's own, from its GLview dump — docs/gl21-x1650-plan.ru.md §1):

    uniform components per stage  512        (no uniform blocks on 2.1: every member is loose)
    varying floats                44
    texture units                 16 per stage, 0 in the vertex stage
    vertex attributes             16
    texture side                  4096 texels
    loops                         compile-time bounds only
    GLSL version                  120 (no in/out, uint, texture(), texelFetch, layout())

Grammar checks are lexical and deliberately conservative: a false positive costs a minute of
reading, a false negative costs a debugging session on a 2006 driver with no debug output.
`glslangValidator`, where it is installed, is the real parser; this runs everywhere.
"""

import argparse
import glob
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SHADER_DIR = ROOT / "crates" / "horizon-client-gl" / "src" / "shaders"

LIMITS = {
    "uniform_components": 512,
    "varying_floats": 44,
    "texture_units": 16,
    "vertex_texture_units": 0,
    "vertex_attributes": 16,
    "max_texture_size": 4096,
}

TYPE_COMPONENTS = {
    "float": 1, "int": 1, "bool": 1,
    "vec2": 2, "vec3": 3, "vec4": 4,
    "ivec2": 2, "ivec3": 3, "ivec4": 4,
    "bvec2": 2, "bvec3": 3, "bvec4": 4,
    "mat2": 4, "mat3": 9, "mat4": 16,
    "sampler2D": 0, "samplerCube": 0, "sampler3D": 0, "sampler1D": 0, "sampler2DShadow": 0,
}
SAMPLERS = {t for t, n in TYPE_COMPONENTS.items() if t.startswith("sampler")}

# Tokens GLSL 1.20 does not have. Each is a real porting mistake, not a style opinion.
FORBIDDEN = [
    (r"\buint\b", "uint: 1.30+"),
    (r"\buvec[234]\b", "uvec: 1.30+"),
    (r"\btexture\s*\(", "texture(): 1.30+ (use texture2D/textureCube)"),
    (r"\btexelFetch\b", "texelFetch: 1.30+"),
    (r"\btextureGrad\b", "textureGrad: 1.30+ (use texture2DGradARB, GL_ARB_shader_texture_lod)"),
    (r"\btextureLod\b", "textureLod: 1.30+ (use texture2DLod / texture2DLodARB)"),
    (r"\btextureSize\b", "textureSize: 1.30+ (pass the size as a uniform)"),
    (r"\bsampler2DArray\b", "sampler2DArray: needs EXT_texture_array, absent on the target"),
    (r"\bsampler2DMS\b", "sampler2DMS: 1.50+"),
    (r"\bgl_VertexID\b", "gl_VertexID: 1.30+"),
    (r"\bgl_InstanceID\b", "gl_InstanceID: 1.40+"),
    (r"\bin\s+vec[234]", "`in`: 1.30+ (use attribute/varying)"),
    (r"\bout\s+vec[234]", "`out`: 1.30+ (use varying and gl_FragColor)"),
    (r"\blayout\s*\(", "layout(): 1.40+ (every uniform is a loose location on 2.1)"),
    (r"\bflat\s+varying\b|\bvarying\s+flat\b", "flat varying: 1.30+ (replicate the value per vertex instead)"),
    (r"\bprecision\s+\w+", "precision qualifiers are GLSL ES, not desktop 1.20"),
    (r"\bswitch\s*\(", "switch: 1.30+"),
    (r"\bbuffer\b\s+\w+\s*\{", "uniform blocks: 1.40+ / ARB_uniform_buffer_object, absent"),
    (r"\bgl_FragData\b", "gl_FragData[1+]: MRT beyond what the path draws; use gl_FragColor"),
    (r"#version\s+(?!120\b)", "#version other than 120 (the backend prepends 120 itself)"),
    (r"\bdouble\b|\bfloat64\b", "double precision: absent on SM 3.0 hardware"),
]

# Names GLSL reserves but that read like ordinary identifiers.
RESERVED = {"asm", "class", "union", "enum", "typedef", "template", "this", "packed", "goto",
            "inline", "noinline", "volatile", "public", "static", "extern", "external",
            "interface", "long", "short", "half", "fixed", "unsigned", "superp", "input",
            "output", "hvec2", "hvec3", "hvec4", "fvec2", "fvec3", "fvec4", "dvec2", "dvec3",
            "dvec4", "sampler1DShadow", "sampler2DRect", "sampler3DRect", "sampler2DRectShadow"}


def strip_comments(text):
    """Removes // and /* */ while keeping line numbers."""
    out, i, n = [], 0, len(text)
    while i < n:
        if text.startswith("//", i):
            while i < n and text[i] != "\n":
                i += 1
        elif text.startswith("/*", i):
            i += 2
            while i < n and not text.startswith("*/", i):
                out.append("\n" if text[i] == "\n" else " ")
                i += 1
            i += 2
        else:
            out.append(text[i])
            i += 1
    return "".join(out)


def strip_directives(text):
    """Drops `#...` lines, keeping the newlines so line numbers stay put.

    The statements are split on `;`, so `#define X 1` lands in the same chunk as whatever
    declaration follows it and hides it: the Rust half of this budget pass had that bug (a
    composed shader begins with `#version` and two `#define`s, so the *first* declaration of
    every shader went uncounted) and this half would have it too.
    """
    return "".join("\n" if line.lstrip().startswith("#") else line
                   for line in text.splitlines(keepends=True))


def split_declaration(statement):
    """`uniform vec2 a, b[4]` -> (kind, type, [(name, count)])."""
    # Qualifiers the parser steps over: `flat varying vec4` fails on the target anyway (the
    # grammar rule below says so), but it must still be measured as a varying, not skipped.
    m = re.match(r"\s*(uniform|varying|attribute)\s+((?:flat|smooth|centroid|invariant|highp|"
                 r"mediump|lowp)\s+)*(\w+)\s+(.*)", statement, re.S)
    if not m:
        return None
    kind, ty, rest = m.group(1), m.group(3), m.group(4)
    names = []
    for part in rest.split(","):
        part = part.strip()
        if not part:
            continue
        name = part.split("[")[0].strip()
        count = 1
        if "[" in part:
            size = part[part.index("[") + 1:part.index("]")].strip()
            count = int(size) if size.isdigit() else 0  # 0: a constant we cannot see
        if name:
            names.append((name, count))
    return kind, ty, names


def measure(path, text):
    """Everything measured and checked about one file."""
    rel = path.relative_to(SHADER_DIR).as_posix() if path.is_relative_to(SHADER_DIR) else path.name
    stage = "vertex" if rel.endswith(".vert") else "fragment"
    body = strip_directives(strip_comments(text))
    code = body.split(";")
    findings = []
    decls = {"uniform": [], "varying": [], "attribute": []}

    for statement in code:
        parsed = split_declaration(statement)
        if not parsed:
            continue
        kind, ty, names = parsed
        if ty not in TYPE_COMPONENTS:
            findings.append((rel, "unknown type", f"{ty} is not a GLSL 1.20 type this checker knows"))
            continue
        for name, count in names:
            decls[kind].append({"name": name, "type": ty, "count": count})

    components = lambda items: sum(TYPE_COMPONENTS[d["type"]] * d["count"] for d in items)
    uniforms = components(decls["uniform"])
    varyings = components(decls["varying"])
    attributes = len(decls["attribute"])
    samplers = sum(1 for d in decls["uniform"] if d["type"] in SAMPLERS)

    if uniforms > LIMITS["uniform_components"]:
        findings.append((rel, "uniform budget",
                         f"{uniforms} components, limit {LIMITS['uniform_components']}"))
    if varyings > LIMITS["varying_floats"]:
        findings.append((rel, "varying budget",
                         f"{varyings} floats, limit {LIMITS['varying_floats']}"))
    if samplers > LIMITS["texture_units"]:
        findings.append((rel, "texture units", f"{samplers} samplers, limit {LIMITS['texture_units']}"))
    if attributes > LIMITS["vertex_attributes"]:
        findings.append((rel, "attributes", f"{attributes}, limit {LIMITS['vertex_attributes']}"))

    if stage == "vertex":
        for name in ("texture2D", "texture2DLod", "texture2DProj", "textureCube", "texture2DGradARB"):
            if re.search(r"\b" + name + r"\s*\(", body):
                findings.append((rel, "vertex texture fetch",
                                 f"{name} in a vertex shader; the target has 0 vertex texture units"))

    for pattern, why in FORBIDDEN:
        for m in re.finditer(pattern, body):
            line = body[:m.start()].count("\n") + 1
            findings.append((rel, "1.20 grammar", f"line {line}: {why}"))

    for m in re.finditer(r"\bfor\s*\(([^)]*)\)", body):
        header = m.group(1)
        cond = header.split(";")[1] if header.count(";") >= 2 else ""
        if "<" in cond or "<=" in cond:
            bound = re.split(r"<", cond, 1)[-1].lstrip("=").strip()
            token = re.match(r"[A-Za-z_][A-Za-z_0-9]*|\d+", bound)
            token = token.group(0) if token else ""
            constant = token.isdigit() or (token.isupper() and token.isidentifier())
            if not constant:
                line = body[:m.start()].count("\n") + 1
                findings.append((rel, "dynamic loop",
                                 f"line {line}: `for ({header.strip()})` needs a compile-time bound"))
    for m in re.finditer(r"\bwhile\s*\(", body):
        line = body[:m.start()].count("\n") + 1
        findings.append((rel, "dynamic loop", f"line {line}: `while` needs a compile-time bound"))

    for m in re.finditer(r"\b(?:uniform|varying|attribute)\s+\w+\s+(\w+)", body):
        if m.group(1) in RESERVED:
            findings.append((rel, "reserved word", f"`{m.group(1)}` is reserved in GLSL"))

    # A declared uniform nothing reads is dead weight on a 512-component budget; a varying
    # nothing reads is the same on 44. A declaration counts as its one occurrence; more than
    # that means something reads it.
    for d in decls["uniform"]:
        uses = len(re.findall(r"\b" + re.escape(d["name"]) + r"\b", body))
        if uses <= 1:
            findings.append((rel, "unused uniform",
                             f"`{d['name']}` is declared but never read (wastes "
                             f"{TYPE_COMPONENTS[d['type']] * d['count']} of {LIMITS['uniform_components']} components)"))
    # A varying is written in one stage and read in the other, so this file's own writes
    # count: only a varying with no occurrence at all past its declaration is dead here.
    for d in decls["varying"]:
        uses = len(re.findall(r"\b" + re.escape(d["name"]) + r"\b", body))
        if uses <= 1:
            findings.append((rel, "unused varying",
                             f"`{d['name']}` is declared but never used in this stage (wastes "
                             f"{TYPE_COMPONENTS[d['type']] * d['count']} of {LIMITS['varying_floats']} floats)"))

    return {
        "path": rel,
        "stage": stage,
        "lines": text.count("\n") + 1,
        "uniform_components": uniforms,
        "varying_floats": varyings,
        "samplers": samplers,
        "attributes": attributes,
        "declarations": decls,
        "findings": [{"file": f, "kind": k, "detail": d} for f, k, d in findings],
    }


def cross_stage(reports, files):
    """A vertex and a fragment source next to each other, named alike, must agree on their
    varyings: same names, same types. A 2.1 driver links a mismatch only with a warning in a
    log nobody reads, and then samples undefined values."""
    findings = []
    by_stem = {}
    for report, path in zip(reports, files):
        stem = str(path.with_suffix(""))
        by_stem.setdefault(stem, {})[report["stage"]] = report
    for stem, pair in sorted(by_stem.items()):
        if set(pair) != {"vertex", "fragment"}:
            continue
        v = {d["name"]: d for d in pair["vertex"]["declarations"]["varying"]}
        f = {d["name"]: d for d in pair["fragment"]["declarations"]["varying"]}
        rel = pair["vertex"]["path"]
        for name in sorted(set(v) - set(f)):
            findings.append((rel, "varying mismatch",
                             f"`{name}` is written by the vertex stage and not declared by the fragment stage"))
        for name in sorted(set(f) - set(v)):
            findings.append((rel, "varying mismatch",
                             f"`{name}` is read by the fragment stage and not written by the vertex stage"))
        for name in sorted(set(v) & set(f)):
            if v[name]["type"] != f[name]["type"]:
                findings.append((rel, "varying mismatch",
                                 f"`{name}` is {v[name]['type']} in the vertex stage and "
                                 f"{f[name]['type']} in the fragment stage"))
    return findings


def shader_files(paths):
    if paths:
        out = []
        for p in paths:
            path = (ROOT / p).resolve()
            if path.is_dir():
                out.extend(sorted(Path(x) for x in glob.glob(str(path / "**" / "*.vert"), recursive=True)))
                out.extend(sorted(Path(x) for x in glob.glob(str(path / "**" / "*.frag"), recursive=True)))
            elif path.suffix in (".vert", ".frag"):
                out.append(path)
        return out
    out = []
    for ext in ("vert", "frag"):
        out.extend(sorted(Path(p) for p in glob.glob(str(SHADER_DIR / "**" / f"*.{ext}"), recursive=True)))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("paths", nargs="*", help="files or directories (default: the crate's shaders)")
    ap.add_argument("--check", action="store_true", help="exit 1 on any finding")
    ap.add_argument("--json", metavar="PATH", help="write the report")
    ap.add_argument("--verbose", action="store_true", help="list every declaration")
    args = ap.parse_args()

    files = shader_files(args.paths)
    if not files:
        print("no shaders found: crates/horizon-client-gl/src/shaders/ is empty")
        return 0 if not args.check else 1

    reports = [measure(p, p.read_text(encoding="utf-8")) for p in files]
    total = sum(len(r["findings"]) for r in reports)
    cross = cross_stage(reports, files)

    print(f"{'shader':44}{'stage':10}{'uniform':>9}{'varying':>9}{'samplers':>9}{'attrs':>7}  findings")
    for r in reports:
        print(f"{r['path']:44}{r['stage']:10}{r['uniform_components']:>9}"
              f"{r['varying_floats']:>9}{r['samplers']:>9}{r['attributes']:>7}  {len(r['findings'])}")
        if args.verbose:
            for kind in ("uniform", "varying", "attribute"):
                for d in r["declarations"][kind]:
                    print(f"    {kind:10} {d['type']:16} {d['name']}"
                          + (f"[{d['count']}]" if d["count"] != 1 else ""))
    if total or cross:
        print()
        for r in reports:
            for f in r["findings"]:
                print(f"{f['file']}: {f['kind']}: {f['detail']}")
        for path, kind, detail in cross:
            print(f"{path}: {kind}: {detail}")
    print()
    total += len(cross)
    print(f"{len(files)} shader(s), {total} finding(s), budgets: "
          f"uniform {LIMITS['uniform_components']}, varying {LIMITS['varying_floats']}, "
          f"samplers {LIMITS['texture_units']}, attributes {LIMITS['vertex_attributes']}")

    if args.json:
        Path(args.json).write_text(json.dumps({"limits": LIMITS, "shaders": reports}, indent=2) + "\n")
        print(f"written: {args.json}")

    return 1 if (args.check and total) else 0


if __name__ == "__main__":
    sys.exit(main())
