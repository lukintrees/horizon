#!/usr/bin/env python3
"""Fail on the shapes `cargo fmt` would rewrite, without a Rust toolchain.

`cargo fmt --check` is the step that fails first in CI, and the machine the OpenGL 2.1 renderer is
being written on has no `cargo`. Every rule here was taken from a diff that CI produced for this
renderer, so they are rustfmt's own decisions and not guesses:

- **`chain_width = 60`** (`use_small_heuristics = "Default"`): a method chain wider than 60 columns,
  measured from its first `.method` to the end of the line, gets a method a line. A line can be well
  under the 100-column `max_width` and still be rewritten — which is how `assert!(problems.iter()
  .any(|p| ...))` came back from CI as five lines.
- **`merge_derives = true`**: `#[derive(A)]` followed by `#[derive(B)]` is joined into one.
- **`reorder_imports = true`**: names inside a top-level `use`'s braces, and runs of `mod`
  declarations at the top level, are sorted (`Capabilities` before `glsl_name`).
- **`max_width = 100`**, for code only: rustfmt does not reflow a comment (`wrap_comments` is off),
  so a long `//` line is fine.

Usage:
    python3 tools/gl21_fmt_guard.py                  # the gl21 files
    python3 tools/gl21_fmt_guard.py path/to/file.rs  # named files
    python3 tools/gl21_fmt_guard.py --list

Exits 1 when it finds a shape rustfmt would change, with the line and what to do about it.

Not a formatter. It knows the rules that have bitten this code, not rustfmt's layout algorithm: a
file it passes can still fail CI, but a file it fails certainly does.
"""

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GUARDED = ROOT / "crates" / "horizon-client-render" / "src" / "gl21"

# rustfmt's `use_small_heuristics = "Default"`, out of a `max_width` of 100.
CHAIN_WIDTH = 60
MAX_WIDTH = 100


def code_of(line):
    """The line without its comment: rustfmt does not reflow comments, so their width is not a
    formatting complaint."""
    cut = line.find("//")
    return line if cut < 0 else line[:cut]


def chain_width(line):
    """The width of the line's method chain, from its first `.method`, or None."""
    code = code_of(line).rstrip()
    first = None
    for match in re.finditer(r"\.\w", code):
        first = match.start()
        break
    if first is None:
        return None
    return len(code) - first


def merged_derives(text):
    """Two `#[derive(..)]` attributes in a row: `merge_derives` joins them."""
    problems = []
    lines = text.splitlines()
    for number, line in enumerate(lines, 1):
        if not line.strip().startswith("#[derive("):
            continue
        following = lines[number] if number < len(lines) else ""
        if following.strip().startswith("#[derive("):
            problems.append(
                (number, "two `#[derive(..)]` in a row: rustfmt merges them into one")
            )
    return problems


def _sort_key(name):
    """rustfmt's order inside braces: `self` first, then as written."""
    return (not name.startswith("self"), name)


def unsorted_use(line):
    """The names of a top-level `use a::{b, c};` that are not in rustfmt's order.

    Only top-level ones: `reorder_imports` is not applied inside a function body.
    """
    if not line or line[0].isspace():
        return None
    match = re.search(r"\buse\s+[\w:]*\{([^}]*)\}\s*;", line)
    if not match:
        return None
    names = [name.strip() for name in match.group(1).split(",") if name.strip()]
    if names != sorted(names, key=_sort_key):
        return names
    return None


def _declaration_key(line):
    """What rustfmt sorts a brace-less declaration by: `pub mod b;` sorts as `mod b`."""
    text = line.rstrip(";").strip()
    return re.sub(r"^pub(\([^)]*\))?\s+", "", text)


def unsorted_modules(text):
    """A run of top-level `mod` declarations out of order."""
    out = []
    run = []
    start = 0
    for number, raw in enumerate(text.splitlines(), 1):
        stripped = raw.strip()
        # Top-level only: those start in column 0, which is where rustfmt sorts them.
        simple = re.fullmatch(r"(pub(\([^)]*\))?\s+)?mod\s+[\w]+;", stripped)
        if simple and not raw[:1].isspace():
            if not run:
                start = number
            run.append(_declaration_key(stripped))
        else:
            if len(run) > 1 and run != sorted(run):
                out.append((start, f"these `mod` lines are not sorted: {', '.join(sorted(run))}"))
            run = []
    if len(run) > 1 and run != sorted(run):
        out.append((start, f"these `mod` lines are not sorted: {', '.join(sorted(run))}"))
    return out


def check_text(text, name="<memory>"):
    """Everything rustfmt would change, as (line, message)."""
    del name  # for symmetry with `check_file`; the caller prints the path
    problems = []
    for number, raw in enumerate(text.splitlines(), 1):
        code = code_of(raw)
        if not code.strip():
            continue
        if len(code) > MAX_WIDTH:
            problems.append((number, f"{len(code)} columns of code is past rustfmt's {MAX_WIDTH}"))
        width = chain_width(raw)
        if width is not None and width > CHAIN_WIDTH:
            problems.append(
                (
                    number,
                    f"this method chain is {width} columns; rustfmt breaks one past "
                    f"{CHAIN_WIDTH}: bind a step to a name",
                )
            )
        names = unsorted_use(code)
        if names is not None:
            ordered = ", ".join(sorted(names, key=_sort_key))
            problems.append((number, f"`use` names are not in rustfmt's order: {ordered}"))
    problems.extend(unsorted_modules(text))
    problems.extend(merged_derives(text))
    return problems


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("files", nargs="*")
    parser.add_argument("--list", action="store_true", help="print the files it would check")
    args = parser.parse_args(argv)
    files = [Path(f) for f in args.files] or sorted(GUARDED.glob("*.rs"))
    if args.list:
        for file in files:
            print(file)
        return 0
    found = 0
    for file in files:
        text = Path(file).read_text(encoding="utf-8")
        for line, message in check_text(text, str(file)):
            display = file
            if file.is_absolute() and ROOT in file.parents:
                display = file.relative_to(ROOT)
            print(f"{display}:{line}: {message}")
            found += 1
    if found:
        print(f"{found} shape(s) rustfmt would rewrite")
        return 1
    print(f"{len(files)} file(s) in the shape rustfmt keeps")
    return 0


if __name__ == "__main__":
    sys.exit(main())
