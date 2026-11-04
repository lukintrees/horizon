#!/usr/bin/env python3
"""Create an unsigned Win7 x86 EXE copy with a coarse wall-clock import.

GetSystemTimePreciseAsFileTime and GetSystemTimeAsFileTime have the same Win32
void WINAPI(FILETIME*) ABI. The latter is available on Windows 7 but has lower
resolution. Only ordinary, unbound kernel32 name imports are rewritten. This
is NOT a general PE compatibility fix, and it does not prove the game will run.
"""
import argparse
import os
from pathlib import Path
import struct
import tempfile

import pe_imports

PRECISE = 'GetSystemTimePreciseAsFileTime'
COARSE = 'GetSystemTimeAsFileTime'


def patch(data):
    """Return (new bytes, count); reject malformed, signed, bound or non-x86 PE."""
    # Parse imports first, so malformed files never reach raw header accesses.
    entries = pe_imports.imports(data, include_details=True)
    try:
        pe = struct.unpack_from('<I', data, 0x3c)[0]
        coff = pe + 4
        machine, optional_size, characteristics = struct.unpack_from('<H14xHH', data, coff)
        optional = coff + 20
        magic = struct.unpack_from('<H', data, optional)[0]
        if machine != 0x14c or magic != 0x10b or not characteristics & 2 or characteristics & 0x2000:
            raise ValueError('only PE32 i686 executables are supported')
        # Unlike other data directories, the security directory contains a file
        # offset, not an RVA. Rewriting an Authenticode-signed image invalidates it.
        if optional_size < 96 + 8 * 5:
            raise ValueError('missing PE security directory')
        cert_offset, cert_size = struct.unpack_from('<II', data, optional + 96 + 8 * 4)
        if cert_offset or cert_size:
            raise ValueError('signed images must not be rewritten')
    except struct.error as error:
        raise ValueError('truncated PE header') from error

    targets = []
    users = {}
    for dll, symbols in entries:
        for name, _slot, name_offset, stamp in symbols:
            if name_offset is not None:
                users.setdefault(name_offset, []).append((dll.lower(), name))
            if dll.lower() == 'kernel32.dll' and name == PRECISE:
                if stamp:
                    raise ValueError('bound imports must be relinked before rewriting')
                targets.append(name_offset)
    if not targets:
        raise ValueError('no kernel32 precise-time import found')
    for offset in targets:
        if any(dll != 'kernel32.dll' or name != PRECISE for dll, name in users[offset]):
            raise ValueError('import name is shared with an unrelated DLL')
        for other, references in users.items():
            if other != offset and any(other - 2 < offset + len(PRECISE) + 1
                                       and other + len(name) + 1 > offset - 2
                                       for _dll, name in references):
                raise ValueError('overlapping import name records')
        if offset < 2 or data[offset:offset + len(PRECISE) + 1] != PRECISE.encode() + b'\0':
            raise ValueError('invalid import name')

    result = bytearray(data)
    for offset in set(targets):
        # The old hint refers to a different export; reset it and let the
        # loader search by name. Zero-fill the remaining bytes in-place.
        result[offset - 2:offset] = b'\0\0'
        replacement = COARSE.encode() + b'\0'
        result[offset:offset + len(PRECISE) + 1] = replacement.ljust(len(PRECISE) + 1, b'\0')
    # Ordinary user-mode EXEs do not need a PE checksum; the old one is stale.
    struct.pack_into('<I', result, optional + 64, 0)
    expected = [(dll, [COARSE if dll.lower() == 'kernel32.dll' and name == PRECISE else name
                       for name, _slot, _offset, _stamp in symbols]) for dll, symbols in entries]
    if pe_imports.imports(result) != expected:
        raise ValueError('post-patch import verification failed')
    return bytes(result), len(targets)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path, help='separate output EXE; may be replaced')
    args = parser.parse_args()
    try:
        if args.source.resolve() == args.output.resolve():
            raise ValueError('source and output must be different files')
        result, count = patch(args.source.read_bytes())
        # Never truncate an existing output if parsing or writing fails.
        temp_name = None
        try:
            with tempfile.NamedTemporaryFile(dir=args.output.parent, delete=False) as temp:
                temp_name = temp.name
                temp.write(result)
            os.replace(temp_name, args.output)
        finally:
            if temp_name and os.path.exists(temp_name):
                os.unlink(temp_name)
        print(f'{args.output}: replaced {count} precise time import(s) with coarse Win7 time; source unchanged')
        print('Other compatibility issues and reduced time precision remain; runtime is not verified.')
    except (OSError, ValueError) as error:
        parser.exit(1, f'{error}\n')


if __name__ == '__main__':
    main()
