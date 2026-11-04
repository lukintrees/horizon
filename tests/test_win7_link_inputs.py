import importlib.util
from pathlib import Path
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

    def test_mangled_compatibility_wrapper_is_not_a_direct_import(self):
        self.assertIsNone(module.DIRECT.search(
            "foo.obj: T _RINvNtGetSystemTimePreciseAsFileTimeCompatibilityWrapper"
        ))


if __name__ == "__main__":
    unittest.main()
