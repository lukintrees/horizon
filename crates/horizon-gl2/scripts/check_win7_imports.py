#!/usr/bin/env python3
"""Fail when a 32-bit Windows executable imports something Windows 7 does not provide.

Two things break on Windows 7:

* API-set DLLs (`api-ms-win-*`, `ext-ms-*`). Windows 7 has them only with later updates, and the
  Windows 7 Rust targets do not link against them.
* Functions that only Windows 8 and later export. The list is short on purpose: it covers what
  Rust's std and this probe could pull in, not every Windows 8 API.

Usage: check_win7_imports.py PATH/TO/gl2-probe.exe
Needs pefile (pip install pefile). Exit status 0 when the imports are acceptable, 1 otherwise.
"""

import sys

import pefile

IMAGE_FILE_MACHINE_I386 = 0x014C

# Exported only from Windows 8 onward (or, for the last two, Windows 10).
WINDOWS_8_PLUS = {
    "WaitOnAddress",
    "WakeByAddressSingle",
    "WakeByAddressAll",
    "GetSystemTimePreciseAsFileTime",
    "SetDefaultDllDirectories",
    "AddDllDirectory",
    "RemoveDllDirectory",
    "GetTempPath2W",
    "ProcessPrng",
    "SetThreadDescription",
}


def main(path: str) -> int:
    pe = pefile.PE(path, fast_load=False)
    if pe.FILE_HEADER.Machine != IMAGE_FILE_MACHINE_I386:
        print(f"{path}: not a 32-bit (i386) image, machine 0x{pe.FILE_HEADER.Machine:04x}")
        return 1

    problems = []
    dll_count = 0
    function_count = 0
    for entry in getattr(pe, "DIRECTORY_ENTRY_IMPORT", []):
        dll = entry.dll.decode("ascii", "replace")
        dll_count += 1
        lowered = dll.lower()
        if lowered.startswith(("api-ms-win-", "ext-ms-")):
            problems.append(f"API-set import: {dll}")
        for imp in entry.imports:
            function_count += 1
            if imp.name is None:
                continue
            name = imp.name.decode("ascii", "replace")
            if name in WINDOWS_8_PLUS:
                problems.append(f"Windows 8+ function: {dll}!{name}")

    print(f"{path}: {dll_count} DLLs, {function_count} imported functions")
    for entry in getattr(pe, "DIRECTORY_ENTRY_IMPORT", []):
        print(f"  {entry.dll.decode('ascii', 'replace')}")
    if problems:
        for problem in problems:
            print(f"::error::{problem}")
        return 1
    print("imports are within Windows 7")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    sys.exit(main(sys.argv[1]))
