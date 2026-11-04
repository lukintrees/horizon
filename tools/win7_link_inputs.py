#!/usr/bin/env python3
"""Inspect rustc's temporary COFF inputs *during* the final Win7 link.

A successful rustc link removes its temporary object and raw-dylib import
libraries before an ordinary post-build audit can inspect them. This diagnostic
runs from the linker wrapper while they still exist. It is not a compatibility
check and may show objects that the linker ultimately discards.
"""
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

SYMBOL = 'GetSystemTimePreciseAsFileTime'
# x86 COFF may decorate a direct symbol as __imp__Name@4 or _Name@4.
DIRECT = re.compile(r'(?:^|\s)(?:__imp_+|_+)?' + SYMBOL + r'(?:@\d+)?(?:\s|$)')


def main():
    nm = os.environ['HORIZON_LLVM_NM']
    root = Path(tempfile.gettempdir())
    directories = [p for p in root.glob('rustc*') if p.is_dir()]
    # cargo rustc can place its link intermediates under the target tree rather
    # than the OS temp directory. Scan full archives too: the earlier undefined-
    # symbol audit cannot detect a defined raw-dylib import table member.
    for part in ('build', 'deps'):
        directory = Path('target/i686-win7-windows-msvc/debug') / part
        if directory.is_dir():
            directories.append(directory)
    sdk = Path(os.environ.get('ProgramFiles(x86)', '')) / 'Windows Kits/10/Lib'
    if sdk.is_dir():
        directories.append(sdk)
    vc = Path(os.environ.get('VCToolsInstallDir', '')) / 'lib/x86'
    if vc.is_dir():
        directories.append(vc)
    print(f'Input search roots: {directories}', flush=True)
    examined = matches = 0
    without_direct = []
    for directory in directories:
        for path in directory.rglob('*'):
            if directory == sdk and 'x86' not in (part.lower() for part in path.parts):
                continue
            if path.suffix.lower() not in ('.o', '.obj', '.lib', '.a', '.rlib') or not path.is_file():
                continue
            needle = SYMBOL.encode()
            try:
                found = False
                tail = b''
                with path.open('rb') as stream:
                    while block := stream.read(1024 * 1024):
                        data = tail + block
                        if needle in data:
                            found = True
                            break
                        tail = data[-len(needle):]
            except OSError:
                continue
            examined += 1
            if not found:
                continue
            result = subprocess.run([nm, '--print-file-name', str(path)], capture_output=True, text=True, errors='replace', check=False)
            direct = [line for line in result.stdout.splitlines() if DIRECT.search(line)]
            if direct:
                for line in direct[:20]:
                    print('DIRECT: ' + line, flush=True)
                    matches += 1
            else:
                without_direct.append(str(path))
                if result.returncode and result.stderr:
                    print('llvm-nm: ' + result.stderr[:500].replace('\n', ' '), flush=True)
    print(f'Examined {examined} inputs; {matches} direct symbol records; {len(without_direct)} inputs contain only non-direct matches.', flush=True)
    for path in without_direct[:10]:
        print(f'Contains symbol bytes, no direct symbol: {path}', flush=True)
    return 0


if __name__ == '__main__':
    sys.exit(main())
