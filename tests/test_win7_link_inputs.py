import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "win7_link_inputs", Path(__file__).parents[1] / "tools/win7_link_inputs.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class DirectSymbolTests(unittest.TestCase):
    def test_x86_import_and_plain_symbol_decorations(self):
        for name in (
            "GetSystemTimePreciseAsFileTime",
            "_GetSystemTimePreciseAsFileTime@4",
            "__imp__GetSystemTimePreciseAsFileTime@4",
        ):
            self.assertIsNotNone(module.DIRECT.search("foo.obj: U " + name))

    def test_linker_response_files_with_spaces_and_utf16(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            obj = root / "object with spaces.obj"
            archive = root / "archive.lib"
            obj.write_bytes(b"object")
            archive.write_bytes(b"archive")
            nested = root / "nested.rsp"
            nested.write_text(f'"{archive}"', encoding='utf-16')
            response = root / "linker.rsp"
            response.write_text(f'"{obj}" "@{nested}" /NOLOGO', encoding='utf-8')
            self.assertEqual(set(module.linker_inputs([f'@{response}'])), {obj, archive})

    def test_mangled_compatibility_wrapper_is_not_a_direct_import(self):
        self.assertIsNone(module.DIRECT.search(
            "foo.obj: T _RINvNtGetSystemTimePreciseAsFileTimeCompatibilityWrapper"
        ))


if __name__ == "__main__":
    unittest.main()
