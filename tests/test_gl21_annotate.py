"""tools/gl21_annotate.py: compiler output turned into check annotations.

The GL 2.1 work is built on a machine that can read the checks API but not the action logs, so this
tool is the only way a `cargo fmt` or `clippy` failure gets back to the code that caused it. A
pattern that stops matching (rustfmt's diff format, clippy's short format) would turn every red
build into "failed without a recognised diagnostic", which is why the formats are pinned here.
"""

import unittest

from tools import gl21_annotate


FMT_DIFF = """Diff in crates/horizon-client-render/src/gl21/atlas.rs at line 12:
     pub fn new(layers: u32) -> Atlas {
-        let x = 1;
+        let x = 2;
Diff in crates/horizon-client-render/src/gl21/draws.rs at line 40:
-        foo(1,2)
+        foo(1, 2)
+        bar(3,4)
"""

CLIPPY_SHORT = """    Checking horizon-client-render v0.1.0
crates/horizon-client-render/src/gl21/vertex.rs:52:9: error[clippy::cast_possible_truncation]: casting `u16` to `u8` may truncate the value
crates/horizon-client-render/src/gl21/atlas.rs:44:5: warning: unused variable: `pad`
error: could not compile `horizon-client-render` (lib) due to 1 previous error
"""


class FormatTests(unittest.TestCase):
    def test_each_hunk_is_one_annotation_at_its_first_line(self):
        found = gl21_annotate.annotations("fmt", FMT_DIFF)
        self.assertEqual([f for f, _line, _message in found], [
            "crates/horizon-client-render/src/gl21/atlas.rs",
            "crates/horizon-client-render/src/gl21/draws.rs",
        ])
        self.assertEqual([line for _f, line, _m in found], [12, 40])
        # The message shows what rustfmt would change, and both files' changes stay their own.
        self.assertIn("let x = 1", found[0][2])
        self.assertIn("bar(3,4)", found[1][2])
        self.assertNotIn("bar", found[0][2])

    def test_a_clean_diff_is_nothing_to_annotate(self):
        self.assertEqual(gl21_annotate.annotations("fmt", ""), [])
        self.assertEqual(gl21_annotate.annotations("fmt", "Diff in a.rs at line 3:\n"), [])


class LinterTests(unittest.TestCase):
    def test_short_diagnostics_carry_file_line_and_kind(self):
        found = gl21_annotate.annotations("clippy", CLIPPY_SHORT)
        self.assertEqual(
            [(f, line) for f, line, _m in found],
            [
                ("crates/horizon-client-render/src/gl21/vertex.rs", 52),
                ("crates/horizon-client-render/src/gl21/atlas.rs", 44),
            ],
        )
        self.assertTrue(found[0][2].startswith("error: casting"))
        self.assertTrue(found[1][2].startswith("warning: unused"))
        # The final "could not compile" line has no file of its own and is not one.
        self.assertEqual(len(found), 2)

    def test_an_unknown_mode_is_refused(self):
        with self.assertRaises(SystemExit):
            gl21_annotate.annotations("cargo", "")


class OutputTests(unittest.TestCase):
    def test_workflow_commands_escape_what_github_reads(self):
        # `%`, newlines and carriage returns are what break a workflow command.
        self.assertEqual(gl21_annotate._escape("a%b\r\nc"), "a%25b%0D%0Ac")

    def test_a_failure_with_no_diagnostic_is_still_an_annotation(self):
        # The step only runs when the tool failed, so silence must not be green.
        import tempfile
        from pathlib import Path

        with tempfile.TemporaryDirectory() as folder:
            empty = Path(folder) / "empty.txt"
            empty.write_text("error: something the pattern does not know\n", encoding="utf-8")
            self.assertEqual(gl21_annotate.main(["x", "fmt", str(empty)]), 1)


if __name__ == "__main__":
    unittest.main()
