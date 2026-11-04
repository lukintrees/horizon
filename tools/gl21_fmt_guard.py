#!/usr/bin/env python3
"""Fail on the shapes `cargo fmt` would rewrite, without a Rust toolchain.

`cargo fmt --check` is the step that fails first in CI, and the machine the OpenGL 2.1 renderer is
being written on has no `cargo`. Every rule here was taken from a diff that CI produced for this
renderer, so they are rustfmt's own decisions and not guesses:

- **`chain_width = 60`** (`use_small_heuristics = "Default"`), for a chain inside an
  `assert!`-family macro: wider than 60 columns from its first `.method`, it gets a method a line.
  That is how `assert!(problems.iter().any(|p| ...))` came back from CI as five lines. A chain that
  is a statement of its own is left to `max_width`, which is why the rule is not applied to one.
- **`merge_derives = true`**: `#[derive(A)]` followed by `#[derive(B)]` is joined into one.
- **`reorder_imports = true`**: names inside a top-level `use`'s braces, and runs of `mod`
  declarations at the top level, are sorted (`Capabilities` before `glsl_name`).
- **`max_width = 100`**, for code only: rustfmt does not reflow a comment (`wrap_comments` is off),
  so a long `//` line is fine.

Usage:
    python3 tools/gl21_fmt_guard.py                  # the gl21 files
    python3 tools/gl21_fmt_guard.py path/to/file.rs  # named files
    python3 tools/gl21_fmt_guard.py --changed        # every Rust file this branch changed
    python3 tools/gl21_fmt_guard.py --list

Exits 1 when it finds a shape rustfmt would change, with the line and what to do about it.

Not a formatter. It knows the rules that have bitten this code, not rustfmt's layout algorithm: a
file it passes can still fail CI, but a file it fails certainly does.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GUARDED = ROOT / "crates" / "horizon-client-render" / "src" / "gl21"

# rustfmt's `use_small_heuristics = "Default"`, out of a `max_width` of 100.
CHAIN_WIDTH = 60
MAX_WIDTH = 100
# The macros whose arguments rustfmt lays out with `chain_width`.
MACRO = re.compile(r"\b(assert|assert_eq|assert_ne|debug_assert|debug_assert_eq)!\(")
# `let name: Vec<(u32, u32)> = ` — a binding whose type is written out, which is where a chain's
# line is long before the chain starts.
TYPED_LET = re.compile(r"^\s*let\s+\w+\s*:\s*[\w:<>()\[\];, &]+?\s*=\s*")


def code_of(line):
    """The line without its comment: rustfmt does not reflow comments, so their width is not a
    formatting complaint."""
    cut = line.find("//")
    return line if cut < 0 else line[:cut]


OPEN_LITERAL = re.compile(r'"[^"]*$')


def in_multiline_literal(text, number):
    """Whether the line at `number` (1-based) belongs to a string literal spread over lines.

    A long literal is written with a trailing backslash per line: rustfmt neither splits nor
    reflows it, so neither its first line nor the pieces after it are a formatting complaint.
    """
    open_literal = False
    for index, line in enumerate(text.splitlines(), 1):
        code = code_of(line).rstrip()
        if index == number:
            return open_literal or code.endswith("\\")
        if code.endswith("\\"):
            open_literal = True
            continue
        # A line with an odd number of quotes (escaped ones aside) opens or closes a literal.
        if len(re.findall(r'(?<!\\)"', code)) % 2 == 1:
            open_literal = not open_literal
    return False


def code_is_long(line):
    """Whether the line's code is past `max_width`, leaving out what rustfmt leaves alone.

    Three shapes run long in this repository and are never rewritten: a comment (not reflowed), a
    string literal (not split, and a long one continues with a trailing backslash), and the inside of
    a macro's brackets (`vertex_attr_array![..]` reaches 142 columns untouched). What is left is the
    line's structure, which is what `max_width` measures.
    """
    code = code_of(line).rstrip()
    if len(code) <= MAX_WIDTH:
        return False
    bare = re.sub(r'"[^"]*"', '""', code)
    bare = re.sub(r"\w+!\[[^\]]*\]", "macro![..]", bare)
    bare = re.sub(r"\w+!\([^)]*\)", "macro!(..)", bare)
    # A literal with a continuation (`.. needs \` / `a driver.."`) leaves a backslash behind.
    bare = bare.replace("\\", "")
    return len(bare) > MAX_WIDTH


def call_widths(line):
    """The argument widths of the `assert`-family macros on `line` that rustfmt would break.

    `assert_eq!(a, b)` is a call to rustfmt: past 60 columns of arguments it goes one argument a
    line, which is how a one-line equality came back from CI as four. Only these macros are judged:
    `#[derive(..)]` at 64 columns, and calls whose arguments are themselves chains, are kept as
    written, so judging every call would send people to reformat code CI accepts.
    """
    code = code_of(line).rstrip()
    out = []
    for match in MACRO.finditer(code):
        depth = 0
        close = None
        for i in range(match.end() - 1, len(code)):
            if code[i] == "(":
                depth += 1
            elif code[i] == ")":
                depth -= 1
                if depth == 0:
                    close = i
                    break
        if close is None:
            continue
        width = close - match.end()
        # `assert!(x.contains(".."))` is 64 columns of arguments and rustfmt keeps it: one method
        # call is not a chain. Two calls, or a comparison of two chains, it breaks.
        body = code[match.end():close]
        calls = len(re.findall(r"\.[a-z_][\w:]*\s*\(", body))
        if width > CHAIN_WIDTH and calls >= 2:
            out.append((match.group(1), width))
    return out


def chain_width(line):
    """The width of a long method chain on `line`, or None.

    Two shapes, both seen in CI:

    - a chain inside an `assert!`-family macro that is not a single method call
      (`assert!(problems.iter().any(..))` came back five lines long): rustfmt measures it against
      `chain_width` (60), while `assert!(x.contains(a))` it keeps on one line;
    - a chain under a written-out binding type (`let runs: Vec<(u32, u32)> = draws.runs.iter()..`):
      that one is broken one method a line however few columns it has.

    What is *not* judged, on purpose: a chain that is a plain statement (`self.a().b();`, 75
    columns in `gfx.rs`), and a chain of one call. Guessing those wrong sends someone to reformat
    code CI accepts.
    """
    code = code_of(line).rstrip()

    def chain_of(body):
        first = None
        for match in re.finditer(r"\.\w", body):
            first = match.start()
            break
        return None if first is None else len(body) - first

    macro = MACRO.search(code)
    if macro:
        body = code[macro.end():]
        if len(re.findall(r"\.[a-z_][\w:]*\s*\(", body)) < 2:
            return None
        return chain_of(body)
    binding = TYPED_LET.match(code)
    if binding:
        body = code[binding.end():]
        if len(re.findall(r"\.[a-z_][\w:]*\s*\(", body)) >= 3:
            # From the receiver: `draws.runs.iter()..` is a chain 62 columns wide, not 57 from its
            # second method.
            return len(body)
    return None


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
        if code_is_long(code) and not in_multiline_literal(text, number):
            what = "comment" if code.strip().startswith("//") else "code"
            problems.append(
                (
                    number,
                    f"{what} of {len(code)} columns is past rustfmt's {MAX_WIDTH}",
                )
            )
        width = chain_width(raw)
        if width is not None and width > CHAIN_WIDTH:
            problems.append(
                (
                    number,
                    f"this method chain is {width} columns; rustfmt breaks one past "
                    f"{CHAIN_WIDTH}: bind a step to a name",
                )
            )
        for called, width in call_widths(raw):
            problems.append(
                (
                    number,
                    f"`{called}!(..)` has {width} columns of arguments; rustfmt breaks a macro "
                    f"call past {CHAIN_WIDTH}: bind a value to a name first",
                )
            )
        names = unsorted_use(code)
        if names is not None:
            ordered = ", ".join(sorted(names, key=_sort_key))
            problems.append((number, f"`use` names are not in rustfmt's order: {ordered}"))
    problems.extend(unsorted_modules(text))
    problems.extend(merged_derives(text))
    return problems


def changed_files():
    """The `.rs` files this branch changed against main: what `cargo fmt --check` looks at.

    A branch's own files are not the only ones it can break: a `_with` helper that moved out of
    `#[cfg(test)]` is a `gfx.rs` change, and rustfmt judges it the same way.
    """
    out = subprocess.run(
        ["git", "diff", "--name-only", "origin/main...HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if out.returncode != 0:
        raise SystemExit(f"gl21_fmt_guard.py: git diff failed: {out.stderr.strip()}")
    committed = [ROOT / line for line in out.stdout.splitlines() if line.endswith(".rs")]
    # Plus the new ones the branch has written but not committed yet: the guard is run before a
    # commit, and a file that is not in the diff yet is the one most likely to be wrong.
    untracked = subprocess.run(
        ["git", "ls-files", "--others", "--exclude-standard", "*.rs"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    fresh = [ROOT / line for line in untracked.stdout.splitlines() if line.endswith(".rs")]
    return sorted({*committed, *fresh})


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("files", nargs="*")
    parser.add_argument("--list", action="store_true", help="print the files it would check")
    parser.add_argument(
        "--changed",
        action="store_true",
        help="every .rs file changed against main, which is what `cargo fmt --check` sees",
    )
    args = parser.parse_args(argv)
    if args.changed:
        files = changed_files()
    else:
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
