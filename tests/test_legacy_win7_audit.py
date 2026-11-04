"""Original synthetic PE images: no Windows SDK, binaries, pefile or network required."""
import hashlib
import json
from pathlib import Path
import random
import struct
import subprocess
import sys
import tempfile
import unittest

from tools.legacy_win7_audit import AuditError, audit_pe

ROOT = Path(__file__).resolve().parents[1]


def image(bits=32, normal=True, delay=True, legacy_delay=False):
    """Writer uses PE/COFF field offsets, independently of the reader's helpers."""
    data = bytearray(0x800)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 0x80)
    data[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", data, 0x84, 0x14C if bits == 32 else 0x8664,
                     1, 0, 0, 0, 224 if bits == 32 else 240, 0x122)
    struct.pack_into("<H", data, 0x98, 0x10B if bits == 32 else 0x20B)
    base = 0x400000 if bits == 32 else 0x140000000
    struct.pack_into("<I" if bits == 32 else "<Q", data, 0xB4 if bits == 32 else 0xB0, base)
    struct.pack_into("<II", data, 0xB8, 0x1000, 0x200)  # section/file alignment
    struct.pack_into("<HH", data, 0xC0, 6, 1)  # OS version
    struct.pack_into("<HH", data, 0xC8, 6, 1)  # subsystem version
    struct.pack_into("<II", data, 0xD0, 0x2000, 0x200)  # image/header size
    struct.pack_into("<H", data, 0xDC, 3)  # Windows CUI
    directory = 0xF8 if bits == 32 else 0x108
    struct.pack_into("<I", data, directory - 4, 16)
    section = 0x178 if bits == 32 else 0x188
    data[section:section + 8] = b".idata\0\0"
    struct.pack_into("<IIII", data, section + 8, 0x800, 0x1000, 0x600, 0x200)

    def string(offset, text):
        data[offset:offset + len(text) + 1] = text.encode("ascii") + b"\0"

    def thunks(offset, name, ordinal):
        struct.pack_into("<III" if bits == 32 else "<QQQ", data, offset,
                         name, (1 << (bits - 1)) | ordinal, 0)

    if normal:
        struct.pack_into("<II", data, directory + 8, 0x1000, 40)
        struct.pack_into("<IIIII", data, 0x200, 0x1140, 0, 0, 0x1100, 0x11C0)
        string(0x300, "KERNEL32.dll")
        string(0x382, "GetTickCount")
        thunks(0x340, 0x1180, 17)
        thunks(0x3C0, 0x1180, 17)
    if delay:
        struct.pack_into("<II", data, directory + 13 * 8, 0x1040, 64)
        adjustment = base if legacy_delay else 0
        struct.pack_into("<IIIIIIII", data, 0x240, 0 if legacy_delay else 1,
                         0x1200 + adjustment, 0, 0x12C0 + adjustment,
                         0x1240 + adjustment, 0, 0, 0)
        string(0x400, "ucrtbase.dll")
        string(0x482, "malloc")
        thunks(0x440, 0x1280 + adjustment, 23)
        # Unlike the INT, an unbound delay IAT holds executable helper addresses.
        struct.pack_into("<III" if bits == 32 else "<QQQ", data, 0x4C0,
                         base + 0x1500, base + 0x1510, 0)
    return data


