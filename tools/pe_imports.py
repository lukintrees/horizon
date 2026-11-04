#!/usr/bin/env python3
"""List the normal static imports in a PE32/PE32+ file, without third-party modules.

This is an inventory, not a Windows-version compatibility test: delay imports,
LoadLibrary/GetProcAddress, manifests, drivers and API behavior need separate checks.
"""
import argparse
from pathlib import Path
import struct


# A deliberately incomplete list: these exports were introduced after Windows 7.
# Absence of findings does not prove compatibility, especially with dynamic loading.
POST_WIN7 = {
    'kernel32.dll': {
        'GetSystemTimePreciseAsFileTime', 'WaitOnAddress', 'WakeByAddressSingle',
        'WakeByAddressAll', 'GetSystemTimeAdjustmentPrecise',
    },
    'user32.dll': {
        'GetDpiForWindow', 'GetSystemMetricsForDpi', 'AdjustWindowRectExForDpi',
        'SetProcessDpiAwarenessContext', 'SetThreadDpiAwarenessContext',
    },
}


def win7_findings(result):
    return [(dll, symbol) for dll, symbols in result for symbol in symbols
            if symbol in POST_WIN7.get(dll.lower(), set())]


def imports(data, include_slots=False):
    """Read normal imports. With include_slots, return (name, IAT RVA) pairs."""
    def u16(offset):
        return struct.unpack_from('<H', data, offset)[0]

    def u32(offset):
        return struct.unpack_from('<I', data, offset)[0]

    def string(offset):
        end = data.find(b'\0', offset)
        if offset < 0 or offset >= len(data) or end < 0:
            raise ValueError('invalid string offset')
        return data[offset:end].decode('ascii')

    try:
        if data[:2] != b'MZ':
            raise ValueError('not a PE file')
        pe = u32(0x3c)
        if data[pe:pe + 4] != b'PE\0\0':
            raise ValueError('invalid PE signature')
        coff = pe + 4
        count, opt_size = u16(coff + 2), u16(coff + 16)
        optional = coff + 20
        magic = u16(optional)
        if magic == 0x10b:
            word, directory = 4, 96
        elif magic == 0x20b:
            word, directory = 8, 112
        else:
            raise ValueError('unsupported PE optional header')
        if opt_size < directory + 16:
            raise ValueError('missing import directory')
        import_rva = u32(optional + directory + 8)
        header_size = u32(optional + 60)
        table = optional + opt_size
        sections = []
        for i in range(count):
            section = table + 40 * i
            virtual_size, address, raw_size, raw_offset = struct.unpack_from('<IIII', data, section + 8)
            sections.append((address, max(virtual_size, raw_size), raw_size, raw_offset))

        def offset(rva):
            if rva < min(header_size, len(data)):
                return rva
            for address, size, raw_size, raw_offset in sections:
                if address <= rva < address + size:
                    delta = rva - address
                    if delta >= raw_size or raw_offset + delta >= len(data):
                        raise ValueError('RVA is not backed by file data')
                    return raw_offset + delta
            raise ValueError('RVA outside PE sections')

        if not import_rva:
            return []
        result = []
        descriptor = offset(import_rva)
        while True:
            original, stamp, chain, name, first = struct.unpack_from('<IIIII', data, descriptor)
            descriptor += 20
            if not (original or stamp or chain or name or first):
                break
            dll = string(offset(name))
            thunk = offset(original or first)
            symbols = []
            while True:
                entry = struct.unpack_from('<I' if word == 4 else '<Q', data, thunk)[0]
                thunk += word
                if entry == 0:
                    break
                if entry & (1 << (word * 8 - 1)):
                    symbol = f'ordinal:{entry & 0xffff}'
                else:
                    symbol = string(offset(entry) + 2)
                symbols.append((symbol, first + len(symbols) * word) if include_slots else symbol)
            result.append((dll, symbols))
        return result
    except (struct.error, UnicodeError, OverflowError) as error:
        raise ValueError('truncated or malformed PE') from error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('path', type=Path)
    parser.add_argument('--github-annotations', action='store_true',
                        help='also publish each DLL inventory as a GitHub Actions notice')
    parser.add_argument('--check-win7', action='store_true',
                        help='fail on known post-Windows-7 imports (not an exhaustive audit)')
    args = parser.parse_args()
    try:
        result = imports(args.path.read_bytes())
    except (OSError, ValueError) as error:
        parser.exit(1, f'{error}\n')
    print('Static imports only; dynamically loaded APIs and delay imports are not included.')
    for dll, symbols in result:
        print(f'{dll}:')
        for symbol in symbols:
            print(f'  {symbol}')
        if args.github_annotations:
            message = f'{dll}: ' + ', '.join(symbols)
            message = message[:3000].replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
            print(f'::notice::{message}')
    findings = win7_findings(result)
    for dll, symbol in findings:
        message = f'Windows 7 incompatibility: {dll}!{symbol} is a known post-Windows-7 static import'
        print(f'::warning::{message}' if args.github_annotations else message)
    if args.check_win7 and findings:
        parser.exit(1, 'Known incompatible Windows 7 imports found.\n')


if __name__ == '__main__':
    main()
