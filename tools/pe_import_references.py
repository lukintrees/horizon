#!/usr/bin/env python3
"""Find candidate x86 instructions referring to a named PE import's IAT slot.

With an MSVC /MAP file, associate each reference with the nearest preceding
public symbol. Byte matching is not disassembly: candidates and nearest symbols
must be verified; indirect calls, address calculations and stripped/local
functions may be missed or misidentified. PE32 only, normal imports only.
"""
import argparse
import bisect
from pathlib import Path
import re
import struct

from pe_imports import imports

MAP_SYMBOL = re.compile(r'^\s*[0-9A-Fa-f]{4}:[0-9A-Fa-f]{8}\s+(\S+)\s+([0-9A-Fa-f]{8,16})\s+(.*)$')


def parse_map(text):
    symbols = []
    for line in text.splitlines():
        match = MAP_SYMBOL.match(line)
        if match:
            symbols.append((int(match.group(2), 16), match.group(1), match.group(3).strip()))
    symbols.sort()
    return symbols


def find_references(data, dll, symbol, map_text=''):
    try:
        pe = struct.unpack_from('<I', data, 0x3c)[0]
        if data[:2] != b'MZ' or data[pe:pe + 4] != b'PE\0\0':
            raise ValueError('not a PE file')
        coff = pe + 4
        count, opt_size = struct.unpack_from('<H', data, coff + 2)[0], struct.unpack_from('<H', data, coff + 16)[0]
        optional = coff + 20
        if struct.unpack_from('<H', data, optional)[0] != 0x10b:
            raise ValueError('only PE32 x86 is supported')
        if struct.unpack_from('<H', data, coff)[0] != 0x14c:
            raise ValueError('only x86 machine code is supported')
        image_base = struct.unpack_from('<I', data, optional + 28)[0]
        slots = [slot for name, entries in imports(data, include_slots=True)
                 if name.lower() == dll.lower() for imported, slot in entries if imported == symbol]
        if not slots:
            return []
        maps = parse_map(map_text)
        addresses = [item[0] for item in maps]
        results = []
        for i in range(count):
            section = optional + opt_size + 40 * i
            _, address, raw_size, raw_offset = struct.unpack_from('<IIII', data, section + 8)
            flags = struct.unpack_from('<I', data, section + 36)[0]
            if not flags & 0x20000000:
                continue
            if raw_offset + raw_size > len(data):
                raise ValueError('truncated executable section')
            body = data[raw_offset:raw_offset + raw_size]
            for slot in slots:
                absolute = image_base + slot
                if absolute > 0xffffffff:
                    raise ValueError('IAT address outside PE32 address space')
                needle = struct.pack('<I', absolute)
                start = 0
                while (index := body.find(needle, start)) != -1:
                    start = index + 1
                    # FF /2 and /4 absolute indirect call/jump, or other x86
                    # encodings with a matching immediate/displacement.
                    prefix = body[max(0, index - 2):index]
                    kind = {'ff15': 'indirect call', 'ff25': 'indirect jump'}.get(prefix.hex(), 'other candidate')
                    va = image_base + address + index
                    map_index = bisect.bisect_right(addresses, va) - 1
                    nearest = maps[map_index] if map_index >= 0 else None
                    results.append((va, kind, absolute, nearest))
        return results
    except (struct.error, OverflowError) as exc:
        raise ValueError('truncated or malformed PE') from exc


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('exe', type=Path)
    parser.add_argument('map', type=Path)
    parser.add_argument('--dll', default='kernel32.dll')
    parser.add_argument('--symbol', default='GetSystemTimePreciseAsFileTime')
    args = parser.parse_args()
    try:
        result = find_references(args.exe.read_bytes(), args.dll, args.symbol, args.map.read_text(errors='replace'))
    except (OSError, ValueError) as error:
        parser.exit(1, f'{error}\n')
    print(f'Candidate code references to {args.dll}!{args.symbol}: {len(result)}')
    for address, kind, slot, nearest in result[:50]:
        if nearest:
            base, name, obj = nearest
            detail = f'{name}+0x{address-base:x} ({obj})'
        else:
            detail = 'no preceding public map symbol'
        print(f'  VA 0x{address:08x}: {kind}, IAT 0x{slot:08x}; nearest {detail}')
    print('Caution: byte matches and nearest public symbols are not proof of the caller.')


if __name__ == '__main__':
    main()
