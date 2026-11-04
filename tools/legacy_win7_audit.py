#!/usr/bin/env python3
"""Bounded, dependency-free PE import inventory; NOT Win7 compatibility certification.

Exit 0: requested static checks passed; 1: static violations; 2: unreadable,
malformed or unsupported input. Audits only explicitly supplied files, never loads
DLLs. Deny rules are exact DLL / DLL!symbol matches, not an OS API database.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import struct

MAX_FILE = 512 * 1024 * 1024
LIMITS = ["dynamic API loading/fallbacks", "transitive DLL/export resolution",
          "CRT deployment or statically linked CRT code", "allocator and CPU ISA",
          "Win7/X1650 execution and full-game acceptance"]


class AuditError(ValueError):
    """Input cannot be inventoried safely; never silently omit its imports."""


def require(condition, message):
    if not condition:
        raise AuditError(message)


class PE:
    def __init__(self, data: bytes):
        self.data = data
        self.symbol_budget = 65536
        require(len(data) <= MAX_FILE, "file exceeds 512 MiB audit limit")
        require(data[:2] == b"MZ", "missing DOS MZ signature")
        pe = self.unpack("I", 0x3C)[0]
        require(pe >= 0x40 and self.bytes(pe, 4) == b"PE\0\0", "missing PE signature")
        machine, count, _, _, _, optional_size, flags = self.unpack("HHIIIHH", pe + 4)
        optional = pe + 24
        self.bytes(optional, optional_size)
        magic = self.unpack("H", optional)[0]
        require(magic in (0x10B, 0x20B), "unsupported optional-header magic")
        self.width = 4 if magic == 0x10B else 8
        directory_offset = 96 if self.width == 4 else 112
        require(optional_size >= directory_offset, "short optional header")
        self.image_base = self.unpack("I" if self.width == 4 else "Q",
                                      optional + (28 if self.width == 4 else 24))[0]
        self.headers = self.unpack("I", optional + 60)[0]
        directories = self.unpack("I", optional + directory_offset - 4)[0]
        require(directories <= (optional_size - directory_offset) // 8,
                "data-directory count exceeds optional header")
        self.directories = [self.unpack("II", optional + directory_offset + i * 8)
                            for i in range(directories)]
        section_table = optional + optional_size
        require(1 <= count <= 96, "unsupported section count (expected 1..96)")
        require(section_table + count * 40 <= self.headers <= len(data),
                "invalid SizeOfHeaders")
        self.sections = []
        for i in range(count):
            virtual_size, rva, raw_size, raw = self.unpack("IIII", section_table + i * 40 + 8)
            span = max(virtual_size, raw_size)
            require(rva >= self.headers and rva + span <= 2**32, "invalid section RVA range")
            if raw_size:
                require(raw >= self.headers, "section overlaps headers")
                self.bytes(raw, raw_size)
            for old_rva, old_span, old_raw, old_size in self.sections:
                require(not (rva < old_rva + old_span and old_rva < rva + span),
                        "overlapping section RVAs")
                require(not (raw_size and old_size and raw < old_raw + old_size
                             and old_raw < raw + raw_size), "overlapping section file ranges")
            self.sections.append((rva, span, raw, raw_size))
        self.metadata = {
            "machine": f"0x{machine:04x}", "pe_kind": "PE32" if self.width == 4 else "PE32+",
            "image_base": hex(self.image_base), "large_address_aware": bool(flags & 0x20),
            "os_version": list(self.unpack("HH", optional + 40)),
            "subsystem_version": list(self.unpack("HH", optional + 48)),
            "subsystem": self.unpack("H", optional + 68)[0],
        }

    def bytes(self, offset, size):
        require(0 <= offset <= len(self.data) and 0 <= size <= len(self.data) - offset,
                "truncated file range")
        return self.data[offset:offset + size]

    def unpack(self, fmt, offset):
        return struct.unpack("<" + fmt, self.bytes(offset, struct.calcsize("<" + fmt)))

    def mapped(self, rva):
        require(0 < rva < 2**32, "invalid/null RVA")
        if rva < self.headers:
            return rva, self.headers - rva
        for start, _, raw, size in self.sections:
            if start <= rva < start + size:
                return raw + rva - start, start + size - rva
        raise AuditError(f"unmapped or zero-fill RVA {rva:#x}")

    def at(self, rva, size):
        offset, available = self.mapped(rva)
        require(size <= available, "RVA range crosses file-backed mapping")
        return self.bytes(offset, size)

    def string(self, rva):
        offset, available = self.mapped(rva)
        value = self.data[offset:offset + min(available, 4096)].split(b"\0", 1)
        require(len(value) == 2, "unterminated/overlong import string")
        require(value[0] and all(32 <= c < 127 for c in value[0]), "invalid import string")
        return value[0].decode("ascii")

    def pointer(self, value, legacy):
        return value - self.image_base if legacy else value

    def symbols(self, rva, legacy=False):
        result = []
        ordinal_bit = 1 << (self.width * 8 - 1)
        while self.symbol_budget:
            self.symbol_budget -= 1
            value = int.from_bytes(self.at(rva, self.width), "little")
            if value == 0:
                return result
            if value & ordinal_bit:
                require(value & ~(ordinal_bit | 0xFFFF) == 0, "invalid ordinal thunk bits")
                result.append(f"#{value & 0xFFFF}")
            else:
                name = self.pointer(value, legacy)
                self.at(name, 2)  # IMAGE_IMPORT_BY_NAME hint
                result.append(self.string(name + 2))
            rva += self.width
        raise AuditError("import symbol budget exceeded (65536)")

    def imports(self, index, delay=False):
        if index >= len(self.directories):
            return []
        rva, size = self.directories[index]
        if not (rva or size):
            return []
        require(rva and size, "incomplete import directory")
        self.at(rva, size)
        stride = 32 if delay else 20
        result = []
        for i in range(min(size // stride, 4096)):
            fields = struct.unpack("<" + "I" * (stride // 4), self.at(rva + i * stride, stride))
            if not any(fields):
                return result
            legacy = False
            if delay:
                attrs, name, _, iat, table, bound, _, stamp = fields
                require(attrs in (0, 1), "unsupported delay-import attributes")
                legacy = attrs == 0
            else:
                table, stamp, _, name, iat = fields
                bound = 0
            require(name and iat, "missing import DLL name or IAT")
            require(table or not (stamp or bound), "bound IAT without name table is unsupported")
            result.append({"kind": "delay" if delay else "normal",
                           "dll": self.string(self.pointer(name, legacy)),
                           "symbols": self.symbols(self.pointer(table or iat, legacy), legacy)})
        raise AuditError("unterminated import directory or descriptor limit (4096)")


def crt_family(dll):
    dll = dll.lower()
    if dll in ("ucrtbase.dll", "ucrtbased.dll") or dll.startswith("api-ms-win-crt-"):
        return "ucrt"
    if dll == "msvcrt.dll":
        return "legacy-msvcrt"
    if re.fullmatch(r"(?:vcruntime|msvcr|msvcp|concrt|vcomp)[\w.]*\.dll", dll):
        return "msvc-runtime"
    return None


def audit_pe(data, deny_dll=(), deny_import=()):
    pe = PE(data)
    imports = pe.imports(1) + pe.imports(13, delay=True)
    violations = []
    if pe.metadata["machine"] != "0x014c" or pe.width != 4:
        violations.append("expected IMAGE_FILE_MACHINE_I386 + PE32")
    for field in ("os_version", "subsystem_version"):
        if tuple(pe.metadata[field]) > (6, 1):
            violations.append(f"{field} exceeds Win7 6.1")
    denied_dlls = {dll.lower() for dll in deny_dll}
    denied_imports = {(dll.lower(), symbol) for dll, symbol in deny_import}
    for entry in imports:
        dll = entry["dll"].lower()
        if dll in denied_dlls:
            violations.append(f"{entry['kind']}: denied DLL {entry['dll']}")
        for symbol in entry["symbols"]:
            if (dll, symbol) in denied_imports:
                violations.append(f"{entry['kind']}: denied import {entry['dll']}!{symbol}")
    return {**pe.metadata, "size": len(data), "sha256": hashlib.sha256(data).hexdigest(),
            "imports": imports,
            "crt_dependencies": [{"kind": e["kind"], "dll": e["dll"], "family": crt_family(e["dll"])}
                                 for e in imports if crt_family(e["dll"])],
            "violations": violations,
            "status": "static_checks_failed" if violations else "static_checks_passed",
            "win7_qualified": False}


def import_rule(value):
    dll, sep, symbol = value.partition("!")
    if not dll or not sep or not symbol:
        raise argparse.ArgumentTypeError("expected DLL!symbol (or DLL!#ordinal)")
    return dll, symbol


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("files", type=Path, nargs="+")
    parser.add_argument("--deny-dll", action="append", default=[], metavar="DLL")
    parser.add_argument("--deny-import", type=import_rule, action="append", default=[], metavar="DLL!SYMBOL")
    args = parser.parse_args(argv)
    reports, code = [], 0
    for path in args.files:
        try:
            with path.open("rb") as stream:
                data = stream.read(MAX_FILE + 1)
            report = audit_pe(data, args.deny_dll, args.deny_import)
            code = max(code, int(bool(report["violations"])))
        except (OSError, AuditError) as error:
            report = {"status": "invalid", "error": str(error), "win7_qualified": False}
            code = 2
        reports.append({"path": str(path), **report})
    print(json.dumps({"schema_version": 1, "files": reports, "not_proven": LIMITS,
                      "deny_dll": args.deny_dll, "deny_import": args.deny_import}, indent=2))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
