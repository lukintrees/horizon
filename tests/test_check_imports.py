import contextlib
import io
from pathlib import Path
import struct
import tempfile
import unittest

from tools import check_imports


def elf(interpreter, libraries, runpath=None):
    data = bytearray(4096)
    data[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into("<Q", data, 32, 64)
    struct.pack_into("<HH", data, 54, 56, 3 if interpreter else 2)
    struct.pack_into("<IIQQQQQQ", data, 64, 1, 4, 0, 0x400000, 0, len(data), len(data), 4096)
    strings = bytearray(b"\0")
    entries = [(5, 0x400400)]
    for tag, value in [(1, lib) for lib in libraries] + ([(29, runpath)] if runpath else []):
        entries.append((tag, len(strings)))
        strings.extend(value.encode() + b"\0")
    entries.append((0, 0))
    struct.pack_into("<IIQQQQQQ", data, 120, 2, 4, 512, 0x400200, 0,
                     len(entries) * 16, len(entries) * 16, 8)
    if interpreter:
        raw = interpreter.encode() + b"\0"
        data[256:256 + len(raw)] = raw
        struct.pack_into("<IIQQQQQQ", data, 176, 3, 4, 256, 0x400100, 0, len(raw), len(raw), 1)
    for i, entry in enumerate(entries):
        struct.pack_into("<qQ", data, 512 + i * 16, *entry)
    data[1024:1024 + len(strings)] = strings
    return data


class LinuxImportsTests(unittest.TestCase):
    def check_binary(self, interpreter, libraries, *, runpath=None, **options):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "program"
            path.write_bytes(elf(interpreter, libraries, runpath))
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                return check_imports.check(path, **options)

    def test_musl_desktop(self):
        self.assertTrue(self.check_binary("/lib/ld-musl-x86_64.so.1",
                        ["libc.musl-x86_64.so.1", "libgcc_s.so.1", "libasound.so.2", "libudev.so.1"],
                        musl=True))

    def test_musl_requires_its_dynamic_loader(self):
        for loader in (None, "/lib64/ld-linux-x86-64.so.2", "/nix/store/musl/lib/ld-musl-x86_64.so.1"):
            with self.subTest(loader=loader):
                self.assertFalse(self.check_binary(loader, [], musl=True))

    def test_musl_rejects_glibc_and_unexpected_libraries(self):
        for library in ("libc.so.6", "libpthread.so.0", "libm.so.6", "libcustom.so"):
            with self.subTest(library=library):
                self.assertFalse(self.check_binary("/lib/ld-musl-x86_64.so.1", [library], musl=True))

    def test_glibc_policy_still_rejects_musl(self):
        self.assertTrue(self.check_binary("/lib64/ld-linux-x86-64.so.2", ["libc.so.6", "libasound.so.2"]))
        self.assertFalse(self.check_binary("/lib/ld-musl-x86_64.so.1", ["libc.musl-x86_64.so.1"]))

    def test_server_and_runpath_checks_apply_to_both_abis(self):
        for musl, loader in ((False, "/lib64/ld-linux-x86-64.so.2"), (True, "/lib/ld-musl-x86_64.so.1")):
            with self.subTest(musl=musl):
                self.assertTrue(self.check_binary(loader, ["libgcc_s.so.1"], musl=musl, server=True))
                for lib in ("libasound.so.2", "libudev.so.1"):
                    self.assertFalse(self.check_binary(loader, [lib], musl=musl, server=True))
                self.assertFalse(self.check_binary(loader, [], musl=musl, runpath="/nix/store/libs"))


def pe(dlls):
    """A minimal PE32 image whose import directory names `dlls`, one descriptor each.

    Each `dll` may be a plain name, or `(name, [procedure, ...])` so the import-by-name thunks
    exist too and `--windows7`'s entry-point check has something to read.
    """
    data = bytearray(0x1000)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 0x40)
    data[0x40:0x44] = b"PE\0\0"
    # COFF: one section; optional header 96 (PE32 fixed part) + 16 data directories of 8 bytes.
    struct.pack_into("<H", data, 0x44 + 2, 1)
    struct.pack_into("<H", data, 0x44 + 16, 96 + 128)
    optional = 0x40 + 24
    struct.pack_into("<H", data, optional, 0x10B)
    directories = optional + 96
    # One section: RVA 0x1000 is file offset 0x200, and it covers the rest of the file.
    struct.pack_into("<8xIIII", data, optional + 96 + 128, 0xE00, 0x1000, 0xE00, 0x200)

    def at(rva):
        return rva - 0x1000 + 0x200

    entries = [dll if isinstance(dll, tuple) else (dll, []) for dll in dlls]
    thunks_at = 0x1800
    names_at = 0x1A00
    for i, (dll, procedures) in enumerate(entries):
        raw = dll.encode() + b"\0"
        name_rva = 0x1200 + i * 0x40
        data[at(name_rva):at(name_rva) + len(raw)] = raw
        if procedures:
            # Name RVA at +12, first thunk at +0 of each 20-byte IMAGE_IMPORT_DESCRIPTOR.
            struct.pack_into("<I", data, at(0x1100) + 20 * i, thunks_at)
            for j, procedure in enumerate(procedures):
                # IMAGE_IMPORT_BY_NAME is a 2-byte hint then the name; the thunk points at it.
                text = struct.pack("<H", 0) + procedure.encode() + b"\0"
                data[at(names_at):at(names_at) + len(text)] = text
                struct.pack_into("<I", data, at(thunks_at), names_at)
                thunks_at += 4
                names_at += len(text) + (-len(text) % 2)
            thunks_at += 4  # the NUL thunk that ends the array
        struct.pack_into("<I", data, at(0x1100) + 20 * i + 12, name_rva)
    # The descriptors are zero-terminated; those bytes are already zero.
    struct.pack_into("<I", data, directories + 8, 0x1100)
    return data


