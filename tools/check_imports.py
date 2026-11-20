#!/usr/bin/env python3
"""Fail when a release binary needs a library its players may not have.

Windows .exe: every DLL in the import and delay-import tables must ship with
Windows (no VCRUNTIME140.dll, vulkan-1.dll, ...): Vulkan and the like are
loaded at run time, so a missing driver is an error message, not a dialog.
Linux ELF: the loader must be the standard one (not a /nix/store path) and the
needed libraries the ones every desktop distribution has. `--server`: a server
build, which may not need ALSA or udev either (minimal server images lack them).
`--musl`: require the musl loader and libraries instead of glibc.
"""

import argparse
import struct
import sys
from pathlib import Path

# DLLs that are part of Windows 10 and 11. api-ms-win-* and ext-ms-win-* API
# sets resolve inside Windows too. uiautomationcore.dll (UI Automation, in System32
# since Windows 7) is what the launcher's accessibility (accesskit_windows) imports.
WINDOWS_SYSTEM = {
    "advapi32.dll", "bcrypt.dll", "bcryptprimitives.dll", "cfgmgr32.dll", "combase.dll",
    "comctl32.dll", "comdlg32.dll", "crypt32.dll", "d2d1.dll", "d3d11.dll", "d3d12.dll",
    "d3dcompiler_47.dll", "dbghelp.dll", "dcomp.dll", "dwmapi.dll", "dwrite.dll", "dxgi.dll",
    "gdi32.dll", "hid.dll", "icuuc.dll", "imm32.dll", "iphlpapi.dll", "kernel32.dll",
    "kernelbase.dll", "mfplat.dll", "mmdevapi.dll", "msvcrt.dll", "ncrypt.dll", "ntdll.dll",
    "ole32.dll", "oleaut32.dll", "opengl32.dll", "propsys.dll", "secur32.dll", "setupapi.dll",
    "shcore.dll", "shell32.dll", "shlwapi.dll", "ucrtbase.dll", "user32.dll", "userenv.dll",
    "uiautomationcore.dll", "uxtheme.dll", "version.dll", "winmm.dll", "ws2_32.dll",
    "xinput1_4.dll", "xinput9_1_0.dll",
}
LINUX_LOADER = "/lib64/ld-linux-x86-64.so.2"
LINUX_SYSTEM = {
    "libc.so.6", "libm.so.6", "libgcc_s.so.1", "libdl.so.2", "libpthread.so.0", "librt.so.1",
    "ld-linux-x86-64.so.2", "libasound.so.2", "libudev.so.1",
}
MUSL_LOADER = "/lib/ld-musl-x86_64.so.1"
MUSL_SYSTEM = {"libc.musl-x86_64.so.1", "libgcc_s.so.1", "libasound.so.2", "libudev.so.1"}
# The game's sound (cpal) and controllers (gilrs): a desktop has them, a server image may not.
LINUX_DESKTOP_ONLY = {"libasound.so.2", "libudev.so.1"}

# `--windows7`: API-set DLLs the Windows 7 loader refuses. The default profile allows every
# `api-ms-win-*` set because Windows 10 and 11 resolve them all; Windows 7 resolves only the
# UCRT's `api-ms-win-crt-*` (once KB2999226 is installed) and a handful of core sets. WinRT is
# absent from Windows 7 entirely, and `PathCch*` arrived in Windows 8, so a static import of
# either refuses the image before `main` (docs/design/opengl2-win7-x86.md §19.5, §19.9).
WINDOWS7_ABSENT_PREFIXES = ("api-ms-win-core-winrt", "api-ms-win-core-path")
WINDOWS7_ABSENT = {"xinput1_4.dll", "d3d12.dll", "dcomp.dll", "dwrite.dll", "shcore.dll"}
# Needs KB2999226 (the Universal C Runtime update) — installable, unlike WinRT.
WINDOWS7_UCRT_PREFIX = "api-ms-win-crt-"

