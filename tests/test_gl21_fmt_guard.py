"""tools/gl21_fmt_guard.py: the shapes rustfmt rewrites, caught without a toolchain.

Every example below is a shape `cargo fmt --check` actually asked for on this branch (the long chain
came back as a five-line rewrite, the merged derives as one attribute, the `use` brace as a reorder),
so the guard is pinned to what CI did rather than to what rustfmt's documentation says in general.
"""

import unittest

from tools import gl21_fmt_guard


def found(text):
    return [message for _line, message in gl21_fmt_guard.check_text(text)]


class ChainTests(unittest.TestCase):
    def test_a_chain_past_sixty_columns_is_one_method_a_line(self):
        # The line CI turned into five: 69 columns from `.iter()` to the end.
        line = (
            '        let width = problems.iter().any(|p| p.contains("a_uv") '
            '&& p.contains("vec2") && p.contains("z"));\n'
        )
        self.assertTrue(any("method chain" in message for message in found(line)))
        # A shorter one: rustfmt leaves it on one line.
        short = '        let ok = problems.iter().any(|p| p.contains("a"));\n'
        self.assertEqual(found(short), [])

    def test_a_short_chain_and_a_chain_free_line_pass(self):
        self.assertEqual(found("        let screen = self.gl >= (3, 3) && modern;\n"), [])
        self.assertEqual(found("        let n = v.len() as u32;\n"), [])

    def test_the_chain_is_measured_from_its_first_method_not_the_line(self):
        # A long receiver with a two-method chain stays: 30 columns from `.iter()`.
        long_receiver = "        let all = self.some_really_long_field_name.iter().count();\n"
        self.assertEqual(found(long_receiver), [])


class DeriveTests(unittest.TestCase):
    def test_two_derives_in_a_row_are_merged(self):
        text = "#[derive(Clone, PartialEq)]\n#[derive(bytemuck::Pod)]\nstruct V;\n"
        self.assertTrue(any("derive" in message for message in found(text)))

    def test_one_derive_with_many_names_passes(self):
        text = "#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]\nstruct V;\n"
        self.assertEqual(found(text), [])


class ImportTests(unittest.TestCase):
    def test_a_use_brace_is_sorted_with_upper_case_first(self):
        self.assertTrue(found("pub use context::{glsl_name, Capabilities};\n"))
        self.assertEqual(found("pub use context::{Capabilities, glsl_name};\n"), [])

    def test_self_comes_first(self):
        self.assertEqual(found("use a::{self, Capabilities};\n"), [])

    def test_a_use_inside_a_function_is_not_reordered(self):
        # `reorder_imports` is not applied to a body's `use`, which is why a test's
        # `use super::..; use std::..;` passes CI as written.
        text = "    fn t() {\n        use super::Thing;\n        use std::mem::offset_of;\n    }\n"
        self.assertEqual(found(text), [])


class ModuleTests(unittest.TestCase):
    def test_top_level_modules_are_sorted_ignoring_pub(self):
        self.assertTrue(found("mod profile;\npub mod arena;\n"))
        self.assertEqual(found("pub mod arena;\nmod profile;\npub mod vertex;\n"), [])


class CommentTests(unittest.TestCase):
    def test_a_long_comment_is_not_a_formatting_complaint(self):
        long_doc = "//! " + "word " * 40 + "\n"
        line_comment = "    // " + "x" * 150 + "\n"
        self.assertEqual(found(long_doc + line_comment), [])

    def test_code_past_a_hundred_columns_is(self):
        self.assertTrue(found("    let x = " + "a" * 95 + ";\n"))


class ExitTests(unittest.TestCase):
    def test_main_reports_one_for_a_file_it_would_rewrite(self):
        import tempfile
        from pathlib import Path

        with tempfile.TemporaryDirectory() as folder:
            bad = Path(folder) / "bad.rs"
            bad.write_text("pub use a::{b, C};\n", encoding="utf-8")
            self.assertEqual(gl21_fmt_guard.main([str(bad)]), 1)
            good = Path(folder) / "good.rs"
            good.write_text("pub use a::{C, b};\n", encoding="utf-8")
            self.assertEqual(gl21_fmt_guard.main([str(good)]), 0)


if __name__ == "__main__":
    unittest.main()
