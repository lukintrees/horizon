#!/usr/bin/env python3
"""Reproducible C01 syntactic inventory (not a Rust name/type resolver).

Includes renderer owners/factories and candidate accesses through r/old/previous.
Candidate aliases require semantic review; this never proves GPU independence.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
OUTPUT = ROOT / "docs/design/legacy-renderer-inventory.json"


def scan(text, members):
    access = re.compile(r"\b(?:renderer|r|old|previous)\s*\.\s*(" + "|".join(sorted(members)) + r")\b")
    records = []
    for number, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("//"):
            continue
        kinds = []
        if re.search(r"\b(?:self\.)?renderer\b|\bRenderer\b", line):
            kinds.append("owner/factory/reference")
        if access.search(line):
            kinds.append("candidate-member-access")
        if kinds:
            records.append(dict(line=number, kinds=kinds, code=line.strip()))
    return records


def inventory(root):
    gfx = (root / "crates/horizon-client-render/src/gfx.rs").read_text()
    members = set(re.findall(r"pub (?:fn )?(\w+)\s*(?:[(:])", gfx))
    game = root / "crates/horizon-client/src/game.rs"
    sources = []
    for path in [game, *sorted(game.with_suffix("").rglob("*.rs"))]:
        data = path.read_bytes()
        text = data.decode()
        references = scan(text, members)
        if references:
            # Hash the LF form: core.autocrlf gives Windows workers CRLF files,
            # while the index stores LF and a Linux regeneration must match.
            sources.append(dict(path=str(path.relative_to(root)).replace("\\", "/"),
                                sha256=hashlib.sha256(text.replace("\r\n", "\n").encode()).hexdigest(),
                                references=references))
    return dict(contract_version=1, precision="syntactic candidates; review alias bindings", sources=sources)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = json.dumps(inventory(ROOT), ensure_ascii=False, indent=2) + "\n"
    if args.check:
        if not OUTPUT.exists() or OUTPUT.read_text() != text:
            print("C01 renderer inventory stale; run tools/ci/legacy_renderer_inventory.py", file=sys.stderr)
            return 1
    else:
        OUTPUT.write_text(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