class InventoryTests(unittest.TestCase):
    def test_normal_delay_names_ordinals_crt_and_identity(self):
        data = image()
        report = audit_pe(data)
        self.assertEqual(report["imports"], [
            {"kind": "normal", "dll": "KERNEL32.dll", "symbols": ["GetTickCount", "#17"]},
            {"kind": "delay", "dll": "ucrtbase.dll", "symbols": ["malloc", "#23"]},
        ])
        self.assertEqual(report["crt_dependencies"], [
            {"kind": "delay", "dll": "ucrtbase.dll", "family": "ucrt"}])
        self.assertEqual(report["sha256"], hashlib.sha256(data).hexdigest())
        self.assertEqual(report["size"], 2048)
        self.assertEqual(report["status"], "static_checks_passed")
        self.assertFalse(report["win7_qualified"])
        self.assertTrue(report["large_address_aware"])

    def test_legacy_delay_va_form(self):
        self.assertEqual(audit_pe(image(legacy_delay=True))["imports"], audit_pe(image())["imports"])

    def test_normal_iat_fallback_header_names_and_missing_delay_int(self):
        data = image(delay=False)
        struct.pack_into("<I", data, 0x200, 0)  # no OriginalFirstThunk
        data[0x50:0x5B] = b"USER32.dll\0"
        struct.pack_into("<I", data, 0x20C, 0x50)  # name in file-backed headers
        self.assertEqual(audit_pe(data)["imports"][0],
                         {"kind": "normal", "dll": "USER32.dll", "symbols": ["GetTickCount", "#17"]})
        data = image(normal=False)
        struct.pack_into("<I", data, 0x250, 0)  # delay IAT is NOT a name-table fallback
        with self.assertRaisesRegex(AuditError, "delay import name table"):
            audit_pe(data)

    def test_absent_directories_and_empty_inventory(self):
        data = image(normal=False, delay=False)
        self.assertEqual(audit_pe(data)["imports"], [])
        struct.pack_into("<I", data, 0xF4, 0)
        self.assertEqual(audit_pe(data)["crt_dependencies"], [])

    def test_pe32plus_uses_64bit_thunks_but_fails_x86_requirement(self):
        report = audit_pe(image(bits=64))
        self.assertEqual(report["imports"][1]["symbols"], ["malloc", "#23"])
        self.assertEqual(report["image_base"], "0x140000000")
        self.assertEqual(report["violations"], ["expected IMAGE_FILE_MACHINE_I386 + PE32"])
        for machine in (0x8664, 0x1C4):
            data = image()
            struct.pack_into("<H", data, 0x84, machine)
            self.assertEqual(audit_pe(data)["status"], "static_checks_failed")

    def test_header_versions_do_not_claim_os_support(self):
        for offset in (0xC0, 0xC8):
            data = image()
            struct.pack_into("<HH", data, offset, 6, 2)
            self.assertEqual(len(audit_pe(data)["violations"]), 1)

    def test_exact_deny_rules_cover_normal_delay_and_ordinal_imports(self):
        report = audit_pe(image(), deny_dll=["UCRTBASE.DLL"],
                          deny_import=[("kernel32.DLL", "GetTickCount"), ("ucrtbase.dll", "#23")])
        self.assertEqual(report["violations"], [
            "normal: denied import KERNEL32.dll!GetTickCount", "delay: denied DLL ucrtbase.dll",
            "delay: denied import ucrtbase.dll!#23"])
        self.assertEqual(audit_pe(image(), deny_import=[("kernel32.dll", "gettickcount")])["violations"], [])

    def test_explicit_post_win7_api_rule_rejects_both_import_kinds(self):
        for kind, offset in (("normal", 0x382), ("delay", 0x482)):
            data = image()
            if kind == "delay":
                data[0x400:0x40D] = b"KERNEL32.dll\0"
            data[offset:offset + 14] = b"WaitOnAddress\0"
            report = audit_pe(data, deny_import=[("kernel32.dll", "WaitOnAddress")])
            self.assertEqual(report["violations"], [f"{kind}: denied import KERNEL32.dll!WaitOnAddress"])

    def test_crt_inventory_is_not_proof_of_deployment_or_static_linking(self):
        for dll, family in [("MSVCRT.dll", "legacy-msvcrt"), ("vcruntime140.dll", "msvc-runtime"),
                            ("msvcp140d.dll", "msvc-runtime"), ("concrt140.dll", "msvc-runtime"),
                            ("api-ms-win-crt-runtime-l1-1-0.dll", "ucrt")]:
            with self.subTest(dll=dll):
                data = image(normal=False)
                data[0x400:0x400 + len(dll) + 1] = dll.encode() + b"\0"
                self.assertEqual(audit_pe(data)["crt_dependencies"][0]["family"], family)
                self.assertFalse(audit_pe(data)["win7_qualified"])


