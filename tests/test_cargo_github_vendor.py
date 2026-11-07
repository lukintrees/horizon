"""Offline tests of actual vendor installation and rejection, not network stubs."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "cargo_github_vendor", Path(__file__).resolve().parents[1] / "tools/cargo_github_vendor.py")
vendor = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(vendor)


class GithubVendorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source"
        self.source.mkdir()
        self.dest = self.root / "vendor"
        self.package = {"name": "fixture", "version": "1.2.3", "checksum": "a" * 64}
        files = {"Cargo.toml": '[package]\nname="fixture"\nversion="1.2.3"\nlicense="MIT"\n',
                 "src/lib.rs": "pub fn fixture() -> u32 { 42 }\n", "LICENSE": "test fixture"}
        for name, text in files.items():
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        self.checksum = {"package": "a" * 64, "files": {
            n: hashlib.sha256((self.source / n).read_bytes()).hexdigest() for n in files}}
        self.write_checksum()

    def write_checksum(self):
        (self.source / ".cargo-checksum.json").write_text(json.dumps(self.checksum))

    def test_installs_only_verified_listed_files_and_is_repeatable(self):
        (self.source / "build.rs").write_text('panic!("unlisted");')
        vendor.install(self.source, self.dest, self.package)
        vendor.install(self.source, self.dest, self.package)
        installed = self.dest / "fixture-1.2.3"
        self.assertEqual((installed / "src/lib.rs").read_text(), "pub fn fixture() -> u32 { 42 }\n")
        self.assertFalse((installed / "build.rs").exists())
        self.assertEqual({p.name for p in self.dest.iterdir()}, {"fixture-1.2.3"})

    def test_modified_source_or_missing_binary_rejected_without_partial_install(self):
        for modification in ("tamper", "remove"):
            with self.subTest(modification=modification):
                source = self.source / "src/lib.rs"
                if modification == "tamper":
                    source.write_text("changed")
                else:
                    source.unlink()
                with self.assertRaises(ValueError):
                    vendor.install(self.source, self.dest, self.package)
                self.assertFalse(self.dest.exists())

    def test_package_checksum_and_file_manifest_are_required(self):
        for checksum in ({"package": "b" * 64, "files": self.checksum["files"]},
                         {"files": self.checksum["files"]},
                         {"package": "a" * 64, "files": {}}):
            with self.subTest(checksum=checksum):
                (self.source / ".cargo-checksum.json").write_text(json.dumps(checksum))
                with self.assertRaises(ValueError):
                    vendor.install(self.source, self.dest, self.package)
                self.assertFalse(self.dest.exists())

    def test_traversal_absolute_and_symlink_paths_rejected(self):
        outside = self.root / "outside"
        outside.write_text("outside")
        (self.source / "link").symlink_to(outside)
        for name in ("../outside", str(outside), "link", "C:/outside", "..\\outside"):
            with self.subTest(name=name):
                self.checksum["files"][name] = hashlib.sha256(outside.read_bytes()).hexdigest()
                self.write_checksum()
                with self.assertRaises(ValueError):
                    vendor.install(self.source, self.dest, self.package)
                del self.checksum["files"][name]
        self.assertEqual(outside.read_text(), "outside")

    def test_existing_corrupt_vendor_is_not_silently_accepted(self):
        vendor.install(self.source, self.dest, self.package)
        (self.dest / "fixture-1.2.3/src/lib.rs").write_text("corrupt")
        with self.assertRaises(ValueError):
            vendor.install(self.source, self.dest, self.package)

    def test_injected_unlisted_build_script_in_existing_vendor_is_rejected(self):
        vendor.install(self.source, self.dest, self.package)
        (self.dest / "fixture-1.2.3/build.rs").write_text("fn main() {}")
        with self.assertRaises(ValueError):
            vendor.install(self.source, self.dest, self.package)

    def test_existing_vendor_cannot_rewrite_its_own_file_checksums(self):
        vendor.install(self.source, self.dest, self.package)
        installed = self.dest / "fixture-1.2.3"
        (installed / "src/lib.rs").write_text("changed")
        self.checksum["files"]["src/lib.rs"] = hashlib.sha256(b"changed").hexdigest()
        (installed / ".cargo-checksum.json").write_text(json.dumps(self.checksum))
        with self.assertRaises(ValueError):
            vendor.install(self.source, self.dest, self.package)

    def test_source_pins_are_full_commit_hashes(self):
        # This is a consumed configuration contract, not prose/source matching.
        for repository, commit, prefix in vendor.SOURCES:
            self.assertRegex(commit, r"^[0-9a-f]{40}$")
            self.assertEqual(len(repository.split("/")), 2)
            self.assertNotIn("..", Path(prefix).parts)


if __name__ == "__main__":
    unittest.main()
