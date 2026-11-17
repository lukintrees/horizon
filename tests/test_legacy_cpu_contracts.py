"""The shared CPU contracts name no wgpu type.

`horizon-render-api` and the client-render bridges are the plain data both
backends read: vertices, ids, stamps, colors, a lightmap. A GPU handle
(`wgpu::Device`, `Queue`, `Buffer`, `Texture`, ...) in one of them would make the
legacy backend depend on a wgpu object graph. The client-render crate still links
wgpu, so this checks the shared declarations, not the dependency graph: comments
may name wgpu, code may not.
"""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BRIDGES = [
    "crates/horizon-client-render/src/lightmap.rs",
    "crates/horizon-client-render/src/render_bridge.rs",
    "crates/horizon-client-render/src/entity_bridge.rs",
    "crates/horizon-client-render/src/gui_bridge.rs",
    "crates/horizon-client-render/src/paged_hud.rs",
]
COMMON = [
    "crates/horizon-render-api/src/streaming.rs",
    "crates/horizon-render-api/src/screenshot.rs",
    "crates/horizon-render-api/src/stroke.rs",
    "crates/horizon-render-api/src/render_contract.rs",
    "crates/horizon-render-api/src/resource_contract.rs",
    "crates/horizon-render-api/src/platform_contract.rs",
]


def common_files():
    return COMMON + BRIDGES


def code_lines(text):
    """`(line number, line)` with `//` comments removed."""
    for number, line in enumerate(text.splitlines(), 1):
        code = line.split("//", 1)[0]
        if code.strip():
            yield number, code


class CpuContractTests(unittest.TestCase):
    def test_the_scanned_contracts_exist(self):
        for relative in common_files():
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


if __name__ == "__main__":
    unittest.main()