class MalformedTests(unittest.TestCase):
    def test_bad_signatures_headers_ranges_and_directories_are_rejected(self):
        patches = [
            (0, b"NZ"), (0x3C, struct.pack("<I", 0xFFFFFFF0)), (0x80, b"PE!!"),
            (0x98, b"\x07\x01"), (0x94, b"\x01\x00"), (0x86, b"\x61\x00"),
            (0xF4, struct.pack("<I", 17)), (0xD4, struct.pack("<I", 0x180)),
            (0x184, struct.pack("<I", 0x100)),  # section RVA overlaps headers
            (0x18C, struct.pack("<I", 0x700)),  # raw range extends past EOF
            (0x100, struct.pack("<II", 0, 40)), (0x100, struct.pack("<II", 0x1000, 0)),
            (0x104, struct.pack("<I", 20)),  # no terminating descriptor within size
            (0x20C, struct.pack("<I", 0x1700)),  # zero-fill, not file bytes
            (0x20C, struct.pack("<I", 0)), (0x210, struct.pack("<I", 0)),
            (0x340, struct.pack("<I", 0x17FF)),  # hint crosses mapped end
            (0x344, struct.pack("<I", 0x80010001)),  # reserved ordinal bits
            (0x240, struct.pack("<I", 2)),  # unknown delay attributes
            (0x300, b"\xFF"),
        ]
        for offset, patch in patches:
            with self.subTest(offset=hex(offset), patch=patch):
                data = image()
                data[offset:offset + len(patch)] = patch
                with self.assertRaises(AuditError):
                    audit_pe(data)

    def test_bound_iat_without_original_names_is_not_silently_decoded(self):
        for table_offset, stamp_offset, error in ((0x200, 0x204, "bound IAT"),
                                                  (0x250, 0x25C, "delay import name table")):
            data = image()
            struct.pack_into("<I", data, table_offset, 0)
            struct.pack_into("<I", data, stamp_offset, 1)
            with self.assertRaisesRegex(AuditError, error):
                audit_pe(data)

    def test_unterminated_string_thunks_and_delay_table(self):
        for offset, value in ((0x20C, 0x15F0), (0x200, 0x15F0)):
            data = image()
            data[0x7F0:] = b"X" * 16 if offset == 0x20C else struct.pack("<IIII", *([0x80000011] * 4))
            struct.pack_into("<I", data, offset, value)
            with self.assertRaises(AuditError):
                audit_pe(data)
        data = image()
        struct.pack_into("<I", data, 0x164, 32)  # delay directory excludes terminator
        with self.assertRaisesRegex(AuditError, "unterminated import directory"):
            audit_pe(data)

    def test_overlapping_sections_and_legacy_va_underflow(self):
        for second_rva in (0x1000, 0x2000):
            data = image()
            struct.pack_into("<H", data, 0x86, 2)
            data[0x1A0:0x1C8] = data[0x178:0x1A0]
            struct.pack_into("<I", data, 0x1AC, second_rva)
            with self.assertRaisesRegex(AuditError, "overlapping section"):
                audit_pe(data)
        data = image(legacy_delay=True)
        struct.pack_into("<I", data, 0x244, 0x1200)
        with self.assertRaisesRegex(AuditError, "RVA"):
            audit_pe(data)

    def test_symbol_budget_stops_a_large_otherwise_terminated_table(self):
        data = image(delay=False)
        end = 0x340 + 4 * 65536
        data.extend(b"\0" * (((end + 4 + 511) & ~511) - len(data)))
        data[0x340:end] = b"\x11\0\0\x80" * 65536
        raw_size = len(data) - 0x200
        struct.pack_into("<I", data, 0x180, raw_size)
        struct.pack_into("<I", data, 0x188, raw_size)
        struct.pack_into("<I", data, 0xD0, (0x1000 + raw_size + 4095) & ~4095)
        with self.assertRaisesRegex(AuditError, "symbol budget exceeded"):
            audit_pe(data)

    def test_truncation_and_structured_mutations_never_escape_as_parser_crashes(self):
        full = image()
        for end in range(0, len(full), 11):
            with self.assertRaises(AuditError):
                audit_pe(full[:end])
        rng = random.Random(719)
        for _ in range(150):
            data = image()
            offset = rng.choice([0x3C, 0xD4, 0xF4, 0x100, 0x164, 0x180, 0x184,
                                 0x188, 0x18C, 0x200, 0x20C, 0x240, 0x244, 0x250, 0x340, 0x440])
            struct.pack_into("<I", data, offset, rng.getrandbits(32))
            try:
                report = audit_pe(data)
                self.assertFalse(report["win7_qualified"])
            except AuditError:
                pass


class CliTests(unittest.TestCase):
    def run_cli(self, *arguments):
        return subprocess.run([sys.executable, str(ROOT / "tools/legacy_win7_audit.py"), *map(str, arguments)],
                              cwd=ROOT, text=True, capture_output=True, timeout=10)

    def test_exit_codes_and_json_for_multiple_files_including_invalid_input(self):
        with tempfile.TemporaryDirectory() as temp:
            good, wrong, bad = [Path(temp) / name for name in ("good.exe", "x64.dll", "bad.exe")]
            good.write_bytes(image())
            wrong.write_bytes(image(bits=64))
            bad.write_bytes(b"not a PE")
            for arguments, code in [([good], 0), ([good, wrong], 1),
                                    ([good, "--deny-import", "ucrtbase.dll!malloc"], 1),
                                    ([good, bad], 2), ([Path(temp) / "missing.dll"], 2)]:
                result = self.run_cli(*arguments)
                self.assertEqual(result.returncode, code, result.stderr + result.stdout)
                report = json.loads(result.stdout)
                self.assertEqual(report["schema_version"], 1)
                self.assertTrue(report["not_proven"])
                self.assertTrue(all(not f["win7_qualified"] for f in report["files"]))
                self.assertNotIn("Traceback", result.stderr)
            self.assertEqual(self.run_cli(good, "--deny-import", "invalid-rule").returncode, 2)


if __name__ == "__main__":
    unittest.main()
