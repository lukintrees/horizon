#!/usr/bin/env python3
"""Check the renderer against the OpenGL 2.1 budget of Horizon's oldest target GPU.

The target is an ATI Radeon X1650 (RV535, 2006) on Windows 7 x86 with Catalyst
8.591 (9.3, the last driver for the chip): OpenGL 2.1, GLSL 1.20, Direct3D 9
Shader Model 3.0 underneath. The numbers below are that driver's own limits, as
it reports them (vgamuseum.info's GLview dump of "Radeon X1600/X1650 Series",
Version 2.1.8545; the same figures in AMD's Catalyst release notes):

  max texture size              4096 x 4096
  max 3D texture size           2048
  texture array layers          none (no GL_EXT_texture_array)
  max vertex attributes         16
  max vertex texture image units 0        <- no vertex texture fetch at all
  max fragment texture units    16
  vertex/fragment uniform comps 512 / 512
  max varying floats            44
  max draw buffers              4
  max FBO size                  4096
  vertex/fragment instructions  512 native each (SM 3.0 slots), static loops only
  texture render targets        no EXT_color_buffer_float (16F/32F targets absent)
  MSAA on FBOs                  2x/4x/6x via EXT_framebuffer_multisample

Mesa's llvmpipe, where this is tested on Linux, reports OpenGL 4.5: it accepts
everything in this list, so it proves what a port *draws*, never that it fits the
X1650. That is why every limit here is a static check: the CI-visible part of
the port is this file, and a device that is not an X1650 cannot check it for us.

    python3 tools/gl21_budget.py                    # report, exits 0
    python3 tools/gl21_budget.py --check             # exits 1 when over budget
    python3 tools/gl21_budget.py --json out.json     # machine-readable report
    python3 tools/gl21_budget.py --baseline base.json  # diff against a saved report
    python3 tools/gl21_budget.py --profile gl33      # what the current wgpu path needs

`--profile gl33` describes what the renderer needs today (desktop GL 3.3, wgpu's
OpenGL floor), so the two profiles' difference is the size of the port.
"""

import argparse
import glob
import json
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RENDER = ROOT / "crates" / "horizon-client-render" / "src"

# ---------------------------------------------------------------------------
# Profiles. `None` means "not limited" (the GL 3.3 profile).

GL21 = {
    "name": "gl21-x1650",
    "description": "Radeon X1650, Catalyst 9.3, Windows 7 x86: GL 2.1 / GLSL 1.20",
    "glsl_version": 120,
    "vertex_texture_units": 0,
    "fragment_texture_units": 16,
    "combined_texture_units": 16,
    "vertex_uniform_components": 512,
    "fragment_uniform_components": 512,
    "varying_floats": 44,
    "vertex_attributes": 16,
    "draw_buffers": 4,
    "max_texture_size": 4096,
    "max_3d_texture_size": 2048,
    # Things the API simply has not got.
    "texture_arrays": False,
    "texture_3d": False,  # 3D textures exist (GL 1.2/2.0) but shaders may not sample per-layer
    "storage_buffers": False,
    "compute_shaders": False,
    "indirect_draw": False,
    "instanced_arrays_with_divisor": False,  # ARB_instanced_arrays absent on this driver
    "float_render_targets": False,
    "integer_render_targets": False,
    "pixel_shader_instructions": 512,
    "dynamic_loops": False,
}

GL33 = {
    "name": "gl33",
    "description": "wgpu's OpenGL floor today: desktop GL 3.3 / GLSL 3.30",
    "glsl_version": 330,
    "vertex_texture_units": 16,
    "fragment_texture_units": 16,
    "combined_texture_units": 32,
    "vertex_uniform_components": 4096,
    "fragment_uniform_components": 4096,
    "varying_floats": 60,
    "vertex_attributes": 16,
    "draw_buffers": 8,
    "max_texture_size": 16384,
    "max_3d_texture_size": 2048,
    "texture_arrays": True,
    "texture_3d": True,
    "storage_buffers": True,  # GLSL 1.50+/GL 3.1 for SSBO; wgpu gates it
    "compute_shaders": False,  # GL 4.3
    "indirect_draw": True,  # GL 4.0
    "instanced_arrays_with_divisor": True,
    "float_render_targets": True,
    "integer_render_targets": True,
    "pixel_shader_instructions": None,
    "dynamic_loops": True,
}

PROFILES = {p["name"]: p for p in (GL21, GL33)}

