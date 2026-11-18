#!/usr/bin/env python3
"""Cross-build Horizon for Windows 7 (PE32 and PE64) from Linux with mingw-w64.

The `*-win7-windows-gnu` targets exist only on nightly Rust: they lower the PE
subsystem version the linker declares, so the produced image still loads on
Windows 7. Nightly ships no precompiled std for them, which is why every command
carries `-Zbuild-std=std,panic_abort`; that needs the toolchain's `rust-src`
component.

Nothing here edits the system toolchain. The rustup toolchain is chosen with
`--toolchain` (default: `$RUSTUP_TOOLCHAIN`, else `nightly`) and exported as
`RUSTUP_TOOLCHAIN` for cargo, so a machine whose `rustc` is not rustup's is
untouched. The C parts (mimalloc, hidapi, ...) are built by cc-rs, which finds
the host `gcc` unless the target-prefixed compiler is named; the script exports
`CC_<target>`, `CXX_<target>`, `AR_<target>`, the linker and `WINDRES` for the
target so the icon resource in `packaging/windows/exe.rs` is compiled by
mingw-w64's windres rather than an unprefixed `windres`.

    python3 tools/win7_build.py                                  # both targets, the legacy crates
    python3 tools/win7_build.py --target x86_64
    python3 tools/win7_build.py -p horizon-client-render -p horizon-game --features legacy-gl
    python3 tools/win7_build.py --profile fast -p horizon-game --features legacy-gl
    python3 tools/win7_build.py --check -p horizon-game --features legacy-gl   # faster iterate

The exact cargo command (and the environment it gets) is printed before it runs.
After a `build`, every `.exe` in that target's output directory is read as a PE
image and its declared subsystem version is printed: 6.1 is Windows 7, and a value
at or below it loads there (these targets declare 4.0 on i686 and 5.2 on x86_64).

Exit status: 0 succeeded, 1 a build failed, 2 usage (argparse), 3 a compiler,
linker or windres tool is missing for the requested target.
"""
from __future__ import annotations

import argparse
import os
import shutil
import struct
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# `--target` name → (rust triple, mingw-w64 tool prefix).
TARGETS = {
    "x86_64": ("x86_64-win7-windows-gnu", "x86_64-w64-mingw32"),
    "i686": ("i686-win7-windows-gnu", "i686-w64-mingw32"),
}

# The crates the legacy GL2 backend is: the contract and the backend itself.
# `horizon-client-render` and `horizon-game` need `--features legacy-gl`.
DEFAULT_PACKAGES = ("horizon-legacy-render", "horizon-render-api")

PROFILES = ("dev", "fast", "release")

# Windows 7's own version, for the summary line: a PE whose subsystem version is at
# or below this one loads on Windows 7, a higher one refuses it.
WINDOWS_7_VERSION = (6, 1)


def toolchain_default() -> str:
    return os.environ.get("RUSTUP_TOOLCHAIN") or "nightly"


def cargo_binary() -> Path | None:
    """A rustup cargo if there is one, so `--toolchain nightly` means what it says.

    The system cargo is not necessarily rustup's; `~/.cargo/bin/cargo` is where a
    rustup installed next to a distro toolchain puts its proxy. Falling back to
    `cargo` on PATH is fine: RUSTUP_TOOLCHAIN is ignored by a non-rustup cargo.
    """
    local = Path.home() / ".cargo" / "bin" / "cargo"
    if local.is_file() and os.access(local, os.X_OK):
        return local
    found = shutil.which("cargo")
    return Path(found) if found else None


def target_tools(prefix: str) -> dict[str, str]:
    """The target C toolchain, as cc-rs, rustc's linker and winresource find it."""
    return {
        "cc": f"{prefix}-gcc",
        "cxx": f"{prefix}-g++",
        "ar": f"{prefix}-ar",
        "windres": f"{prefix}-windres",
    }


def build_env(triple: str, prefix: str, toolchain: str) -> dict[str, str]:
    env = os.environ.copy()
    env["RUSTUP_TOOLCHAIN"] = toolchain
    # cc-rs looks these up with the target's dashes turned into underscores.
    key = triple.replace("-", "_")
    env[f"CC_{key}"] = f"{prefix}-gcc"
    env[f"CXX_{key}"] = f"{prefix}-g++"
    env[f"AR_{key}"] = f"{prefix}-ar"
    # rustc would derive `x86_64-w64-mingw32-gcc` itself; naming it keeps the
    # command readable and fails loudly when the linker is somewhere else.
    env[f"CARGO_TARGET_{key.upper()}_LINKER"] = f"{prefix}-gcc"
    # winresource warns "unknown Windows target ... invoking unprefixed windres"
    # for a target it does not know, then runs `windres` from PATH, which is not
    # there on a Linux box. WINDRES is its documented override.
    env["WINDRES"] = f"{prefix}-windres"
    return env


