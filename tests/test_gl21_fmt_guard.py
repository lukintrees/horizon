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
        # The line CI turned into five: a chain of 77 columns inside an `assert!`.
        line = (
            '        assert!(problems.iter().any(|p| p.contains("a_uv") '
            '&& p.contains("vec2") && p.contains("z")));\n'
        )
        self.assertTrue(any("method chain" in message for message in found(line)))
        # A shorter one: rustfmt leaves it on one line.
        short = '        assert!(problems.iter().any(|p| p.contains("a")));\n'
        self.assertEqual(found(short), [])

    def test_a_chain_that_is_a_statement_is_left_to_max_width(self):
        # `gfx.rs` has chains of 75 columns as statements and CI accepts them: judging those would
        # send someone to reformat code rustfmt keeps.
        statement = "        let x = some.long_call_here(" + "1, " * 12 + "1).collect();\n"
        self.assertEqual(found(statement), [])


class TypedBindingTests(unittest.TestCase):
    def test_a_chain_after_a_written_out_type_is_one_method_a_line(self):
        # CI broke this one, a 98-column line whose chain is 62 columns: past `chain_width`, so
        # rustfmt gives every method its own line.
        line = (
            "        let runs: Vec<(u32, u32)> = draws.runs.iter()"
            ".map(|run| (run.begin, run.count)).collect();\n"
        )
        self.assertTrue(any("chain" in message for message in found(line)))

    def test_a_typed_let_that_fits_is_left_alone(self):
        line = (
            "        let vertices: Vec<GlHudVertex> ="
            " batch.vertices.iter().map(GlHudVertex::of).collect();\n"
        )
        self.assertEqual(found(line), [])

    def test_a_binding_without_a_type_is_left_to_max_width(self):
        line = "        let x = some.long_call_here(1, 1, 1).collect();\n"
        self.assertEqual(found(line), [])


class StructLiteralTests(unittest.TestCase):
    def test_a_struct_literal_with_long_fields_goes_one_a_line(self):
        # CI turned `Viewport { width: 320, height: 240 }` into four lines.
        line = "        let viewport = Viewport { width: 320, height: 240 };\n"
        self.assertTrue(any("struct literal" in message for message in found(line)))

    def test_a_short_one_stays(self):
        line = "            solid: SectionRuns::new(0, 2),\n"
        self.assertEqual(found(line), [])


class MacroWidthTests(unittest.TestCase):
    def test_a_macro_call_with_too_many_argument_columns_goes_a_line_each(self):
        # CI turned this one-line equality into four lines.
        line = "        assert_eq!(missing_attributes.count(), TERRAIN.len() + TERRAIN_UNIFORMS.len());\n"
        self.assertTrue(any("assert_eq!" in message for message in found(line)))

    def test_a_derive_or_a_plain_call_is_left_alone(self):
        # 64 columns in a derive and 65 in a call: rustfmt keeps both as written.
        derive = "#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]\n"
        self.assertEqual(found(derive), [])
        call = "pub fn plan(sections: &[SectionDraw], sky: Option<SectionRuns>, present: bool) -> Plan {\n"
        self.assertEqual(found(call), [])


class LiteralTests(unittest.TestCase):
    def test_a_long_string_over_several_lines_is_not_reformatted(self):
        text = (
            'const M: &str = "a long message that runs \\\n'
            "     on and on and on past the hundred-column mark the formatter uses \\\n"
            '     and ends here";\n'
        )
        self.assertEqual(found(text), [])

    def test_a_macro_bracket_reaches_past_max_width_untouched(self):
        line = "        let a = wgpu::vertex_attr_array![" + "0=>Float32x3," * 10 + "];\n"
        self.assertEqual(found(line), [])

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
