import contextlib
import io
import json
import struct
import tempfile
import unittest
from pathlib import Path

from tools import legacy_win7_audit as audit

PE_OFFSET = 0x80
OPTIONAL_OFFSET = PE_OFFSET + 24
RAW_OFFSET = 0x200
SECTION_RVA = 0x1000


def raw(rva):
    return RAW_OFFSET + rva - SECTION_RVA


def directory_offset(pe64, index):
    return OPTIONAL_OFFSET + (112 if pe64 else 96) + index * 8


def set_directory(image, pe64, index, rva=None, size=None):
    offset = directory_offset(pe64, index)
    if rva is not None:
        struct.pack_into("<I", image, offset, rva)
    if size is not None:
        struct.pack_into("<I", image, offset + 4, size)


def delay_descriptor_rva(normal_count):
    end = SECTION_RVA + (normal_count + 1) * 20
    return (end + 15) & ~15


def make_pe(machine=0x014C, normal=(), delay=(), delay_va=False, original_thunks=True):
    """Build a small file-backed PE with independently laid-out import tables.

    Each module is (DLL name, symbols); a symbol is a name or an ordinal integer.
    No bytes are derived from the parser under test.
    """
    pe64 = machine in (0x8664, 0xAA64)
    width = 8 if pe64 else 4
    ordinal_flag = 1 << (63 if pe64 else 31)
    optional_size = 240 if pe64 else 224
    image_base = 0x140000000 if pe64 else 0x400000
    image = bytearray(RAW_OFFSET + 0x1000)
    image[:2] = b"MZ"
    struct.pack_into("<I", image, 0x3C, PE_OFFSET)
    struct.pack_into("<I", image, PE_OFFSET, 0x00004550)
    characteristics = 0x0022 if pe64 else 0x0122  # executable, large-address-aware
    struct.pack_into("<HHIIIHH", image, PE_OFFSET + 4, machine, 1, 0, 0, 0,
                     optional_size, characteristics)
    struct.pack_into("<H", image, OPTIONAL_OFFSET, 0x20B if pe64 else 0x10B)
    struct.pack_into("<Q" if pe64 else "<I", image,
                     OPTIONAL_OFFSET + (24 if pe64 else 28), image_base)
    struct.pack_into("<HH", image, OPTIONAL_OFFSET + 40, 6, 1)
    struct.pack_into("<HH", image, OPTIONAL_OFFSET + 48, 6, 1)
    struct.pack_into("<I", image, OPTIONAL_OFFSET + 56, 0x2000)
    struct.pack_into("<I", image, OPTIONAL_OFFSET + 60, RAW_OFFSET)
    struct.pack_into("<H", image, OPTIONAL_OFFSET + 68, 2)
    struct.pack_into("<I", image, OPTIONAL_OFFSET + (108 if pe64 else 92), 16)
    section = OPTIONAL_OFFSET + optional_size
    image[section:section + 6] = b".rdata"
    struct.pack_into("<IIIIIIHHI", image, section + 8, 0x1000, SECTION_RVA,
                     0x1000, RAW_OFFSET, 0, 0, 0, 0, 0x40000040)

    cursor = [0x1100]

    def allocate(size, alignment=4):
        result = (cursor[0] + alignment - 1) & ~(alignment - 1)
        cursor[0] = result + size
        if cursor[0] > SECTION_RVA + 0x1000:
            raise ValueError("fixture section overflow")
        return result

    def put_thunks(symbols):
        lookup = allocate((len(symbols) + 1) * width, width)
        iat = allocate((len(symbols) + 1) * width, width)
        for index, symbol in enumerate(symbols):
            if isinstance(symbol, int):
                value = ordinal_flag | symbol
            else:
                name = allocate(2 + len(symbol.encode("ascii")) + 1, 2)
                struct.pack_into("<H", image, raw(name), index + 1)
                image[raw(name) + 2:raw(name) + 2 + len(symbol)] = symbol.encode("ascii")
                value = name
            struct.pack_into("<Q" if pe64 else "<I", image, raw(lookup) + index * width, value)
            struct.pack_into("<Q" if pe64 else "<I", image, raw(iat) + index * width, value)
        return lookup, iat

    for index, (dll, symbols) in enumerate(normal):
        lookup, iat = put_thunks(symbols)
        name = allocate(len(dll) + 1)
        image[raw(name):raw(name) + len(dll)] = dll.encode("ascii")
        struct.pack_into("<IIIII", image, raw(SECTION_RVA + index * 20),
                         lookup if original_thunks else 0, 0, 0, name, iat)
    import_size = (len(normal) + 1) * 20
    set_directory(image, pe64, 1, SECTION_RVA, import_size)

    delay_rva = delay_descriptor_rva(len(normal))
    for index, (dll, symbols) in enumerate(delay):
        lookup, iat = put_thunks(symbols)
        name = allocate(len(dll) + 1)
        image[raw(name):raw(name) + len(dll)] = dll.encode("ascii")

        def address(value):
            return value + image_base if delay_va and value else value

        struct.pack_into("<IIIIIIII", image, raw(delay_rva + index * 32),
                         0 if delay_va else 1, address(name), 0, address(iat),
                         address(lookup), 0, 0, 0)
    if delay:
        set_directory(image, pe64, 13, delay_rva, (len(delay) + 1) * 32)
    return bytes(image)


