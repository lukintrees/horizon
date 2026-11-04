import importlib.util
from pathlib import Path
import struct
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools'))
import pe_import_references as refs

spec = importlib.util.spec_from_file_location('pe_imports', Path(__file__).resolve().parents[1] / 'tools/pe_imports.py')
pe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pe)


def fixture(bits):
    data = bytearray(0x600)
    data[:2] = b'MZ'
    struct.pack_into('<I', data, 0x3c, 0x80)
    data[0x80:0x84] = b'PE\0\0'
    coff = 0x84
    opt_size = 0xe0 if bits == 32 else 0xf0
    struct.pack_into('<H', data, coff + 2, 1)
    struct.pack_into('<H', data, coff + 16, opt_size)
    opt = coff + 20
    struct.pack_into('<H', data, opt, 0x10b if bits == 32 else 0x20b)
    struct.pack_into('<I', data, opt + 60, 0x200)
    struct.pack_into('<I', data, opt + (96 if bits == 32 else 112) + 8, 0x1000)
    section = opt + opt_size
    struct.pack_into('<IIII', data, section + 8, 0x400, 0x1000, 0x400, 0x200)
    struct.pack_into('<IIIII', data, 0x200, 0x1040, 0, 0, 0x1080, 0x1040)
    word = '<III' if bits == 32 else '<QQQ'
    struct.pack_into(word, data, 0x240, 0x10a0, (1 << (bits - 1)) | 7, 0)
    data[0x280:0x28d] = b'KERNEL32.dll\0'
    data[0x2a2:0x2ab] = b'ExitTest\0'
    return bytes(data)


class ImportTests(unittest.TestCase):
    def test_32_and_64_bit_named_and_ordinal_imports(self):
        for bits in (32, 64):
            self.assertEqual(pe.imports(fixture(bits)), [('KERNEL32.dll', ['ExitTest', 'ordinal:7'])])

    def test_import_slots_and_candidate_x86_calls(self):
        data = bytearray(fixture(32))
        coff = 0x84
        optional = coff + 20
        section = optional + 0xe0
        struct.pack_into('<H', data, coff, 0x14c)
        struct.pack_into('<I', data, optional + 28, 0x400000)
        struct.pack_into('<I', data, section + 36, 0x20000000)
        data[0x320:0x326] = b'\xff\x15' + struct.pack('<I', 0x401040)
        self.assertEqual(pe.imports(data, include_slots=True), [
            ('KERNEL32.dll', [('ExitTest', 0x1040), ('ordinal:7', 0x1044)])])
        symbols = ' 0001:00000000 _test_function 00401000 f object.obj\n'
        result = refs.find_references(data, 'kernel32.DLL', 'ExitTest', symbols)
        self.assertEqual(result, [(0x401122, 'indirect call', 0x401040,
                                  (0x401000, '_test_function', 'f object.obj'))])
        self.assertEqual(refs.find_references(data, 'kernel32.dll', 'missing', symbols), [])
        with self.assertRaises(ValueError):
            refs.find_references(fixture(64), 'kernel32.dll', 'ExitTest')

    def test_flags_known_new_imports_case_insensitively_by_dll(self):
        result = [('KERNEL32.dll', ['ExitTest', 'GetSystemTimePreciseAsFileTime']),
                  ('USER32.dll', ['GetDpiForWindow']),
                  ('other.dll', ['GetDpiForWindow'])]
        self.assertEqual(pe.win7_findings(result), [('KERNEL32.dll', 'GetSystemTimePreciseAsFileTime'),
                                                    ('USER32.dll', 'GetDpiForWindow')])
        self.assertEqual(pe.win7_findings([('KERNEL32.dll', ['ExitTest'])]), [])

    def test_rejects_malformed_input(self):
        for data in (b'', b'MZ', fixture(32)[:0x210]):
            with self.assertRaises(ValueError):
                pe.imports(data)


if __name__ == '__main__':
    unittest.main()
