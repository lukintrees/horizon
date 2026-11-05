#!/usr/bin/env python3
"""Inspect PE architecture and ordinary/delay/CRT imports without executing the file.

This is a bounded, read-only parser using only the Python standard library. It reports
facts for a Win7 x86 build audit; it does not establish OS, CPU, CRT, DLL, or runtime
compatibility. See docs/design/legacy-win7-build-audit.md for those limitations.
"""

import argparse
import hashlib
import json
import struct
import sys
from pathlib import Path

MACHINES = {
    0x014C: "i686",
    0x01C0: "arm",
    0x01C4: "armv7",
    0x8664: "x86_64",
    0xAA64: "arm64",
}
MACHINE_MAGIC = {0x014C: 0x10B, 0x01C0: 0x10B, 0x01C4: 0x10B,
                 0x8664: 0x20B, 0xAA64: 0x20B}
IMPORT_DIRECTORY = 1
DELAY_IMPORT_DIRECTORY = 13
MAX_DESCRIPTORS = 4096
MAX_THUNKS = 65536
MAX_STRING = 4096


class PeError(ValueError):
    """The input is not a supported, file-backed PE image."""


def _unpack(fmt, data, offset, what):
    size = struct.calcsize(fmt)
    if offset < 0 or offset + size > len(data):
        raise PeError(f"truncated {what}")
    return struct.unpack_from(fmt, data, offset)


def _crt_family(dll):
    name = dll.lower()
    if name == "msvcrt.dll":
        return "legacy-msvcrt"
    if name in ("ucrtbase.dll", "ucrtbased.dll") or name.startswith("api-ms-win-crt-"):
        return "universal-crt"
    if name.startswith(("msvcr", "msvcp", "vcruntime", "vccorlib", "concrt")):
        return "msvc-runtime"
    return None