# `--windows7` also refuses *entry points* Windows 7 does not export. A DLL name check says
# nothing here: kernel32.dll exists on Windows 7, but not every procedure in it does, and a
# missing procedure refuses the image with "The procedure entry point X could not be located in
# the dynamic link library KERNEL32.dll" before `main` runs (docs/design/opengl2-win7-x86.md §19.13).
#
# Only names that are wrong for a *static* import belong here: an export added after Window 7's
# kernel32/ntdll/user32. Loading one with GetProcAddress at run time is fine and leaves no import.
WINDOWS7_ABSENT_EXPORTS = {
    "kernel32.dll": {
        "GetCurrentThreadStackLimits": "Windows 8",   # horizon-guest-stack measures instead
        "WaitOnAddress": "Windows 8",
        "WakeByAddressAll": "Windows 8",
        "WakeByAddressSingle": "Windows 8",
        "CreateFile2": "Windows 8",
        "CopyFile2": "Windows 8",
        "SetThreadDescription": "Windows 10 1607",
        "GetThreadDescription": "Windows 10 1607",
        "SetThreadInformation": "Windows 8",
        "GetSystemCpuSetInformation": "Windows 10",
        "CreatePseudoConsole": "Windows 10 1809",
        "PowerCreateRequest": "Windows 8",
    },
}


def windows_allowed(name):
    name = name.lower()
    return name in WINDOWS_SYSTEM or name.startswith(("api-ms-win-", "ext-ms-win-"))


def windows7_problem(name):
    """Why Windows 7 would refuse a static import of `name`, or None if it loads.

    Only what cannot be installed counts: WinRT is absent from Windows 7 with no way to add it,
    and `PathCch*` arrived in Windows 8. The UCRT is a prerequisite that KB2999226 provides, so it
    is reported separately (`windows7_prerequisite`) rather than failing the check.
    """
    name = name.lower()
    if name in WINDOWS7_ABSENT or name.startswith(WINDOWS7_ABSENT_PREFIXES):
        return f"imports {name}, which Windows 7 does not have"
    return None


def windows7_prerequisite(name):
    """The update Windows 7 needs for `name`, or None if it loads as shipped."""
    name = name.lower()
    if name.startswith(WINDOWS7_UCRT_PREFIX) or name == "ucrtbase.dll":
        return name
    return None


def windows7_missing_exports(dll, procedures):
    """`(name, version)` for each of `procedures` that `dll` gained after Windows 7."""
    absent = WINDOWS7_ABSENT_EXPORTS.get(dll.lower(), {})
    return [(name, absent[name]) for name in procedures if name in absent]


def pe_imports(data):
    """{dll: [imported procedure names]} for a PE32/PE32+ file's import and delay-import tables.

    Index 0 of a thunk array is the import's by-name flag, not a name, so names start at 1.
    """
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("not a PE file")
    sections_count, optional_size = struct.unpack_from("<H12xH", data, pe + 6)
    optional = pe + 24
    magic = struct.unpack_from("<H", data, optional)[0]
    wide = magic == 0x20B
    directories = optional + (112 if wide else 96)
    sections = [struct.unpack_from("<8xIIII", data, optional + optional_size + 40 * i)
                for i in range(sections_count)]

    def offset(rva):
        for size, address, raw_size, raw in sections:
            if address <= rva < address + max(size, raw_size):
                return rva - address + raw
        raise ValueError(f"RVA {rva:#x} outside every section")

    def name_at(rva):
        start = offset(rva)
        return data[start:data.index(b"\0", start)].decode("ascii")

    thunk = "<Q" if wide else "<I"
    thunk_size = 8 if wide else 4
    ordinal_flag = 1 << (63 if wide else 31)

    def procedure_names(rva):
        names = []
        while rva:
            value = struct.unpack_from(thunk, data, offset(rva))[0]
            if not value:
                break
            if not value & ordinal_flag:
                # IMAGE_IMPORT_BY_NAME: a 2-byte hint, then the NUL-terminated name.
                names.append(name_at(value + 2))
            rva += thunk_size
        return names

    imports = {}
    # Directory 1: imports (20-byte descriptors, name at +12, thunks at +0); 13: delay imports
    # (32 bytes, name at +4, thunks at +16).
    for index, size, name_field, thunk_field in ((1, 20, 12, 0), (13, 32, 4, 16)):
        rva = struct.unpack_from("<I", data, directories + 8 * index)[0]
        if not rva:
            continue
        at = offset(rva)
        while any(data[at:at + size]):
            dll = name_at(struct.unpack_from("<I", data, at + name_field)[0]).lower()
            thunks = struct.unpack_from("<I", data, at + thunk_field)[0]
            imports.setdefault(dll, []).extend(procedure_names(thunks))
            at += size
    return imports


