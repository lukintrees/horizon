#!/usr/bin/env python3
"""Assemble an offline Cargo directory source from pinned GitHub vendor trees.

Does not modify Cargo.lock or Cargo configuration, or contact crates.io. A match
requires name/version/package checksum metadata and every listed file checksum.
This checks consistency with a trusted vendor manifest, NOT the SHA256 of the
original .crate archive (which cannot be reconstructed from a source checkout).
An incomplete inventory exits 2, and rejected/modified crates are never installed.
Downloaded sources, provenance and reports stay under ignored .local/.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
# Permissively licensed individual registry dependencies only. The license of the
# containing project (e.g. Firefox) is not a license for its vendor dependencies.
SOURCES = (
    ("ClickHouse/rust_vendor", "dfe46a63dec0a285e01d02ce293340d07e32f811", ""),
    ("mozilla-firefox/firefox", "dcafde486e11b7a7776137b92fec2a6f56744eab", "third_party/rust"),
    ("chromium-full-mirror/chromiumos_third_party_rust_crates",
     "968671f9c4ac2150a507ba09c158c9370b3c64c8", "vendor"),
    ("khorshuheng/rust-code-analysis-vendored", "270b9a5adfc50b1baee7f57668ee916becdce0ea", "vendor"),
)


def git(repo: Path, *args: str, text: str | None = None) -> str:
    return subprocess.run(["git", "-C", str(repo), *args], input=text,
                          capture_output=True, text=True, check=True).stdout


def packages(lock: Path) -> dict[tuple[str, str], dict]:
    data = tomllib.loads(lock.read_text())
    result = {}
    for p in data["package"]:
        if not p.get("source", "").startswith("registry+"):
            continue
        if not re.fullmatch(r"[a-zA-Z0-9_-]+", p["name"]) or not re.fullmatch(
                r"[a-zA-Z0-9.+_-]+", p["version"]):
            raise ValueError("unsafe package name/version")
        if not re.fullmatch(r"[0-9a-f]{64}", p["checksum"]):
            raise ValueError("invalid package checksum")
        result[p["name"], p["version"]] = p
    return result


def safe_file(directory: Path, name: str) -> Path:
    path = PurePosixPath(name)
    if not name or "\\" in name or path.is_absolute() or ".." in path.parts or ":" in name:
        raise ValueError(f"unsafe checksum path: {name!r}")
    target = directory.joinpath(*path.parts)
    if target.resolve() != target.absolute() or not target.is_file():
        raise ValueError(f"missing file or symbolic link: {name}")
    return target


def verify(directory: Path, package: dict, *, exact: bool = False) -> dict:
    manifest = tomllib.loads(safe_file(directory, "Cargo.toml").read_text())["package"]
    checksum = json.loads(safe_file(directory, ".cargo-checksum.json").read_text())
    if (manifest["name"], manifest["version"]) != (package["name"], package["version"]):
        raise ValueError("name/version mismatch")
    if checksum.get("package") != package["checksum"]:
        raise ValueError("package checksum metadata mismatch")
    files = checksum.get("files")
    if not isinstance(files, dict) or "Cargo.toml" not in files:
        raise ValueError("missing file checksums")
    for name, expected in files.items():
        if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
            raise ValueError(f"invalid file checksum: {name}")
        if hashlib.sha256(safe_file(directory, name).read_bytes()).hexdigest() != expected:
            raise ValueError(f"file checksum mismatch: {name}")
    if exact:
        actual = set()
        for path in directory.rglob("*"):
            if path.is_symlink():
                raise ValueError("symbolic link in installed vendor package")
            if path.is_file():
                actual.add(path.relative_to(directory).as_posix())
        if actual != {*files, ".cargo-checksum.json"}:
            raise ValueError("unlisted files in installed vendor package")
    return checksum


def install(directory: Path, vendor: Path, package: dict) -> None:
    checksum = verify(directory, package)
    destination = vendor / f"{package['name']}-{package['version']}"
    if destination.exists():
        if verify(destination, package, exact=True) != checksum:
            raise ValueError("installed checksum manifest differs from pinned source")
        return
    vendor.mkdir(parents=True, exist_ok=True)
    # Only manifest-listed files: never copy unlisted build scripts or modules.
    with tempfile.TemporaryDirectory(prefix=".stage-", dir=vendor) as temp:
        staging = Path(temp) / "package"
        staging.mkdir()
        for name in [*checksum["files"], ".cargo-checksum.json"]:
            target = staging / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(safe_file(directory, name), target)
        verify(staging, package, exact=True)
        staging.rename(destination)


def sparse(repo: Path, commit: str, paths: list[str]) -> None:
    git(repo, "sparse-checkout", "set", "--no-cone", "--stdin", text="".join(f"/{p}\n" for p in paths))
    # Update only this dependency checkout's index/tree, not the Horizon branch.
    git(repo, "read-tree", "-mu", commit)


def fetch(source: tuple[str, str, str], cache: Path, wanted: dict) -> tuple[Path, list[tuple[dict, str]]]:
    repository, commit, prefix = source
    if not re.fullmatch(r"[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+", repository) or not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("invalid pinned GitHub source")
    repo = cache / repository.replace("/", "--")
    if not repo.exists():
        subprocess.run(["git", "clone", "--depth", "1", "--filter=blob:none", "--no-checkout",
                        f"https://github.com/{repository}.git", str(repo)], check=True)
    try:
        git(repo, "cat-file", "-e", f"{commit}^{{commit}}")
    except subprocess.CalledProcessError:
        git(repo, "fetch", "--depth", "1", "origin", commit)
    tree = f"{commit}:{prefix}" if prefix else commit
    dirs = git(repo, "ls-tree", "-d", "--name-only", tree).splitlines()
    # Firefox uses names without versions and suffixes for duplicate versions.
    dirs = [d for d in dirs if any(d == n or d == n.replace("-", "_") or
            d.startswith(n + "-") or d.startswith(n + "_") for n, _ in wanted)]
    paths = [f"{prefix}/{d}".lstrip("/") for d in dirs]
    sparse(repo, commit, [f"{p}/{f}" for p in paths for f in ("Cargo.toml", ".cargo-checksum.json")])
    found = []
    for path in paths:
        directory = repo / path
        try:
            p = tomllib.loads((directory / "Cargo.toml").read_text())["package"]
            expected = wanted.get((p["name"], p["version"]))
            ck = json.loads((directory / ".cargo-checksum.json").read_text())
            if expected and ck.get("package") == expected["checksum"]:
                found.append((expected, path))
        except (OSError, KeyError, ValueError):
            continue
    sparse(repo, commit, [p + "/" for _, p in found])
    return repo, found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lock", type=Path, default=ROOT / "Cargo.lock")
    parser.add_argument("--cache", type=Path, default=ROOT / ".local/cargo-github-vendor")
    args = parser.parse_args()
    args.cache.mkdir(parents=True, exist_ok=True)
    wanted = packages(args.lock)
    vendor = args.cache / "vendor"
    report: dict = {"verification": "vendor metadata + all listed file SHA256; not original archive SHA256",
                    "accepted": [], "rejected": [], "missing": []}
    for source in SOURCES:
        repo, found = fetch(source, args.cache, wanted)
        for package, path in found:
            try:
                install(repo / path, vendor, package)
            except (OSError, ValueError, KeyError) as error:
                report["rejected"].append({"name": package["name"], "version": package["version"],
                                           "repository": source[0], "error": str(error)})
                continue
            report["accepted"].append({**package, "repository": source[0], "commit": source[1], "path": path})
            wanted.pop((package["name"], package["version"]))
    report["missing"] = list(wanted.values())
    report["status"] = "incomplete" if wanted else "complete"
    (args.cache / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    # Explicit, opt-in --config only; never silently change the normal game build.
    config = args.cache / "offline.toml"
    config.write_text('[source.crates-io]\nreplace-with = "github-vendor"\n\n'
                      '[source.github-vendor]\ndirectory = ' + json.dumps(str(vendor.resolve())) + '\n')
    print(f"{report['status']}: {len(report['accepted'])} verified, {len(wanted)} missing; {args.cache / 'report.json'}")
    print(f"cargo --config {config} check --offline --locked -p horizon-client-render")
    return 2 if wanted else 0


if __name__ == "__main__":
    raise SystemExit(main())
