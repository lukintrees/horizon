"""The rustc-to-annotation reporter: the shapes cargo actually emits, and the escapes."""

import importlib.util
import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent


def load():
    spec = importlib.util.spec_from_file_location(
        "gl21_annotate_rustc", ROOT / "tools" / "gl21_annotate_rustc.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


reporter = load()


def compiler_message(level="error", message="mismatched types", file="src/mesh.rs", line=7,
                     column=9, code="E0308", rendered=None):
    return json.dumps(
        {
            "reason": "compiler-message",
            "message": {
                "level": level,
                "message": message,
                "code": {"code": code} if code else None,
                "spans": [
                    {
                        "file_name": file,
                        "line_start": line,
                        "column_start": column,
                        "is_primary": True,
                    }
                ],
                "rendered": rendered if rendered is not None else f"error[{code}]: {message}\n"
                f" --> {file}:{line}:{column}\n  |\n{line} |     let x = 1;\n  |         ^",
            },
        }
    )


class Annotations(unittest.TestCase):
    def test_a_compile_error_becomes_one_command_with_its_location(self):
        payloads, plain = reporter.parse([compiler_message()])
        lines = list(reporter.report(payloads, plain))
        self.assertEqual(len(lines), 1)
        line = lines[0]
        self.assertTrue(line.startswith("::error "), line)
        self.assertIn("file=src/mesh.rs", line)
        self.assertIn("line=7", line)
        self.assertIn("col=9", line)
        self.assertIn("E0308", line)
        self.assertIn("mismatched types", line)
        self.assertNotIn("\n", line, "a workflow command must be one line")

    def test_warnings_are_not_failures_and_are_not_reported_as_one(self):
        payloads, plain = reporter.parse([compiler_message(level="warning", code=None)])
        self.assertEqual(list(reporter.report(payloads, plain)), [])

    def test_a_stream_with_no_diagnostics_falls_back_to_its_output_tail(self):
        # A leftover libtest failure or a linker crash: the only description is the text.
        plain_lines = [f"line {i}" for i in range(100)] + ["panicked at 'assertion failed'"]
        lines = list(reporter.report([], plain_lines))
        self.assertEqual(len(lines), 1)
        self.assertTrue(lines[0].startswith("::error::"))
        self.assertIn("assertion failed", lines[0])
        self.assertIn("line 99", lines[0])
        self.assertNotIn("line 0", lines[0], "the tail, not the whole output")

    def test_a_noisy_stream_is_capped_and_says_so(self):
        payloads, plain = reporter.parse([compiler_message()] * 60)
        lines = list(reporter.report(payloads, plain, limit=5))
        self.assertEqual(lines[0], "::notice::5 diagnostics shown, the rest are in the job log")
        self.assertEqual(len(lines), 6, "the cap plus the notice")

    def test_non_json_lines_between_messages_do_not_break_the_parse(self):
        stream = [
            "   Compiling horizon-client-gl v0.1.0",
            compiler_message(),
            "running 12 tests",
            "test foo ... FAILED",
        ]
        payloads, plain = reporter.parse(stream)
        self.assertEqual(len(payloads), 1)
        self.assertEqual(len(plain), 3)
        self.assertEqual(len(list(reporter.report(payloads, plain))), 1)

    def test_the_escapes_are_the_ones_a_workflow_command_needs(self):
        self.assertEqual(reporter.escape("50% of\r\nlines"), "50%25 of%0D%0Alines")
        self.assertEqual(reporter.escape_property("a,b:c"), "a%2Cb%3Ac")
        # `%` before the others, or the escapes themselves get double-escaped.
        self.assertEqual(reporter.escape("%0D"), "%250D")

    def test_a_diagnostic_without_a_primary_span_still_reaches_the_reader(self):
        obj = json.loads(compiler_message())
        obj["message"]["spans"] = []
        obj["message"]["rendered"] = ""
        lines = list(reporter.report([obj], []))
        self.assertEqual(len(lines), 1)
        self.assertIn("mismatched types", lines[0])
        self.assertNotIn("file=", lines[0], "no location is better than a wrong one")

    def test_reading_files_by_path_is_the_same_as_reading_stdin(self):
        import tempfile

        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "cargo.json"
            path.write_text(compiler_message() + "\nnot json\n", encoding="utf-8")
            payloads, plain = reporter.parse(path.read_text().splitlines())
        self.assertEqual(len(payloads), 1)
        self.assertEqual(len(plain), 1)
        self.assertIn("src/mesh.rs", list(reporter.report(payloads, plain))[0])


if __name__ == "__main__":
    unittest.main()
