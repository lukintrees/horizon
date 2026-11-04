#!/usr/bin/env python3
"""Turn a compiler's or rustfmt's output into GitHub Actions annotations.

Written for `.github/workflows/gl21-check.yml`: the OpenGL 2.1 work (`docs/opengl21-plan.md`) is
built on a machine that can read the checks API but not the action logs (they are served from blob
storage the sandbox has no route to), so each complaint is republished as an annotation, which the
API does return.

Usage:
    python3 tools/gl21_annotate.py fmt /tmp/fmt.diff
    python3 tools/gl21_annotate.py clippy /tmp/clippy.txt
    python3 tools/gl21_annotate.py test /tmp/test.txt

Exits 1 when it found something to annotate and 0 when it found nothing, so the step it runs in is
red exactly when the build is.

The three formats:

- `cargo fmt --check` writes a diff whose hunks start with `Diff in <path> at line <n>:` (newer
  rustfmt) or `Diff in <path> at line <n>:` without a column, followed by `@@`-less `+`/`-` lines.
- `cargo clippy --message-format=short` writes `path:line:col: error[kind]: message` (and the same
  with `warning`).
- `cargo test` writes a panic as `thread '<name>' panicked at <path>:<line>:<col>:` and a compile
  error as `error[E0425]: ...` followed by ` --> <path>:<line>:<col>`.
"""

import re
import sys
from pathlib import Path

# `Diff in crates/horizon-client-render/src/gl21/atlas.rs at line 12:`; older rustfmt prints the
# path on its own after `Diff in `, so both are taken.
FMT_HUNK = re.compile(r"^Diff in (?P<file>.+?)(?: at line (?P<line>\d+))?:?$")
# `crates/x/src/y.rs:12:5: error[clippy::foo]: message`, or `error: message` with no code.
SHORT = re.compile(
    r"^(?P<file>[^:]+\.rs):(?P<line>\d+):(?P<col>\d+): (?P<kind>error|warning)"
    r"(?:\[[^\]]+\])?: (?P<message>.*)$"
)

# `---- a_test stdout ----` opens a failed test's output; what it printed follows.
SECTION = re.compile(r"^---- (?P<name>\S+) stdout ----$")
# `thread 'gl21::draws::tests::a_test' panicked at crates/x/src/y.rs:12:5:`
# `thread 'a_test' (6464) panicked at crates/x/src/y.rs:12:5:` — rustc 1.87 prints the thread's id,
# older ones do not. Both match.
PANIC = re.compile(
    r"^thread '(?P<name>[^']+)'(?: \(\d+\))? panicked at (?P<file>[^:\s]+):(?P<line>\d+):(?P<col>\d+):?"
)
# The location a rustc diagnostic points at: ` --> crates/x/src/y.rs:12:5`.
LOCATION = re.compile(r"^\s*--> (?P<file>[^:\s]+):(?P<line>\d+):(?P<col>\d+)$")
# `error[E0425]: cannot find value `x`` / `error: could not compile ...`
COMPILE = re.compile(r"^error(?:\[(?P<code>E\d+)\])?: (?P<message>.+)$")

GITHUB_LIMIT = 50


def _escape(text):
    """A workflow command's data: `%`, carriage returns and newlines are what break it."""
    return text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def fmt_annotations(text):
    """(file, line, message) for each hunk of a `cargo fmt --check` diff."""
    out = []
    file = None
    line = 1
    changed = []
    for raw in text.splitlines():
        hunk = FMT_HUNK.match(raw.strip())
        if hunk:
            if file and changed:
                out.append((file, line, _changed_message(changed)))
            file = hunk.group("file").strip()
            line = int(hunk.group("line") or 1)
            changed = []
        elif file and raw.startswith(("+", "-")) and not raw.startswith(("+++", "---")):
            changed.append(raw.strip())
    if file and changed:
        out.append((file, line, _changed_message(changed)))
    return out


def _changed_message(changed):
    shown = " | ".join(changed[:6])
    more = f" (+{len(changed) - 6} more)" if len(changed) > 6 else ""
    return f"rustfmt would rewrite this: {shown}{more}"