class PeImage:
    def __init__(self, data):
        self.data = data
        self.sections = []
        self._parse_headers()

    def _parse_headers(self):
        data = self.data
        if len(data) < 64 or data[:2] != b"MZ":
            raise PeError("missing DOS MZ header")
        pe_offset = _unpack("<I", data, 0x3C, "DOS e_lfanew")[0]
        if _unpack("<I", data, pe_offset, "PE signature")[0] != 0x00004550:
            raise PeError("missing PE signature")
        coff = pe_offset + 4
        (self.machine, section_count, _timestamp, _symbols, _symbol_count,
         optional_size, characteristics) = _unpack("<HHIIIHH", data, coff, "COFF header")
        if not 1 <= section_count <= 96:
            raise PeError(f"invalid section count {section_count}")
        optional = coff + 20
        if optional + optional_size > len(data) or optional_size < 64:
            raise PeError("truncated optional header")
        self.magic = _unpack("<H", data, optional, "optional-header magic")[0]
        if self.magic not in (0x10B, 0x20B):
            raise PeError(f"unsupported optional-header magic 0x{self.magic:04x}")
        expected_magic = MACHINE_MAGIC.get(self.machine)
        if expected_magic is not None and self.magic != expected_magic:
            raise PeError("COFF machine and PE32/PE32+ format disagree")
        self.pe64 = self.magic == 0x20B
        self.image_base = _unpack("<Q" if self.pe64 else "<I", data,
                                  optional + (24 if self.pe64 else 28), "image base")[0]
        self.os_version = _unpack("<HH", data, optional + 40, "OS version")
        self.subsystem_version = _unpack("<HH", data, optional + 48, "subsystem version")
        self.header_size = _unpack("<I", data, optional + 60, "SizeOfHeaders")[0]
        self.characteristics = characteristics
        directory_count_offset = optional + (108 if self.pe64 else 92)
        directories_offset = optional + (112 if self.pe64 else 96)
        if directory_count_offset + 4 > optional + optional_size:
            raise PeError("optional header has no data-directory count")
        self.directory_count = _unpack("<I", data, directory_count_offset,
                                       "data-directory count")[0]
        available = (optional + optional_size - directories_offset) // 8
        if directories_offset > optional + optional_size or available < min(self.directory_count, 16):
            raise PeError("truncated data directories")
        self.directories_offset = directories_offset
        section_table = optional + optional_size
        if section_table + section_count * 40 > len(data):
            raise PeError("truncated section table")
        for index in range(section_count):
            offset = section_table + index * 40
            name = data[offset:offset + 8].split(b"\0", 1)[0].decode("ascii", "replace")
            fields = _unpack("<IIIIIIHHI", data, offset + 8, f"section {name}")
            self.sections.append((name, *fields))

    def directory(self, index):
        if index >= min(self.directory_count, 16):
            return 0, 0
        return _unpack("<II", self.data, self.directories_offset + index * 8,
                       f"data directory {index}")

    def offset(self, rva, size=1):
        if size < 0:
            raise PeError("negative read size")
        if rva < self.header_size:
            if rva + size <= self.header_size and rva + size <= len(self.data):
                return rva
            raise PeError(f"unmapped header RVA 0x{rva:x}")
        for (name, virtual_size, virtual_address, raw_size, raw_pointer,
             *_rest) in self.sections:
            delta = rva - virtual_address
            if 0 <= delta < max(virtual_size, raw_size):
                if raw_pointer == 0 or delta + size > raw_size:
                    raise PeError(f"RVA 0x{rva:x} in {name} is not file-backed")
                offset = raw_pointer + delta
                if offset + size > len(self.data):
                    raise PeError(f"truncated {name} at RVA 0x{rva:x}")
                return offset
        raise PeError(f"unmapped RVA 0x{rva:x}")

    def read(self, rva, size, what):
        return self.data[self.offset(rva, size):self.offset(rva, size) + size]

    def number(self, fmt, rva, what):
        return _unpack(fmt, self.data, self.offset(rva, struct.calcsize(fmt)), what)

    def string(self, rva, what):
        chars = bytearray()
        for index in range(MAX_STRING):
            char = self.read(rva + index, 1, what)[0]
            if char == 0:
                try:
                    return chars.decode("ascii")
                except UnicodeDecodeError as error:
                    raise PeError(f"non-ASCII {what}") from error
            chars.append(char)
        raise PeError(f"unterminated {what}")

    def descriptors(self, index, size, label):
        rva, directory_size = self.directory(index)
        if rva == 0 and directory_size == 0:
            return [], 0
        if rva == 0 or directory_size < size:
            raise PeError(f"invalid {label} directory")
        count = min(directory_size // size, MAX_DESCRIPTORS)
        result = []
        for item in range(count):
            values = self.number("<" + "I" * (size // 4), rva + item * size,
                                 f"{label} descriptor {item}")
            if not any(values):
                return result, directory_size
            result.append((rva + item * size, values))
        raise PeError(f"unterminated {label} descriptor table")

    def thunks(self, rva, label):
        width = 8 if self.pe64 else 4
        fmt = "<Q" if self.pe64 else "<I"
        ordinal_flag = 1 << (63 if self.pe64 else 31)
        address_mask = ordinal_flag - 1
        symbols = []
        for index in range(MAX_THUNKS):
            value = self.number(fmt, rva + index * width, f"{label} thunk {index}")[0]
            if value == 0:
                return symbols
            if value & ordinal_flag:
                symbols.append({"ordinal": value & 0xFFFF})
                continue
            hint = self.number("<H", value & address_mask, f"{label} import hint")[0]
            name = self.string((value & address_mask) + 2, f"{label} import name")
            symbols.append({"name": name, "hint": hint})
        raise PeError(f"unterminated {label} thunk table")

    def ordinary_imports(self):
        modules = []
        descriptors, _size = self.descriptors(IMPORT_DIRECTORY, 20, "import")
        for _rva, values in descriptors:
            lookup, _timestamp, _forwarder, name, first_thunk = values
            if name == 0 or (lookup == 0 and first_thunk == 0):
                raise PeError("import descriptor lacks a DLL name or thunk table")
            dll = self.string(name, "DLL name")
            modules.append({"dll": dll, "kind": "normal",
                            "symbols": self.thunks(lookup or first_thunk, dll)})
        return modules

    def delay_imports(self):
        modules = []
        descriptors, _size = self.descriptors(DELAY_IMPORT_DIRECTORY, 32, "delay-import")
        for _rva, values in descriptors:
            attributes, name, _module, iat, lookup, _bound, _unload, _timestamp = values
            if attributes & ~1:
                raise PeError(f"unsupported delay-import attributes 0x{attributes:x}")
            if name == 0 or (lookup == 0 and iat == 0):
                raise PeError("delay-import descriptor lacks a DLL name or thunk table")

            def as_rva(value):
                if attributes & 1:
                    return value
                if value < self.image_base:
                    raise PeError("delay-import VA is below the image base")
                return value - self.image_base

            dll = self.string(as_rva(name), "delay-loaded DLL name")
            modules.append({"dll": dll, "kind": "delay",
                            "symbols": self.thunks(as_rva(lookup or iat), dll)})
        return modules


def audit_bytes(data, expected_arch="i686"):
    """Return a JSON-serializable report or raise PeError for malformed input."""
    image = PeImage(data)
    arch = MACHINES.get(image.machine, f"unknown-0x{image.machine:04x}")
    imports = image.ordinary_imports() + image.delay_imports()
    crt = [{"dll": item["dll"], "kind": item["kind"], "family": family}
           for item in imports if (family := _crt_family(item["dll"])) is not None]
    return {
        "schema": 1,
        "architecture": arch,
        "machine": f"0x{image.machine:04x}",
        "format": "PE32+" if image.pe64 else "PE32",
        "architecture_ok": expected_arch == "any" or arch == expected_arch,
        "expected_architecture": expected_arch,
        "image_base": image.image_base,
        "os_version": ".".join(map(str, image.os_version)),
        "subsystem_version": ".".join(map(str, image.subsystem_version)),
        "is_dll": bool(image.characteristics & 0x2000),
        "large_address_aware": bool(image.characteristics & 0x0020),
        "imports": imports,
        "crt_dependencies": crt,
    }


def audit_file(path, expected_arch="i686"):
    data = Path(path).read_bytes()
    report = audit_bytes(data, expected_arch)
    report.update(path=str(path), size=len(data), sha256=hashlib.sha256(data).hexdigest())
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("binaries", nargs="+", help="PE .exe or .dll files to inspect")
    parser.add_argument("--expected-arch", default="i686",
                        choices=["i686", "x86_64", "arm", "armv7", "arm64", "any"])
    parser.add_argument("--json", dest="json_path",
                        help="write the complete report to this file, or '-' for stdout")
    args = parser.parse_args(argv)
    reports, errors = [], []
    for name in args.binaries:
        try:
            report = audit_file(name, args.expected_arch)
            reports.append(report)
            if args.json_path != "-":
                print(f"{name}: {report['format']} {report['architecture']}, "
                      f"OS {report['os_version']}, subsystem {report['subsystem_version']}")
                for item in report["imports"]:
                    print(f"  {item['kind']} import {item['dll']}: {len(item['symbols'])} symbol(s)")
                for item in report["crt_dependencies"]:
                    print(f"  CRT dependency: {item['dll']} ({item['kind']}, {item['family']})")
                if not report["architecture_ok"]:
                    print(f"  ERROR: expected {args.expected_arch}")
        except (OSError, PeError) as error:
            errors.append({"path": name, "error": str(error)})
            print(f"{name}: ERROR: {error}", file=sys.stderr)
    failed = bool(errors) or any(not report["architecture_ok"] for report in reports)
    result = {"schema": 1, "ok": not failed, "files": reports, "errors": errors}
    if args.json_path:
        encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
        if args.json_path == "-":
            sys.stdout.write(encoded)
        else:
            Path(args.json_path).write_text(encoded, encoding="utf-8")
    if errors:
        return 2
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