def elf_needs(data):
    """The interpreter and DT_NEEDED libraries of a little-endian ELF64 file."""
    if data[:4] != b"\x7fELF" or data[4] != 2:
        raise ValueError("not an ELF64 file")
    phoff, phentsize, phnum = struct.unpack_from("<Q", data, 32)[0], *struct.unpack_from("<HH", data, 54)
    interpreter, dynamic, loads = None, None, []
    for i in range(phnum):
        kind, _, offset, vaddr, _, filesz = struct.unpack_from("<IIQQQQ", data, phoff + i * phentsize)
        if kind == 3:
            interpreter = data[offset:offset + filesz].rstrip(b"\0").decode()
        elif kind == 2:
            dynamic = (offset, filesz)
        elif kind == 1:
            loads.append((vaddr, offset, filesz))

    def file_offset(address):
        for vaddr, offset, size in loads:
            if vaddr <= address < vaddr + size:
                return address - vaddr + offset
        raise ValueError(f"address {address:#x} outside every segment")

    entries = []
    if dynamic:
        for at in range(dynamic[0], dynamic[0] + dynamic[1], 16):
            tag, value = struct.unpack_from("<qQ", data, at)
            if tag == 0:
                break
            entries.append((tag, value))
    strtab = next((file_offset(v) for t, v in entries if t == 5), None)

    def string(index):
        start = strtab + index
        return data[start:data.index(b"\0", start)].decode()

    needed = [string(v) for t, v in entries if t == 1]
    runpath = [string(v) for t, v in entries if t in (15, 29)]
    return interpreter, needed, runpath


def check(path, server=False, musl=False, windows7=False):
    data = Path(path).read_bytes()
    problems = []
    if data[:2] == b"MZ":
        if musl:
            problems.append("expected a musl ELF binary, got a Windows executable")
        names = pe_imports(data)
        problems += [f"imports {n}, which Windows does not ship" for n in names if not windows_allowed(n)]
        if windows7:
            problems += [p for p in map(windows7_problem, names) if p]
            problems += [f"imports {name} from {dll}, which arrived in {version}"
                         f" and Windows 7 does not export it"
                         for dll, procedures in names.items()
                         for name, version in windows7_missing_exports(dll, procedures)]
            prerequisites = sorted({p for p in map(windows7_prerequisite, names) if p})
            if prerequisites:
                print(f"  {path}: needs KB2999226 (the Universal C Runtime) for "
                      f"{len(prerequisites)} api-ms-win-crt-* imports")
        # One line per distinct DLL: a DLL can appear in several import descriptors.
        problems = list(dict.fromkeys(problems))
        summary = ", ".join(sorted(set(n.lower() for n in names)))
    else:
        interpreter, needed, runpath = elf_needs(data)
        loader = MUSL_LOADER if musl else LINUX_LOADER
        libraries = MUSL_SYSTEM if musl else LINUX_SYSTEM
        # The desktop musl build must be dynamic: GPU and window libraries use dlopen.
        if interpreter != loader and (musl or interpreter is not None):
            problems.append(f"loader is {interpreter}, not {loader}")
        problems += [f"needs {n}, which not every distribution has" for n in needed if n not in libraries]
        if server:
            problems += [f"needs {n}, which a server may not have" for n in needed if n in LINUX_DESKTOP_ONLY]
        problems += [f"has a run path to {p}" for p in runpath if "/nix/store" in p]
        summary = f"{interpreter}; " + ", ".join(needed)
    print(f"{path}: {summary}")
    for problem in problems:
        print(f"  {path} {problem}", file=sys.stderr)
    return not problems


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("binaries", nargs="+")
    parser.add_argument("--server", action="store_true",
                        help="a server build: not even ALSA or udev (the game's sound and controllers)")
    parser.add_argument("--musl", action="store_true", help="require dynamic musl instead of glibc")
    parser.add_argument("--windows7", action="store_true",
                        help="the binary must load on Windows 7 SP1: no WinRT, no PathCch (UCRT "
                             "imports are reported as needing KB2999226)")
    args = parser.parse_args()
    ok = all([check(path, args.server, args.musl, args.windows7) for path in args.binaries])
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