# WGSL type -> 4-byte components when it sits in a uniform block.
SCALARS = {"f32": 1, "i32": 1, "u32": 1, "atomic<i32>": 1, "atomic<u32>": 1}


def components(ty):
    """4-byte components of a WGSL type inside std140, or None if unknown."""
    ty = ty.strip()
    if ty in SCALARS:
        return SCALARS[ty]
    m = re.fullmatch(r"mat([234])x\1<f32>", ty)
    if m:
        return int(m.group(1)) * 4
    m = re.fullmatch(r"vec([234])<(\w+)>", ty)
    if m:
        return int(m.group(1))
    m = re.fullmatch(r"array<(.+),\s*(\d+)>", ty)
    if m:
        inner = components(m.group(1))
        return None if inner is None else inner * int(m.group(2))
    return None


def struct_fields(text, name):
    """The members of a WGSL struct, each as its own declaration text."""
    m = re.search(r"struct\s+%s\s*\{" % re.escape(name), text)
    if not m:
        return []
    depth, i = 1, m.end()
    while i < len(text) and depth:
        depth += {"{": 1, "}": -1}.get(text[i], 0)
        i += 1
    body = text[m.end():i - 1]
    body = re.sub(r"//[^\n]*", "", body)
    fields = []
    for part in split_top_level(body):
        part = part.strip()
        if ":" in part:
            fields.append(part)
    return fields


def split_top_level(body):
    """Split a WGSL struct body on the commas that are not inside `<...>` or `(...)`."""
    parts, depth, cur = [], 0, []
    for ch in body:
        if ch in "<(":
            depth += 1
        elif ch in ">)":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    parts.append("".join(cur))
    return parts


def field_type(field):
    """The type of one member declaration: everything after its top-level colon."""
    depth = 0
    for i, ch in enumerate(field):
        if ch in "<(":
            depth += 1
        elif ch in ">)":
            depth -= 1
        elif ch == ":" and depth == 0:
            return field[i + 1:].strip()
    return ""


def field_components(field):
    return components(field_type(field))


class Finding:
    def __init__(self, category, where, detail, needs):
        self.category = category
        self.where = where
        self.detail = detail
        self.needs = needs  # what the port must do instead

    def as_dict(self):
        return {"category": self.category, "where": self.where,
                "detail": self.detail, "needs": self.needs}


def scan_shaders():
    """Per-shader facts, straight from the WGSL."""
    shaders = []
    for path in sorted(glob.glob(str(RENDER / "**" / "*.wgsl"), recursive=True)):
        rel = str(Path(path).relative_to(RENDER))
        text = Path(path).read_text(encoding="utf-8", errors="replace")
        stripped = re.sub(r"//[^\n]*", "", text)

        bindings = []
        for m in re.finditer(
                r"@group\((\d+)\)\s*@binding\((\d+)\)\s*var(?:<(\w+)>)?\s+(\w+)\s*:\s*"
                r"(texture_\w+|sampler(?:_comparison)?[^;]*|.+)", stripped):
            space = m.group(3) or "uniform"
            # `var<uniform> globals: Globals;` captures "Globals;" for the fallback arm.
            ty = m.group(5).strip().rstrip(";").strip()
            bindings.append({"space": space, "type": ty, "name": m.group(4)})

        uniforms = {}
        for b in bindings:
            if b["space"] == "uniform":
                fields = struct_fields(stripped, b["type"])
                if fields:
                    uniforms[b["type"]] = sum(field_components(f) or 0 for f in fields)

        varyings = {}
        for m in re.finditer(r"struct\s+(\w+)\s*\{", stripped):
            name = m.group(1)
            if name in uniforms:
                continue
            fields = struct_fields(stripped, name)
            located = [f for f in fields if "@location(" in f]
            if located:
                varyings[name] = sum(field_components(f) or 0 for f in located)

        stages = []
        marks = [(m.start(), m.group()) for m in
                 re.finditer(r"@(vertex|fragment|compute)\b", stripped)]
        for i, (pos, kind) in enumerate(marks):
            end = marks[i + 1][0] if i + 1 < len(marks) else len(stripped)
            seg = stripped[pos:end]
            nm = re.search(r"fn\s+(\w+)", seg)
            samples = sorted(set(re.findall(
                r"textureSample\w*|textureLoad|textureGather|textureDimensions", seg)))
            loops = len(re.findall(r"\bloop\b|\bfor\s*\(|\bwhile\b", seg))
            stages.append({"kind": kind.lstrip("@"), "name": nm and nm.group(1),
                           "samples_textures": samples, "loops": loops})

        shaders.append({
            "path": rel,
            "lines": text.count("\n") + 1,
            "storage_buffers": len(re.findall(r"var<storage", stripped)),
            "texture_arrays": len(re.findall(r"texture_\w+_array", stripped)),
            "texture_3d": len(re.findall(r"texture_3d", stripped)),
            "atomic_ops": len(re.findall(r"atomic\w*\(", stripped)),
            "workgroup": len(re.findall(r"var<workgroup", stripped)),
            "uniform_structs": uniforms,
            "varyings": varyings,
            "bindings": bindings,
            "stages": stages,
        })
    return shaders


