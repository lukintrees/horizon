#!/usr/bin/env python3
"""Inspect rustc's temporary COFF inputs *during* the final Win7 link.

A successful rustc link removes its temporary object and raw-dylib import
libraries before an ordinary post-build audit can inspect them. This diagnostic
runs from the linker wrapper while they still exist. It is not a compatibility
check and may show objects that the linker ultimately discards.
"""
import os
import itertools
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile

SYMBOL = 'GetSystemTimePreciseAsFileTime'
# x86 COFF may decorate a direct symbol as __imp__Name@4 or _Name@4.
DIRECT = re.compile(r'(?:^|\s)(?:__imp_+|_+)?' + SYMBOL + r'(?:@\d+)?(?:\s|$)')


def linker_inputs(arguments):
    """Expand MSVC response files while rustc's link temporaries still exist."""
    paths = []
    pending = list(arguments)
    seen_responses = set()
    while pending:
        arg = pending.pop()
        if arg.startswith('@'):
            response = Path(arg[1:].strip('"'))
            if response in seen_responses:
                continue
            seen_responses.add(response)
            try:
                data = response.read_bytes()
                if data.startswith((b'\xff\xfe', b'\xfe\xff')):
                    text = data.decode('utf-16')
                else:
                    text = data.decode('utf-8-sig')
                pending.extend(token.strip('"') for token in shlex.split(text, posix=False))
            except (OSError, UnicodeError, ValueError) as exc:
                print(f'Error reading linker response {response}: {exc}', flush=True)
        else:
            path = Path(arg.strip('"'))
            if path.suffix.lower() in ('.o', '.obj', '.lib', '.a', '.rlib') and path.is_file():
                paths.append(path)
    return paths


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
    inputs = linker_inputs(sys.argv[1:])
    print(f'Linker inputs: {len(inputs)} object and archive paths', flush=True)
    seen = set()
    examined = matches = 0
    without_direct = []
    candidates = itertools.chain(inputs, *(directory.rglob('*') for directory in directories))
    for path in candidates:
        if sdk in path.parents and 'x86' not in (part.lower() for part in path.parts):
            continue
        if path.suffix.lower() not in ('.o', '.obj', '.lib', '.a', '.rlib') or not path.is_file():
            continue
        key = str(path.resolve()).lower()
        if key in seen:
            continue
        seen.add(key)
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