class AuditTests(unittest.TestCase):
    def test_i686_normal_delay_ordinal_and_crt_imports(self):
        image = make_pe(
            normal=(("KERNEL32.dll", ["CreateFileW", 12]),
                    ("VCRUNTIME140.dll", ["_CxxThrowException"])),
            delay=(("api-ms-win-crt-heap-l1-1-0.dll", ["malloc"]),),
        )
        report = audit.audit_bytes(image)
        self.assertTrue(report["architecture_ok"])
        self.assertEqual((report["architecture"], report["format"]), ("i686", "PE32"))
        self.assertEqual((report["os_version"], report["subsystem_version"]), ("6.1", "6.1"))
        self.assertTrue(report["large_address_aware"])
        self.assertEqual(report["imports"][0]["symbols"],
                         [{"name": "CreateFileW", "hint": 1}, {"ordinal": 12}])
        self.assertEqual([item["kind"] for item in report["imports"]],
                         ["normal", "normal", "delay"])
        self.assertEqual(report["imports"][2]["symbols"], [{"name": "malloc", "hint": 1}])
        self.assertEqual(report["crt_dependencies"], [
            {"dll": "VCRUNTIME140.dll", "kind": "normal", "family": "msvc-runtime"},
            {"dll": "api-ms-win-crt-heap-l1-1-0.dll", "kind": "delay",
             "family": "universal-crt"},
        ])

    def test_absolute_delay_addresses_and_first_thunk_fallback(self):
        report = audit.audit_bytes(make_pe(
            normal=(("KERNEL32.dll", ["GetLastError"]),),
            delay=(("USER32.dll", ["MessageBoxW"]),),
            delay_va=True, original_thunks=False,
        ))
        self.assertEqual([item["symbols"][0]["name"] for item in report["imports"]],
                         ["GetLastError", "MessageBoxW"])

    def test_pe32_plus_uses_eight_byte_thunks(self):
        report = audit.audit_bytes(make_pe(
            machine=0x8664, normal=(("KERNEL32.dll", ["ExitProcess", 42]),)),
            expected_arch="x86_64")
        self.assertEqual((report["architecture"], report["format"]), ("x86_64", "PE32+"))
        self.assertEqual(report["image_base"], 0x140000000)
        self.assertEqual(report["imports"][0]["symbols"],
                         [{"name": "ExitProcess", "hint": 1}, {"ordinal": 42}])

    def test_absent_import_directories_are_valid(self):
        image = bytearray(make_pe())
        set_directory(image, False, 1, rva=0, size=0)
        report = audit.audit_bytes(bytes(image))
        self.assertEqual(report["imports"], [])
        self.assertEqual(report["crt_dependencies"], [])

    def test_wrong_architecture_fails_cli_and_is_machine_readable(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory, "wrong.exe")
            output = Path(directory, "report.json")
            binary.write_bytes(make_pe(machine=0x8664))
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                status = audit.main([str(binary), "--json", str(output)])
            result = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(status, 1)
        self.assertFalse(result["ok"])
        self.assertFalse(result["files"][0]["architecture_ok"])
        self.assertEqual(result["errors"], [])

    def test_malformed_input_fails_closed(self):
        invalid_name = bytearray(make_pe(normal=(("KERNEL32.dll", ["Beep"]),)))
        struct.pack_into("<I", invalid_name, raw(SECTION_RVA) + 12, 0xDEADBEEF)
        unterminated = bytearray(make_pe(normal=(("KERNEL32.dll", ["Beep"]),)))
        set_directory(unterminated, False, 1, size=20)
        bad_delay = bytearray(make_pe(delay=(("USER32.dll", ["MessageBoxW"]),)))
        struct.pack_into("<I", bad_delay, raw(delay_descriptor_rva(0)), 2)
        unterminated_thunk = bytearray(make_pe(normal=(("KERNEL32.dll", ["Beep"]),)))
        original_lookup = struct.unpack_from("<I", unterminated_thunk, raw(SECTION_RVA))[0]
        first_value = struct.unpack_from("<I", unterminated_thunk, raw(original_lookup))[0]
        final_thunk = SECTION_RVA + 0x1000 - 4
        struct.pack_into("<I", unterminated_thunk, raw(SECTION_RVA), final_thunk)
        struct.pack_into("<I", unterminated_thunk, raw(final_thunk), first_value)
        cases = {
            "not PE": b"not a PE",
            "truncated PE": make_pe()[:100],
            "unmapped DLL name": bytes(invalid_name),
            "unterminated descriptors": bytes(unterminated),
            "unterminated thunks": bytes(unterminated_thunk),
            "unknown delay attributes": bytes(bad_delay),
        }
        for label, image in cases.items():
            with self.subTest(label=label):
                with self.assertRaises(audit.PeError):
                    audit.audit_bytes(image)

    def test_json_stdout_contains_only_json(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory, "probe.exe")
            binary.write_bytes(make_pe())
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(io.StringIO()):
                status = audit.main([str(binary), "--json", "-"])
            result = json.loads(stdout.getvalue())
        self.assertEqual(status, 0)
        self.assertTrue(result["ok"])
        self.assertEqual(result["files"][0]["architecture"], "i686")

    def test_malformed_file_has_exit_two_and_json_error(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory, "broken.exe")
            output = Path(directory, "report.json")
            truncated = bytearray(b"MZ" + bytes(62))
            struct.pack_into("<I", truncated, 0x3C, PE_OFFSET)
            binary.write_bytes(truncated)
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                status = audit.main([str(binary), "--json", str(output)])
            result = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(status, 2)
        self.assertFalse(result["ok"])
        self.assertEqual(result["files"], [])
        self.assertIn("truncated PE signature", result["errors"][0]["error"])


if __name__ == "__main__":
    unittest.main()
