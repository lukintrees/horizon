#!/usr/bin/env python3
"""Static source audit of legacy GL2 material codes and Frame/HUD flags (M01a).

Reads the repository sources only; it executes no Rust and needs no GPU. Every catalog
row in tests/fixtures/legacy_material_audit/expectations.json is checked against the
source text: producer snippets must exist, constants must have the values written in
the fixture, the legacy overlay accept/reject sets must match what entity.rs parses,
and the Frame blockers must match frame.rs `unsupported()`.

Usage: python3 tools/legacy_material_audit.py [--root DIR] [--expectations FILE] [--report FILE]
Exit 0 when every expectation holds, 1 otherwise. The JSON report is written to
target/legacy-material-audit/report.json (ignored by Git).
"""

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
DEFAULT_EXPECTATIONS = REPO / "tests/fixtures/legacy_material_audit/expectations.json"
DEFAULT_REPORT = REPO / "target/legacy-material-audit/report.json"


def norm(text):
    """Whitespace-insensitive form used for snippet matching."""
    return re.sub(r"\s+", " ", text).strip()


def strip_line_comments(text):
    """Drops `//` comments so a snippet inside a comment does not count as producer code."""
    return "\n".join(line.split("//", 1)[0] for line in text.splitlines())


def expand_pattern(token):
    """`16..=31` -> [16..31]; `37` -> [37]; anything else -> []."""
    token = token.strip()
    m = re.fullmatch(r"(\d+)\s*\.\.=\s*(\d+)", token)
    if m:
        return list(range(int(m.group(1)), int(m.group(2)) + 1))
    if re.fullmatch(r"\d+", token):
        return [int(token)]
    return []