def clippy_annotations(text):
    """(file, line, message) for each `--message-format=short` diagnostic."""
    out = []
    for raw in text.splitlines():
        match = SHORT.match(raw.strip())
        if not match:
            continue
        kind = match.group("kind")
        out.append(
            (
                match.group("file"),
                int(match.group("line")),
                f"{kind}: {match.group('message')}",
            )
        )
    return out


def test_annotations(text):
    """(file, line, message) for each failed test and each compile error of a `cargo test` run."""
    out = []
    lines = text.splitlines()
    # What the test that failed last said: its own panic line, when the harness printed one, is
    # where it points; without one the assertion is a `---- test stdout ----` block whose first
    # line is the message and whose `left`/`right` lines are the interesting part.
    current = None
    for index, raw in enumerate(lines):
        line = raw.strip()
        section = SECTION.match(line)
        if section:
            current = {"name": section.group("name"), "body": []}
        elif current is not None:
            if line.startswith("failures:") or line.startswith("test result:"):
                # The section is over: keep whatever it printed.
                if current["body"]:
                    out.append(_section_annotation(current))
                current = None
            else:
                current["body"].append(line)
        panic = PANIC.match(line)
        if panic:
            out.append(
                (
                    panic.group("file"),
                    int(panic.group("line")),
                    f"{panic.group('name')} panicked",
                )
            )
            continue
        compile_error = COMPILE.match(line)
        if compile_error and "could not compile" not in compile_error.group("message"):
            # `error[..]` on its own line; the location is the `-->` under it.
            where = None
            for below in lines[index + 1 : index + 6]:
                where = LOCATION.match(below.strip())
                if where:
                    break
            if where:
                code = compile_error.group("code") or ""
                out.append(
                    (
                        where.group("file"),
                        int(where.group("line")),
                        f"{code} {compile_error.group('message')}".strip(),
                    )
                )
            continue
        # A single line diagnostic (`path:line:col: error: ..`), same as clippy's short format.
        short = SHORT.match(line)
        if short:
            out.append(
                (
                    short.group("file"),
                    int(short.group("line")),
                    f"{short.group('kind')}: {short.group('message')}",
                )
            )
    return out


def _section_annotation(section):
    """A failed test that printed no `panicked at` line: its name, and the first lines it printed."""
    body = [line for line in section["body"] if line and not line.startswith("note:")]
    where = "?"
    for line in body:
        found = re.search(r"(crates/[\w/.-]+\.rs|tools/[\w/.-]+\.py):(\d+)", line)
        if found:
            where = found.group(2)
            break
    message = " | ".join(body[:4]) or "the test failed without printing anything"
    return ("crates/horizon-client-render/src/gl21/mod.rs", 1 if where == "?" else int(where),
            f"{section['name']}: {message}")


def annotations(mode, text):
    if mode == "fmt":
        return fmt_annotations(text)
    if mode == "clippy":
        return clippy_annotations(text)
    if mode == "test":
        return test_annotations(text)
    raise SystemExit(f"gl21_annotate.py: {mode!r} is neither fmt nor clippy")


def main(argv):
    if len(argv) != 3:
        raise SystemExit(__doc__)
    mode, path = argv[1], argv[2]
    text = Path(path).read_text(encoding="utf-8", errors="replace")
    found = annotations(mode, text)
    for file, line, message in found[:GITHUB_LIMIT]:
        print(f"::error file={file},line={line}::{_escape(message)}")
    if len(found) > GITHUB_LIMIT:
        print(f"::error::{len(found) - GITHUB_LIMIT} more {mode} annotation(s) past GitHub's limit")
    if not found:
        # The step runs only when the tool failed, so silence here means the failure was not a
        # diagnostic the pattern knows: print the tail, or the build looks red for no reason.
        tail = " | ".join(text.splitlines()[-8:])
        print(f"::error::{mode} failed without a recognised diagnostic: {_escape(tail)}")
        return 1
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
