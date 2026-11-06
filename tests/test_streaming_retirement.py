"""Run the actual dependency-free Rust heap retirement module, not a translation.

No Game, terrain generation, GPU, or prepared-resource acceptance is claimed.
"""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(shutil.which("rustc"), "rustc required")
class StreamingRetirement(unittest.TestCase):
    def test_actual_rust_module(self):
        source = ROOT / "crates/horizon-client-render/src/streaming/retirement.rs"
        with tempfile.TemporaryDirectory(prefix="horizon-retirement-") as tmp:
            root = Path(tmp)
            wrapper, exe = root / "tests.rs", root / "tests"
            wrapper.write_text(f'#[path = "{source.as_posix()}"] mod retirement;\n')
            subprocess.run(["rustc", "--edition=2024", "--test", str(wrapper), "-o", str(exe)],
                           check=True, capture_output=True, text=True)
            result = subprocess.run([str(exe)], check=True, capture_output=True, text=True)
            self.assertIn("3 passed; 0 failed", result.stdout)