def cargo_command(
    args: argparse.Namespace, triple: str
) -> list[str]:
    cargo = cargo_binary()
    if cargo is None:
        raise SystemExit("cargo not found (install rustup or put cargo on PATH)")
    command = [str(cargo), "-Zbuild-std=std,panic_abort"]
    if args.check:
        command.append("check")
    else:
        command.append("build")
        if args.profile != "dev":
            command += ["--profile", args.profile]
    command += ["--target", triple]
    for package in args.package:
        command += ["-p", package]
    if args.features:
        command += ["--features", ",".join(args.features)]
    if args.no_default_features:
        command.append("--no-default-features")
    if args.jobs:
        command += ["-j", str(args.jobs)]
    return command


def pe_subsystem(path: Path) -> tuple[tuple[int, int], int] | None:
    """(subsystem version, subsystem id) from a PE optional header, or None."""
    try:
        data = path.read_bytes()
    except OSError:
        return None
    if len(data) < 64 or data[:2] != b"MZ":
        return None
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if pe + 24 + 70 > len(data) or data[pe : pe + 4] != b"PE\0\0":
        return None
    # The optional header starts after the 20-byte COFF header and the 4-byte
    # signature. Layout shared by PE32 and PE32+:
    #   +40 Major/MinorOperatingSystemVersion, +44 Major/MinorImageVersion,
    #   +48 Major/MinorSubsystemVersion, +68 Subsystem.
    optional = pe + 24
    magic = struct.unpack_from("<H", data, optional)[0]
    if magic not in (0x10B, 0x20B):  # PE32, PE32+
        return None
    major, minor = struct.unpack_from("<HH", data, optional + 48)
    subsystem = struct.unpack_from("<H", data, optional + 68)[0]
    return (major, minor), subsystem


def report_executables(directory: Path) -> None:
    exes = sorted(directory.glob("*.exe"))
    if not exes:
        print(f"no .exe in {directory} (crate libraries only)")
        return
    for exe in exes:
        header = pe_subsystem(exe)
        if header is None:
            print(f"{exe}: not a readable PE image")
            continue
        (major, minor), subsystem = header
        kind = {2: "GUI", 3: "console"}.get(subsystem, f"subsystem {subsystem}")
        # The field is the *minimum* subsystem the loader must provide, so a value
        # at or below Windows 7 is fine and anything above it would refuse Win7.
        verdict = "" if (major, minor) <= WINDOWS_7_VERSION else "  (needs a newer Windows than 7)"
        print(f"{exe}: PE subsystem {major}.{minor} ({kind}){verdict}")


def build_target(args: argparse.Namespace, name: str) -> bool:
    triple, prefix = TARGETS[name]
    tools = target_tools(prefix)
    missing = [tool for tool in tools.values() if shutil.which(tool) is None]
    if missing:
        print(
            f"{name}: missing {', '.join(missing)} (install mingw-w64: "
            "Arch's `mingw-w64-gcc`, Debian's `gcc-mingw-w64-*`)",
            file=sys.stderr,
        )
        raise SystemExit(3)
    toolchain = args.toolchain
    env = build_env(triple, prefix, toolchain)
    command = cargo_command(args, triple)
    print(f"== {name} ({triple}), toolchain {toolchain}")
    print("   " + " ".join(command), flush=True)
    result = subprocess.run(command, cwd=ROOT, env=env)
    if result.returncode != 0:
        return False
    if args.check:
        return True
    out = ROOT / "target" / triple / ("debug" if args.profile == "dev" else args.profile)
    report_executables(out)
    return True


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__.splitlines()[0],
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__.split("Exit status:")[0],
    )
    parser.add_argument(
        "--target",
        choices=(*TARGETS, "both"),
        default="both",
        help="the Windows 7 architecture to build (default: both)",
    )
    parser.add_argument(
        "-p",
        "--package",
        action="append",
        default=None,
        metavar="CRATE",
        help=(
            "a crate to build, repeatable "
            f"(default: {' '.join(DEFAULT_PACKAGES)}); "
            "horizon-client-render and horizon-game need --features legacy-gl"
        ),
    )
    parser.add_argument(
        "--features",
        action="append",
        default=[],
        metavar="FEATURES",
        help="features for every selected package, repeatable (legacy-gl, ...)",
    )
    parser.add_argument(
        "--no-default-features",
        action="store_true",
        help="build the selected packages without their default features",
    )
    parser.add_argument(
        "--profile",
        choices=PROFILES,
        default="dev",
        help="cargo profile for build (default: dev; `fast` is the game's playable one)",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="`cargo check` instead of build, for faster iteration",
    )
    parser.add_argument(
        "--toolchain",
        default=toolchain_default(),
        help="rustup toolchain (default: $RUSTUP_TOOLCHAIN, else nightly)",
    )
    parser.add_argument("-j", "--jobs", type=int, default=None, help="cargo jobs")
    args = parser.parse_args()
    if args.package is None:
        args.package = list(DEFAULT_PACKAGES)

    targets = list(TARGETS) if args.target == "both" else [args.target]
    failed = []
    for name in targets:
        if not build_target(args, name):
            failed.append(name)
    if failed:
        print(f"failed: {', '.join(failed)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
