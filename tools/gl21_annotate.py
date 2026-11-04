#!/usr/bin/env python3
"""Turn a compiler's or rustfmt's output into GitHub Actions annotations.

Written for `.github/workflows/gl21-check.yml`: the OpenGL 2.1 work (`docs/opengl21-plan.md`) is
built on a machine that can read the checks API but not the action logs (they are served from blob
storage the sandbox has no route to), so each complaint is republished as an annotation, which the
API does return.

Usage:
    python3 tools/gl21_annotate.py fmt /tmp/fmt.diff
    python3 tools/gl21_annotate.py clippy /tmp/clippy.txt

Exits 1 when it found something to annotate and 0 when it found nothing, so the step it runs in is
red exactly when the build is.

The two formats:

- `cargo fmt --check` writes a diff whose hunks start with `Diff in <path> at line <n>:` (newer
  rustfmt) or `Diff in <path> at line <n>:` without a column, followed by `@@`-less `+`/`-` lines.
- `cargo clippy --message-format=short` writes `path:line:col: error[kind]: message` (and the same
  with `warning`).
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


def annotations(mode, text):
    if mode == "fmt":
        return fmt_annotations(text)
    if mode == "clippy":
        return clippy_annotations(text)
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
