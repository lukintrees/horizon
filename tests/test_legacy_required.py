import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("legacy_required", Path(__file__).resolve().parents[1] / "tools/ci/legacy_required.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class RequiredTests(unittest.TestCase):
    def matrix(self):
        return {name: {"result": "success"} for name in (
            "scope", "policy-tests", "terrain-integration", "gl2-runtime", "windows-x86-abi", "modern")}

    def test_full_success_and_each_failure_missing_cancellation_or_skip(self):
        gate.validate(self.matrix(), "full")
        for name in self.matrix():
            for result in ("failure", "cancelled", "skipped", None):
                needs = self.matrix()
                if result is None:
                    del needs[name]
                else:
                    needs[name]["result"] = result
                with self.subTest(name=name, result=result), self.assertRaises(ValueError):
                    gate.validate(needs, "full")

    def test_docs_requires_scope_and_policy_not_skipped_test_jobs(self):
        needs = self.matrix()
        for name in set(needs) - {"scope", "policy-tests"}:
            needs[name]["result"] = "skipped"
        gate.validate(needs, "docs")
        with self.assertRaises(ValueError):
            gate.validate(needs, "full")
        with self.assertRaises(ValueError):
            gate.validate(needs, "")

    def test_current_merge_candidate_not_merely_same_head(self):
        event = {"pull_request": {"number": 4}}
        report = {"base": "b", "head": "h"}
        pr = {"number": 4, "base": {"sha": "b"}, "head": {"sha": "h"}, "merge_commit_sha": "m"}
        gate.current_candidate(event, report, "m", pr)
        for field in ("base", "head", "merge_commit_sha"):
            changed = dict(pr)
            changed[field] = {"sha": "other"} if field != "merge_commit_sha" else "other"
            with self.subTest(field=field), self.assertRaises(ValueError):
                gate.current_candidate(event, report, "m", changed)
