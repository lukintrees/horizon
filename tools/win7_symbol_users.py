#!/usr/bin/env python3
"""Find undefined references to one symbol in compiled COFF libraries and object files.

Pass the Rust LLVM nm and directories containing .rlib/.lib files. Output is
an investigation aid, not proof that an object was selected by the linker.
"""
import argparse
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--nm', required=True, type=Path)
    parser.add_argument('--symbol', required=True)
    parser.add_argument('directories', nargs='+', type=Path)
    args = parser.parse_args()
    needle = args.symbol.encode('ascii')
    scanned = set()
    for directory in args.directories:
        if not directory.exists():
            continue
        for path in directory.rglob('*'):
            if path.suffix.lower() not in ('.lib', '.rlib', '.o', '.obj') or path in scanned:
                continue
            scanned.add(path)
            try:
                if needle not in path.read_bytes():
                    continue
                result = subprocess.run([str(args.nm), '--undefined-only', '--print-file-name', str(path)],
                                        capture_output=True, text=True, errors='replace', check=False)
                if result.returncode and result.stderr:
                    detail = result.stderr.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')[:1500]
                    print(f'::notice::llvm-nm could not inspect {path}: {detail}')
                for line in result.stdout.splitlines():
                    if args.symbol in line:
                        # An annotation can otherwise be truncated by the runner.
                        text = line[-2500:].replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
                        print('::notice::' + text)
            except OSError as error:
                print(f'could not inspect {path}: {error}')
    print(f'inspected {len(scanned)} COFF archives')


if __name__ == '__main__':
    main()