def scan_rust():
    """Renderer facts the shaders cannot show: formats, state, draw model."""
    facts = {
        "float_render_targets": {},
        "wide_unorm_targets": {},
        "integer_render_targets": {},
        "depth_formats": {},
        "sample_counts": {},
        "depth_compare_samplers": 0,
        "multiview": 0,
        "indirect_draws": 0,
        "instanced_draws": 0,
        "compute_dispatch": 0,
        "bind_groups": 0,
        "bindings_per_group": 0,
        "texture_2d_array": 0,
        "texture_3d": 0,
        "storage_buffers_in_shaders": 0,
    }
    patterns = {
        "float_render_targets": r"Rgba16Float|Rgba32Float|Rg16Float|Rg32Float|R16Float|R32Float|Rg11b10\w+",
        "wide_unorm_targets": r"Rgb10a2Unorm",
        "integer_render_targets": r"R32Uint|Rgba32Uint|Rgba16Uint|R32Sint|Rgba32Sint",
        "depth_formats": r"Depth24PlusStencil8|Depth32Float|Depth24Plus|Depth16Unorm",
        "sample_counts": r"sample_count:\s*(\d+)",
        "depth_compare_samplers": r"depth_compare:\s*\w*Comparison",
        "multiview": r"multiview_mask:\s*Some|multiview:\s*(?:true|Mask)",
        "indirect_draws": r"\.draw_indirect\(|\.draw_indexed_indirect\(",
        "instanced_draws": r"\.draw_indexed\([^)]*,\s*\d+\.\.|\.draw\([^)]*,\s*\d+\.\.",
        "compute_dispatch": r"dispatch_workgroups\(",
        "bind_groups": r"create_bind_group\(",
        "texture_2d_array": r"TextureViewDimension::D2Array",
        "texture_3d": r"TextureViewDimension::D3\b|dimension:\s*wgpu::TextureViewDimension::D3\b",
    }
    for path in sorted(glob.glob(str(RENDER / "**" / "*.rs"), recursive=True)):
        try:
            text = Path(path).read_bytes().decode("utf-8", "replace")
        except OSError:
            continue
        for key, pat in patterns.items():
            found = re.findall(pat, text)
            if not found:
                continue
            if isinstance(facts[key], dict):
                for f in found:
                    facts[key][f] = facts[key].get(f, 0) + 1
            elif key == "sample_counts":
                for f in found:
                    facts[key][f] = facts[key].get(f, 0) + 1
            else:
                facts[key] += len(found)
    return facts


