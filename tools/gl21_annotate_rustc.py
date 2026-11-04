#!/usr/bin/env python3
"""Turn cargo's JSON diagnostics into GitHub annotations.

Why this exists: the `gl21` job's failure log lives in the runner's blob storage, and a
session that can reach only `api.github.com` cannot open it (`docs/gl21-dev-env.md`). The
checks API does serve *annotations*, and `::error file=...,line=...::` workflow commands are
what fills them. So the build step pipes `cargo --message-format=json` here instead of letting
the compiler's text scroll past, and a failing run states its first fifty errors where every
tool that can read a check run can see them.

What it reads: the JSON message stream cargo writes to stdout (`reason:
"compiler-message"`), with the plain lines around it tolerated -- `cargo test` interleaves
libtest's own output, and a tool that dies on that is worse than no tool.

What it writes: one `::error` command per diagnostic, and, when the stream carried no
structured error at all (a test failure, a linker crash), one command carrying the tail of the
plain output, because that is the only description of the failure there is.

Exit status is always 0: this is a reporter, and CI decides pass or fail from the command it
wrapped. Nothing here should be able to hide a build result.
"""

from __future__ import annotations

import json
import sys

# GitHub keeps the first 10 notices and 50 errors of a step; more than that is wasted work.
DEFAULT_LIMIT = 50
# How much of a diagnostic's rendered form to keep. The rendered text carries the `-->` line and
# the source excerpt, which is the part a reader needs; the rest is repetition.
RENDERED_LINES = 12
# How much plain output to attach when there were no structured diagnostics. libtest's summary
# is the *last* thing a failing test run prints, and GitHub truncates an annotation that is too
# long -- from the end, which is where the reason is. `-n 200` (the workflow's own tail) plus 30
# was over the limit, and the annotation that came back had lost "error: test failed".
RAW_TAIL_LINES = 120


def escape(text: str) -> str:
    """A workflow command's message: `%`, CR and LF have to be escaped, and nothing else."""
    return text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def escape_property(text: str) -> str:
    """A workflow command's property: the message escapes plus `,` and `:`."""
    return escape(text).replace(",", "%2C").replace(":", "%3A")


def annotations(payloads, limit: int = DEFAULT_LIMIT):
    """The error diagnostics in a cargo JSON stream, in file order, at most `limit` of them."""
    out = []
    for obj in payloads:
        if obj.get("reason") != "compiler-message":
            continue
        message = obj.get("message") or {}
        if message.get("level") != "error":
            continue
        primary = next(
            (span for span in message.get("spans", []) if span.get("is_primary")),
            None,
        )
        code = message.get("code") or {}
        rendered = message.get("rendered") or ""
        lines = rendered.strip().splitlines()
        text = message.get("message", "")
        if lines:
            text = "\n".join(lines[:RENDERED_LINES])
        out.append(
            {
                "file": (primary or {}).get("file_name", ""),
                "line": (primary or {}).get("line_start", 0),
                "column": (primary or {}).get("column_start", 0),
                "title": code.get("code", "") if isinstance(code, dict) else "",
                "message": text,
            }
        )
        if len(out) >= limit:
            break
    return out


def command(annotation) -> str:
    """One `::error` workflow command."""
    properties = []
    if annotation["file"]:
        properties.append(f"file={escape_property(annotation['file'])}")
    if annotation["line"]:
        properties.append(f"line={annotation['line']}")
    if annotation["column"]:
        properties.append(f"col={annotation['column']}")
    if annotation["title"]:
        properties.append(f"title={escape_property(annotation['title'])}")
    head = ",".join(properties)
    return f"::error {head}::{escape(annotation['message'])}" if head else (
        f"::error::{escape(annotation['message'])}"
    )


def parse(lines):
    """The JSON objects among `lines`, plus the lines that were not JSON."""
    payloads, plain = [], []
    for line in lines:
        stripped = line.strip()
        if not stripped.startswith("{"):
            plain.append(line.rstrip("\n"))
            continue
        try:
            payloads.append(json.loads(stripped))
        except json.JSONDecodeError:
            plain.append(line.rstrip("\n"))
    return payloads, plain


def report(payloads, plain, limit: int = DEFAULT_LIMIT):
    """The workflow commands for one cargo run: its diagnostics, or its output tail."""
    found = annotations(payloads, limit)
    if found:
        if len(found) >= limit:
            yield f"::notice::{limit} diagnostics shown, the rest are in the job log"
        for annotation in found:
            yield command(annotation)
        return
    tail = [line for line in plain if line.strip()][-RAW_TAIL_LINES:]
    if tail:
        # libtest's summary lines first: they survive a truncation that eats the middle.
        summary = [line for line in tail if line.startswith(("error", "failures", "test result"))]
        yield "::error::" + escape(chr(10).join(summary + tail))
        yield f"::notice::the tail is {len(tail)} lines; the job log has all of it"


def main(argv):
    if len(argv) > 1 and argv[1] in {"-h", "--help"}:
        print(__doc__.strip())
        return 0
    limit = DEFAULT_LIMIT
    paths = []
    for argument in argv[1:]:
        if argument.startswith("--limit="):
            limit = int(argument.split("=", 1)[1])
        else:
            paths.append(argument)
    if not paths:
        payloads, plain = parse(sys.stdin)
    else:
        payloads, plain = [], []
        for path in paths:
            try:
                with open(path, encoding="utf-8", errors="replace") as handle:
                    found_payloads, found_plain = parse(handle)
            except OSError as error:
                print(f"::warning::gl21_annotate_rustc: {error}")
                continue
            payloads.extend(found_payloads)
            plain.extend(found_plain)
    for line in report(payloads, plain, limit):
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
