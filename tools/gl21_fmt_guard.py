#!/usr/bin/env python3
"""Fail on the shapes `cargo fmt` would rewrite, without a Rust toolchain.

`cargo fmt --check` is the check that fails first in CI, and the machine the OpenGL 2.1 renderer is
being written on has no `cargo`. This approximates rustfmt's decisions for the files under
`crates/horizon-client-render/src/gl21/`, which are the ones being written:

- **`use_small_heuristics = "Default"`** is rustfmt's default, and it is what surprises people: a
  call's arguments must fit **60** characters, a struct literal **18**, arrays and method chains
  **60**, a `struct_lit_single_line` on one line only below 18. A line can be well under the 100
  column `max_width` and still be rewritten, which is exactly what happened the first time this
  renderer went through CI.
- **`reorder_imports = true`**: names inside `use` braces, and whole `mod` declarations, are sorted
  by ASCII order (so `Capabilities` before `glsl_name`).

Usage:
    python3 tools/gl21_fmt_guard.py                  # the gl21 files
    python3 tools/gl21_fmt_guard.py path/to/file.rs  # named files
    python3 tools/gl21_fmt_guard.py --list

Exits 1 when it finds a shape rustfmt would change, with the line and what to do about it.

Not a formatter and not a substitute for `cargo fmt`: it knows the heuristics, not the layout
algorithm. A file it passes can still fail CI; a file it fails will.
"""

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GUARDED = ROOT / "crates" / "horizon-client-render" / "src" / "gl21"

# rustfmt's `use_small_heuristics = "Default"`: the widths, as fractions of max_width (100).
CALL_WIDTH = 60
STRUCT_LIT_WIDTH = 18
ARRAY_WIDTH = 60
CHAIN_WIDTH = 60
MAX_WIDTH = 100
# A line's indent: at most this much, so a call's width is measurable from the columns.
INDENT = re.compile(r"^[ ]*")

KEYWORDS = {"if", "while", "match", "for", "return", "let", "loop"}


def arguments_width(line, open_at):
    """Characters between the parenthesis at `open_at` and its match, or None."""
    depth = 0
    for i in range(open_at, len(line)):
        c = line[i]
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return i - open_at - 1
    return None


def calls_too_wide(line):
    """The calls on `line` whose arguments are wider than rustfmt's call width."""
    problems = []
    # A string literal may hold parentheses: judged by the whole line's length instead.
    quoted = re.search(r'"[^"]*"', line) is not None
    for match in re.finditer(r"([A-Za-z_][\w:]*)\s*([!]?)\s*\(", line):
        name = match.group(1)
        macro_call = match.group(2) == "!"
        # A plain function call or a method call: `foo(`, `x.foo(`. Not `Type { .. }`'s fields, not
        # a macro (`println!`, `assert_eq!`) and not an enum variant (`Some(x)`).
        if name in KEYWORDS or macro_call or name[0].isupper():
            continue
        before = line[: match.start()].rstrip()
        if before and not (before.endswith(".") or before.endswith("=") or before.endswith("(")):
            # `fn foo(..)` definitions and `if foo(` are fine: rustfmt keeps a definition's
            # parameter list (it breaks only past `fn_call_width` for calls, not definitions), and
            # this tool only judges calls.
            if not before.endswith(("return", ",", "|", "&", "!", " ")):
                continue
        width = arguments_width(line, match.end() - 1)
        if width is None or width <= CALL_WIDTH:
            continue
        if quoted:
            # With a literal inside, rustfmt's own width is what decides; keep it well inside.
            if len(line) <= 78:
                continue
        problems.append((name, width))
    return problems


def struct_too_wide(line):
    """A struct literal in the middle of a line that rustfmt would put on several lines."""
    for match in re.finditer(r"\b([A-Z]\w*)\s*\{", line):
        depth = 0
        for i in range(match.end() - 1, len(line)):
            if line[i] == "{":
                depth += 1
            elif line[i] == "}":
                depth -= 1
                if depth == 0:
                    if i - match.start() > STRUCT_LIT_WIDTH + len(match.group(1)):
                        return match.group(1), i - match.start()
                    break
    return None


def chain_too_wide(line):
    """A method chain among the arguments of a call, wider than rustfmt's chain width.

    A chain that is the whole statement (`x.iter().map(..).collect();`) or the right side of an
    assignment is laid out by `chain_width` too, but its `let`/statement line is what rustfmt
    measures, and that is already reported by the call check. What is judged here is the case that
    bit: a chain inside `assert!(`/`assert_eq!(`, where the chain is one argument of a macro.
    """
    if "!(" not in line:
        return None
    start = line.find("!(") + 1
    dots = [m.start() for m in re.finditer(r"\.\w", line[start:])]
    if not dots:
        return None
    chain = len(line) - start
    if chain > CHAIN_WIDTH:
        return chain
    return None


