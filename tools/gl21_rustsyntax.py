#!/usr/bin/env python3
"""Parse every Rust file of the renderer with tree-sitter, so a syntax error is found without
rustc.

    python3 tools/gl21_rustsyntax.py                  # every Rust file in the workspace
    python3 tools/gl21_rustsyntax.py crates/horizon-client-gl
    python3 tools/gl21_rustsyntax.py --changed        # files changed against the merge base
    python3 tools/gl21_rustsyntax.py --install        # tree_sitter into .local/gl21-env

Why this exists: the crates of this port are written where no Rust toolchain is available
(see docs/gl21-dev-env.md §4), and a syntax error is the one class of mistake a parser can
catch without a compiler. It is not a type checker and must never be presented as one: it
answers "does this file parse", nothing more. `cargo clippy -D warnings` on the build machine
is still the gate.

Needs tree-sitter and tree-sitter-rust (pure Python wheels, no toolchain):
    tools/gl21_preflight.sh --install
or, with a mirror:
    pip install --target .local/gl21-env --index-url <mirror>/simple tree-sitter tree-sitter-rust
The script finds them in .local/gl21-env by itself.
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ENVDIR = ROOT / ".local" / "gl21-env"

SKIP_DIRS = {".git", "target", ".local", "node_modules", "__pycache__"}


def load_tree_sitter():
    if ENVDIR.is_dir() and str(ENVDIR) not in sys.path:
        sys.path.insert(0, str(ENVDIR))
    try:
        import tree_sitter
        import tree_sitter_rust
    except ImportError:
        return None
    return tree_sitter, tree_sitter_rust


def install():
    ENVDIR.mkdir(parents=True, exist_ok=True)
    mirror = os.environ.get("PIP_INDEX_URL")
    cmd = [sys.executable, "-m", "pip", "install", "--target", str(ENVDIR), "--upgrade",
           "tree-sitter", "tree-sitter-rust"]
    if mirror:
        cmd += ["--index-url", mirror]
    print(" ".join(cmd))
    return subprocess.call(cmd)


def rust_files(root, changed=False):
    if changed:
        try:
            base = subprocess.run(["git", "merge-base", "HEAD", "origin/main"],
                                  cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()
        except (subprocess.CalledProcessError, FileNotFoundError):
            base = "HEAD~1"
        out = subprocess.run(["git", "diff", "--name-only", base, "--", "*.rs"],
                             cwd=ROOT, capture_output=True, text=True).stdout
        names = [n for n in out.splitlines() if n.strip()]
        return [ROOT / n for n in names if (ROOT / n).is_file()]
    files = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for name in filenames:
            if name.endswith(".rs"):
                files.append(Path(dirpath) / name)
    return sorted(files)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("paths", nargs="*", default=[], help="directories or files (default: repo)")
    ap.add_argument("--changed", action="store_true", help="only files changed vs origin/main")
    ap.add_argument("--install", action="store_true", help="install tree-sitter into .local/gl21-env")
    ap.add_argument("--max-errors", type=int, default=0,
                    help="print at most this many errors per file (0: all)")
    args = ap.parse_args()

    if args.install:
        return install()

    loaded = load_tree_sitter()
    if loaded is None:
        print("tree-sitter is not installed: run tools/gl21_preflight.sh --install",
              file=sys.stderr)
        return 2
    tree_sitter, tree_sitter_rust = loaded
    parser = tree_sitter.Parser(tree_sitter.Language(tree_sitter_rust.language()))

    if args.paths:
        files = []
        for p in args.paths:
            path = (ROOT / p).resolve()
            if path.is_dir():
                files.extend(rust_files(path))
            elif path.suffix == ".rs" and path.is_file():
                files.append(path)
    else:
        files = rust_files(ROOT, changed=args.changed)

    failed = 0
    total_errors = 0
    for path in files:
        source = path.read_bytes()
        tree = parser.parse(source)
        errors = []

        def walk(node):
            if node.type == "ERROR" or (
                    node.type == "MISSING" and node.is_missing):
                errors.append(node)
            for child in node.children:
                walk(child)

        walk(tree.root_node)
        # `has_error` is set on ancestors too; the walk above collects the leaves.
        if not errors and tree.root_node.has_error:
            errors.append(tree.root_node)
        if errors:
            failed += 1
            rel = path.relative_to(ROOT) if path.is_relative_to(ROOT) else path
            print(f"{rel}: {len(errors)} syntax error(s)")
            shown = errors if args.max_errors == 0 else errors[:args.max_errors]
            for node in shown:
                line, col = node.start_point
                text = source[node.start_byte:node.end_byte][:60].decode("utf-8", "replace")
                text = text.replace("\n", "\\n")
                print(f"  {line + 1}:{col + 1}  {node.type}  {text!r}")
            total_errors += len(errors)

    print(f"{len(files)} file(s) parsed, {failed} with syntax error(s), {total_errors} error(s)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