class WindowsImportsTests(unittest.TestCase):
    def check_dlls(self, dlls, **options):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "program.exe"
            path.write_bytes(pe(dlls))
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                return check_imports.check(path, **options)

    def output_for(self, dlls, **options):
        """The check's verdict and everything it printed, stdout and stderr together."""
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "program.exe"
            path.write_bytes(pe(dlls))
            output = io.StringIO()
            with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
                return check_imports.check(path, **options), output.getvalue()

    def test_default_profile_allows_api_sets(self):
        self.assertTrue(self.check_dlls(["kernel32.dll", "api-ms-win-core-winrt-l1-1-0.dll"]))

    def test_windows7_refuses_winrt_and_pathcch(self):
        for dll in ("api-ms-win-core-winrt-l1-1-0.dll", "api-ms-win-core-winrt-error-l1-1-0.dll",
                    "api-ms-win-core-path-l1-1-0.dll", "xinput1_4.dll"):
            with self.subTest(dll=dll):
                self.assertFalse(self.check_dlls(["kernel32.dll", dll], windows7=True))

    def test_windows7_reports_ucrt_as_installable_and_accepts_a_clean_image(self):
        # The UCRT needs KB2999226, which is a prerequisite to install, not a reason to refuse the
        # image: it is reported but does not fail the check.
        ok, output = self.output_for(["kernel32.dll", "api-ms-win-crt-heap-l1-1-0.dll"], windows7=True)
        self.assertTrue(ok)
        self.assertIn("KB2999226", output)
        self.assertTrue(self.check_dlls(["kernel32.dll", "opengl32.dll", "xinput9_1_0.dll"], windows7=True))

    def test_windows7_refuses_a_procedure_that_arrived_after_it(self):
        # The DLL is there on Windows 7; the procedure is not, and the loader refuses the image
        # with "The procedure entry point ... could not be located in KERNEL32.dll".
        ok, output = self.output_for(
            [("kernel32.dll", ["GetCurrentThreadStackLimits", "VirtualQuery"]),
             ("kernel32.dll", ["SetThreadStackGuarantee"])],
            windows7=True)
        self.assertIn("GetCurrentThreadStackLimits", output)
        self.assertFalse(ok)

    def test_windows7_accepts_the_same_image_once_the_procedure_is_gone(self):
        ok, output = self.output_for(
            [("kernel32.dll", ["VirtualQuery", "SetThreadStackGuarantee", "GetProcAddress"])],
            windows7=True)
        self.assertTrue(ok)
        self.assertNotIn("does not export", output)