def block_after(text, marker):
    """Returns the brace-balanced block that opens right after `marker` (exclusive of braces)."""
    start = text.index(marker) + len(marker)
    depth = 0
    for i in range(start, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            if depth == 0:
                return text[start:i]
            depth -= 1
    raise ValueError(f"unbalanced block after {marker!r}")


def parse_overlay_arms(entity_src):
    """The `let overlay = match code { ... }` arms of entity.rs: list of (codes, guard)."""
    body = block_after(strip_line_comments(entity_src), "let overlay = match code {")
    arms = []
    for raw in body.split("\n"):
        line = raw.strip()
        if "=>" not in line:
            continue
        pattern, _ = line.split("=>", 1)
        guard = None
        g = re.search(r"\bif\s+(\w+)\s*$", pattern.strip())
        if g:
            guard = g.group(1)
            pattern = pattern[: g.start()]
        codes = []
        for part in pattern.split("|"):
            codes.extend(expand_pattern(part))
        arms.append((codes, guard, pattern.strip() == "_"))
    return arms


def parse_additive_allowed(entity_src):
    """`if additive && !matches!(code, 0 | 2 | 3 | 37)`: the codes an additive view accepts."""
    m = re.search(r"if additive && !matches!\(code,([^)]*)\)", strip_line_comments(entity_src))
    if not m:
        raise ValueError("additive guard not found")
    return sorted({c for part in m.group(1).split("|") for c in expand_pattern(part)})


def parse_glint_codes(entity_src):
    m = re.search(r"if matches!\(code,([^)]*)\)\s*\{\s*let state", strip_line_comments(entity_src))
    if not m:
        raise ValueError("glint code guard not found")
    return sorted({c for part in m.group(1).split("|") for c in expand_pattern(part)})


def legacy_entity_sets(entity_src):
    """What entity::runs_view accepts, derived from its match arms (not from the fixture)."""
    arms = parse_overlay_arms(entity_src)
    plain = set()
    additive_arms = set()
    for codes, guard, is_wild in arms:
        if is_wild:
            continue
        if guard is None:
            plain.update(codes)
        elif guard == "additive":
            additive_arms.update(codes)
    allowed = set(parse_additive_allowed(entity_src))
    return {
        "nonadditive": sorted(plain),
        "additive": sorted((plain | additive_arms) & allowed),
        "glint_required": parse_glint_codes(entity_src),
    }


def parse_frame_blockers(frame_src):
    body = block_after(strip_line_comments(frame_src), "fn unsupported(frame: &gfx::Frame<'_>) -> Result<(), Error> {")
    return sorted(set(re.findall(r'"([^"]+)"', body)))


def parse_consts(sources):
    """`pub const NAME: u32 = N;` (or u8/u16) across the given texts."""
    table = {}
    for text in sources:
        for name, value in re.findall(r"pub const (\w+): (?:u8|u16|u32) = (\d+);", text):
            table[name] = int(value)
    return table


class Audit:
    def __init__(self, root, expectations):
        self.root = Path(root)
        self.exp = expectations
        self.results = []
        self.cache = {}

    def read(self, rel):
        if rel not in self.cache:
            self.cache[rel] = (self.root / rel).read_text(encoding="utf-8")
        return self.cache[rel]

    def check(self, check_id, ok, detail):
        self.results.append({"id": check_id, "ok": bool(ok), "detail": detail})

    def run(self):
        exp = self.exp
        entity_src = self.read(exp["entity_rs"])
        sets = legacy_entity_sets(entity_src)
        self.check(
            "legacy.nonadditive_codes",
            sets["nonadditive"] == sorted(exp["legacy_entity"]["nonadditive"]),
            {"source": sets["nonadditive"], "expected": exp["legacy_entity"]["nonadditive"]},
        )
        self.check(
            "legacy.additive_codes",
            sets["additive"] == sorted(exp["legacy_entity"]["additive"]),
            {"source": sets["additive"], "expected": exp["legacy_entity"]["additive"]},
        )
        self.check(
            "legacy.glint_required",
            sets["glint_required"] == sorted(exp["legacy_entity"]["glint_required"]),
            {"source": sets["glint_required"], "expected": exp["legacy_entity"]["glint_required"]},
        )
        blockers = parse_frame_blockers(self.read(exp["frame_rs"]))
        self.check(
            "legacy.frame_blockers",
            blockers == sorted(exp["frame_blockers"]),
            {"source": blockers, "expected": sorted(exp["frame_blockers"])},
        )
        const_sources = [self.read(f) for f in exp["const_files"]]
        consts = parse_consts(const_sources)
        for name, value in exp["const_values"].items():
            self.check(
                f"const.{name}",
                consts.get(name) == value,
                {"source": consts.get(name), "expected": value},
            )
        for entry in exp["producers"]:
            self.check_snippet("producer." + entry["id"], entry["file"], entry["snippet"], entry["category"])
        for entry in exp["present"]:
            self.check_snippet("source." + entry["id"], entry["file"], entry["snippet"], entry.get("note", ""))
        for entry in exp["absent"]:
            text = norm(strip_line_comments(self.read(entry["file"])))
            hits = re.findall(entry["regex"], text)
            self.check(
                "absent." + entry["id"],
                len(hits) == 0,
                {"file": entry["file"], "regex": entry["regex"], "hits": len(hits), "note": entry.get("note", "")},
            )
        return self.results

    def check_snippet(self, check_id, rel, snippet, note):
        text = norm(strip_line_comments(self.read(rel)))
        found = norm(snippet) in text
        self.check(check_id, found, {"file": rel, "snippet": snippet, "note": note})


def write_report(results, path):
    failed = [r for r in results if not r["ok"]]
    report = {
        "schema": 1,
        "status": "passed" if not failed else "failed",
        "checks": len(results),
        "failed": [r["id"] for r in failed],
        "results": results,
        "limits": "static source reading; no Rust build, no GPU, no Win7/X1650 evidence",
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=str(REPO))
    parser.add_argument("--expectations", default=str(DEFAULT_EXPECTATIONS))
    parser.add_argument("--report", default=str(DEFAULT_REPORT))
    args = parser.parse_args(argv)
    expectations = json.loads(Path(args.expectations).read_text(encoding="utf-8"))
    results = Audit(args.root, expectations).run()
    report = write_report(results, Path(args.report))
    for r in results:
        if not r["ok"]:
            print(f"FAIL {r['id']}: {json.dumps(r['detail'], ensure_ascii=False)}", file=sys.stderr)
    print(f"{report['checks'] - len(report['failed'])}/{report['checks']} checks ok -> {args.report}")
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
