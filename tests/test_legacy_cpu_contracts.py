"""R01 acceptance: the shared CPU contracts name no wgpu type.

`frame/`, `render_contract`, `resource_contract` and `platform_contract` are the
types both backends read. They may hold borrowed pixels, ids, stamps and plain
data, but a GPU handle (`wgpu::Device`, `Queue`, `Buffer`, `Texture`, ...) in
one of them would make the legacy backend depend on a wgpu object graph. The
crate still links wgpu (D01), so this checks the shared declarations, not the
dependency graph. Comments may name wgpu; code may not.
"""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FRAME = ROOT / "crates/horizon-client-render/src/frame"
COMMON = [
    "crates/horizon-render-api/src/screenshot.rs",
    "crates/horizon-render-api/src/render_contract.rs",
    "crates/horizon-render-api/src/resource_contract.rs",
    "crates/horizon-render-api/src/platform_contract.rs",
]


def common_files():
    frame = sorted(path.relative_to(ROOT).as_posix() for path in FRAME.rglob("*.rs"))
    return COMMON + frame


def code_lines(text):
    """`(line number, line)` with `//` comments removed."""
    for number, line in enumerate(text.splitlines(), 1):
        code = line.split("//", 1)[0]
        if code.strip():
            yield number, code


class CpuContractTests(unittest.TestCase):
    def test_the_frame_contract_exists_where_this_test_scans(self):
        files = common_files()
        self.assertIn("crates/horizon-client-render/src/frame/mod.rs", files)
        for relative in files:
            self.assertTrue((ROOT / relative).is_file(), relative)

    def test_shared_cpu_contracts_name_no_wgpu_type(self):
        offenders = []
        for relative in common_files():
            text = (ROOT / relative).read_text(encoding="utf-8")
            for number, code in code_lines(text):
                if re.search(r"\bwgpu\b", code):
                    offenders.append(f"{relative}:{number}: {code.strip()}")
        self.assertEqual(offenders, [], "\n".join(offenders))

    def test_the_scan_ignores_comments_but_not_declarations(self):
        text = "// wgpu::Device in prose\nlet device: wgpu::Device;\n"
        self.assertEqual(
            [code for _, code in code_lines(text)], ["let device: wgpu::Device;"]
        )