def report(shaders, rust, profile):
    p = profile
    findings = []
    worst_uniform = 0
    worst_uniform_where = ""
    worst_varying = 0
    worst_varying_where = ""
    vt_units = 0
    max_textures_used = 0
    max_textures_where = ""

    for s in shaders:
        for struct, comps in s["uniform_structs"].items():
            if comps > worst_uniform:
                worst_uniform, worst_uniform_where = comps, f"{s['path']} ({struct})"
        for struct, comps in s["varyings"].items():
            if comps > worst_varying:
                worst_varying, worst_varying_where = comps, f"{s['path']} ({struct})"
        textures = [b for b in s["bindings"]
                    if b["type"].startswith(("texture_", "sampler"))]
        if len(textures) > max_textures_used:
            max_textures_used, max_textures_where = len(textures), s["path"]
        for st in s["stages"]:
            if st["kind"] == "vertex" and st["samples_textures"]:
                vt_units += 1
                findings.append(Finding(
                    "vertex texture fetch", f"{s['path']} @vertex {st['name']}",
                    ", ".join(st["samples_textures"]),
                    "compute the value on the CPU or from a uniform; the X1650 has no "
                    "vertex texture units"))
            if st["kind"] == "compute":
                findings.append(Finding(
                    "compute shader", f"{s['path']} @compute {st['name']}", "",
                    "move to a render-pass ping-pong, or keep it behind the GL3 path"))
            if st["loops"] and not p["dynamic_loops"]:
                findings.append(Finding(
                    "loops", f"{s['path']} @{st['kind']} {st['name']}",
                    f"{st['loops']} loop(s)",
                    "GLSL 1.20 on SM 3.0 rejects data-dependent loops: unroll, or bound "
                    "them at compile time with a constant and `break`"))
        if s["storage_buffers"] and not p["storage_buffers"]:
            findings.append(Finding(
                "storage buffer", s["path"], f"{s['storage_buffers']} declaration(s)",
                "use a uniform block or a texture; GL 2.1 has no SSBO"))
        if s["texture_arrays"] and not p["texture_arrays"]:
            findings.append(Finding(
                "texture array", s["path"], f"{s['texture_arrays']} declaration(s)",
                "split per layer (sampler decisions on the CPU) or pack into an atlas; "
                "GL 2.1 has no array textures"))
        if s["texture_3d"] and not p["texture_3d"]:
            findings.append(Finding(
                "3D texture", s["path"], "",
                "stack the volume in a 2D atlas"))
        if s["workgroup"]:
            findings.append(Finding("workgroup memory", s["path"], "", "no compute on GL 2.1"))

    if worst_uniform > p["vertex_uniform_components"]:
        findings.append(Finding(
            "uniform block", worst_uniform_where,
            f"{worst_uniform} components (limit {p['vertex_uniform_components']})",
            "split the block across stages, pack four u32 into one vec4, and move "
            "array data (lights, emissive layers, sunrise fan) into a texture"))
    if worst_varying > p["varying_floats"]:
        findings.append(Finding(
            "varyings", worst_varying_where,
            f"{worst_varying} floats (limit {p['varying_floats']})",
            "pack to at most 44 floats: drop colour to 1 packed u32, fold flags into one"))
    if max_textures_used > p["combined_texture_units"]:
        findings.append(Finding(
            "texture units", max_textures_where,
            f"{max_textures_used} bindings (limit {p['combined_texture_units']})",
            "merge samplers and put sprite arrays back into one atlas"))

    if rust["float_render_targets"] and not p["float_render_targets"]:
        findings.append(Finding(
            "float render target", "gfx*.rs",
            ", ".join(f"{k} x{v}" for k, v in sorted(rust["float_render_targets"].items())),
            "render to RGBA8 (or 10:10:10:2) and keep HDR/sky maths in the shader; "
            "the target GPU has no EXT_color_buffer_float"))
    if rust["indirect_draws"] and not p["indirect_draw"]:
        findings.append(Finding(
            "indirect draw", "gfx*.rs", f"{rust['indirect_draws']} call(s)",
            "draw from CPU-built ranges (GL 2.1 has no indirect draw)"))
    if rust["compute_dispatch"] and not p["compute_shaders"]:
        findings.append(Finding(
            "compute pass", "gfx*.rs", f"{rust['compute_dispatch']} dispatch(es)",
            "render-pass replacements behind the GL3 path"))
    if rust["sample_counts"]:
        multi = {k: v for k, v in rust["sample_counts"].items() if k != "1"}
        if multi and p["name"].startswith("gl21"):
            findings.append(Finding(
                "MSAA", "gfx*.rs", ", ".join(f"{k}x x{v}" for k, v in multi.items()),
                "keep sample_count 1: 4x MSAA on a 256 MB, 2006 GPU at 1080p is not free"))

    return {
        "profile": p["name"],
        "shaders": len(shaders),
        "shader_lines": sum(s["lines"] for s in shaders),
        "vertex_stages_sampling_textures": vt_units,
        "worst_uniform_block": {"where": worst_uniform_where, "components": worst_uniform,
                                "limit": p["vertex_uniform_components"]},
        "worst_varying_struct": {"where": worst_varying_where, "floats": worst_varying,
                                 "limit": p["varying_floats"]},
        "most_texture_bindings": {"where": max_textures_where, "count": max_textures_used,
                                  "limit": p["combined_texture_units"]},
        "storage_buffers_in_shaders": sum(s["storage_buffers"] for s in shaders),
        "texture_array_decls": sum(s["texture_arrays"] for s in shaders),
        "texture_3d_decls": sum(s["texture_3d"] for s in shaders),
        "atomic_ops": sum(s["atomic_ops"] for s in shaders),
        "compute_entry_points": sum(1 for s in shaders for st in s["stages"]
                                    if st["kind"] == "compute"),
        "rust": rust,
        "findings": [f.as_dict() for f in findings],
        "resources": {
            "render_type": "light",
            "fragment_texture_slots": 256,
            "pipeline_keys": 60,
            "output_width": 1366,
        },
    }