def _sort_key(name):
    """rustfmt's order inside braces: `self` first, then as written (`Capabilities` > `glsl_name`)."""
    return (not name.startswith("self"), name)


def unsorted_use(line):
    """The names inside a top-level `use a::{b, c};` that are not in rustfmt's order.

    Only top-level ones: `reorder_imports` is not applied to a `use` inside a function body, which is
    also why a test's `use super::..; use std::..;` passes CI as written.
    """
    if line[:1].isspace():
        return None
    match = re.search(r"\buse\s+[\w:]*\{([^}]*)\}\s*;", line)
    if not match:
        return None
    names = [name.strip() for name in match.group(1).split(",") if name.strip()]
    if names != sorted(names, key=_sort_key):
        return names
    return None


def lines_of(path):
    return Path(path).read_text(encoding="utf-8").splitlines()


def code_of(line):
    """The line's code, without its comment: rustfmt does not reflow comments (`wrap_comments` is
    off), so what a comment says is never a formatting complaint."""
    cut = line.find("//")
    return line if cut < 0 else line[:cut]


def check_text(text, name="<memory>"):
    """Everything rustfmt would change, as (line, message)."""
    problems = []
    for number, raw in enumerate(text.splitlines(), 1):
        line = code_of(raw)
        if not line.strip():
            continue
        if len(line) > MAX_WIDTH:
            problems.append((number, f"{len(line)} columns is past rustfmt's {MAX_WIDTH}"))
        for called, width in calls_too_wide(line):
            problems.append(
                (
                    number,
                    f"`{called}(...)` has {width} characters of arguments; rustfmt breaks a call "
                    f"past {CALL_WIDTH}: give the value a name first",
                )
            )
        wide_struct = struct_too_wide(line)
        if wide_struct:
            problems.append(
                (
                    number,
                    f"the `{wide_struct[0]}` literal is {wide_struct[1]} columns wide; rustfmt "
                    f"puts a struct literal past {STRUCT_LIT_WIDTH} on its own lines (a "
                    "`Type::new(..)` or `Type::at(..)` constructor reads better anyway)",
                )
            )
        wide_chain = chain_too_wide(line)
        if wide_chain:
            problems.append(
                (
                    number,
                    f"this method chain is {wide_chain} columns; rustfmt breaks chains past "
                    f"{CHAIN_WIDTH}",
                )
            )
        names = unsorted_use(line)
        if names is not None:
            ordered = ", ".join(sorted(names, key=_sort_key))
            problems.append((number, f"`use` names are not in rustfmt's order: {ordered}"))
    problems.extend(unsorted_modules(text))
    return problems


def unsorted_modules(text):
    """`mod` declarations out of order: rustfmt sorts a run of them."""
    out = []
    run = []
    start = 0
    for number, raw in enumerate(text.splitlines(), 1):
        # Top-level declarations only: those start in column 0, which is where rustfmt sorts them.
        stripped = raw.strip()
        top_level = not raw[:1].isspace()
        simple = re.fullmatch(r"(pub(\([^)]*\))?\s+)?(mod|use)\s+[\w:]+;", stripped)
        if simple and top_level:
            if not run:
                start = number
            run.append((stripped, number))
        else:
            out.extend(_unsorted_run(run, start))
            run = []
    if run:
        out.extend(_unsorted_run(run, start))
    return out


def _sorted_key(line):
    """What rustfmt sorts a declaration by: the trailing `;` and the brace-less item's own name.

    `mod profile;` and `pub mod program;` sort by `mod profile` and `mod program`, not by the `pub`
    in front, so this strips a visibility and the `;` before comparing.
    """
    text = line.rstrip(";").strip()
    text = re.sub(r"^pub(\([^)]*\))?\s+", "", text)
    return text


def _unsorted_run(run, start):
    if len(run) < 2:
        return []
    # Only a run of the same kind is sorted together: `mod` lines and `use` lines do not mix.
    kinds = {"mod" if "mod " in line else "use" for line, _ in run}
    if len(kinds) != 1:
        return []
    names = [_sorted_key(line) for line, _ in run]
    if names == sorted(names):
        return []
    return [(start, f"these declarations are not sorted: {', '.join(sorted(names))}")]


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
        problems = check_text(Path(file).read_text(encoding="utf-8"), str(file))
        for line, message in problems:
            display = file.relative_to(ROOT) if file.is_absolute() and ROOT in file.parents else file
            print(f"{display}:{line}: {message}")
            found += 1
    if found:
        print(f"{found} shape(s) rustfmt would rewrite")
        return 1
    print(f"{len(files)} file(s) in the shape rustfmt keeps")
    return 0


if __name__ == "__main__":
    sys.exit(main())