def print_report(rep):
    r = rep
    print(f"profile: {r['profile']}")
    print(f"shaders: {r['shaders']} files, {r['shader_lines']} lines")
    if r["profile"].startswith("gl21"):
        print("GL 2.1 has no uniform blocks at all: every member below is a loose, "
              "per-program uniform.")
    print()
    print(f"{'limit':34}{'worst':>10}{'allowed':>10}  where")
    for key, label in (("worst_uniform_block", "uniform components"),
                       ("worst_varying_struct", "varying floats"),
                       ("most_texture_bindings", "texture bindings")):
        w = r[key]
        got, cap = (w["components"] if "components" in w else
                    w["floats"] if "floats" in w else w["count"]), w["limit"]
        flag = "  OVER" if cap and got > cap else ""
        print(f"{label:34}{got:>10}{cap:>10}  {w['where']}{flag}")
    print(f"{'vertex shaders sampling textures':34}{r['vertex_stages_sampling_textures']:>10}")
    print(f"{'storage buffers in shaders':34}{r['storage_buffers_in_shaders']:>10}")
    print(f"{'array/3D texture declarations':34}{r['texture_array_decls'] + r['texture_3d_decls']:>10}")
    print(f"{'compute entry points':34}{r['compute_entry_points']:>10}")
    print()
    rust = r["rust"]
    print("renderer (Rust):")
    for label, value in (("float render targets", rust["float_render_targets"]),
                         ("integer render targets", rust["integer_render_targets"]),
                         ("depth formats", rust["depth_formats"]),
                         ("sample counts", rust["sample_counts"]),
                         ("depth-compare samplers", rust["depth_compare_samplers"]),
                         ("multiview", rust["multiview"]),
                         ("indirect draws", rust["indirect_draws"]),
                         ("instanced draws", rust["instanced_draws"]),
                         ("compute dispatches", rust["compute_dispatch"]),
                         ("bind groups created", rust["bind_groups"]),
                         ("texture_2d_array sites", rust["texture_2d_array"]),
                         ("texture_3d sites", rust["texture_3d"])):
        if value:
            print(f"  {label:32}{value}")
    print()
    by_cat = {}
    for f in r["findings"]:
        by_cat.setdefault(f["category"], []).append(f)
    print(f"findings: {len(r['findings'])}")
    for cat, items in sorted(by_cat.items(), key=lambda kv: -len(kv[1])):
        print(f"  {cat:24}{len(items):>4}   e.g. {items[0]['where']}"
              + (f": {items[0]['detail']}" if items[0]["detail"] else ""))
    print()
    print("what each finding needs, by category:")
    for cat, items in sorted(by_cat.items()):
        print(f"  {cat}: {items[0]['needs']}")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true", help="exit 1 when over budget")
    ap.add_argument("--json", metavar="PATH", help="write the report here")
    ap.add_argument("--baseline", metavar="PATH", help="diff the report against this one")
    ap.add_argument("--profile", default="gl21-x1650", choices=sorted(PROFILES))
    args = ap.parse_args()

    shaders = scan_shaders()
    rust = scan_rust()
    rep = report(shaders, rust, PROFILES[args.profile])
    print_report(rep)

    if args.json:
        Path(args.json).write_text(json.dumps(rep, indent=2) + "\n")
        print(f"\nwritten: {args.json}")

    status = 0
    if args.baseline:
        base = json.loads(Path(args.baseline).read_text())
        before = {(f["category"], f["where"]) for f in base["findings"]}
        after = {(f["category"], f["where"]) for f in rep["findings"]}
        fixed, added = sorted(before - after), sorted(after - before)
        print(f"\nagainst {args.baseline}: {len(fixed)} fixed, {len(added)} new")
        for cat, where in fixed:
            print(f"  fixed: {cat} @ {where}")
        for cat, where in added:
            print(f"  NEW:   {cat} @ {where}")
        if added:
            status = 1
        if args.baseline and not args.check and added:
            pass  # --baseline alone reports, does not fail; --check decides

    if args.check and rep["findings"]:
        print(f"\nOVER BUDGET: {len(rep['findings'])} finding(s) for {rep['profile']}")
        status = 1
    return status


if __name__ == "__main__":
    sys.exit(main())
