"""CI01 behavior: trusted policy, ownership, spoofing and actual git diffs."""
import importlib.util
import json
import os
import sys
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("legacy_scope", ROOT / "tools/ci/legacy_scope.py")
scope = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scope)
POLICY = json.loads((ROOT / ".github/legacy-gl-policy.json").read_text())


def event(base=None, head="task-a", repo="lukintrees/horizon"):
    return {"repository": {"full_name": POLICY["repository"]}, "pull_request": {
        "number": 7, "base": {"ref": base or POLICY["integration_branch"], "sha": "1" * 40,
                                "repo": {"full_name": POLICY["repository"]}},
        "head": {"ref": head, "sha": "2" * 40, "repo": {"full_name": repo}},
        "body": "Task-ID: M01a\nVariant: A\nDepends-on: none\nScope: module-only\nContract-Version: 1",
    }}


class ScopeTests(unittest.TestCase):
    def test_task_and_exact_same_repo_integration_roles(self):
        self.assertEqual(scope.classify(event(), "pull_request", POLICY)["role"], "task")
        pr = event(base="main", head=POLICY["integration_branch"])
        self.assertEqual(scope.classify(pr, "pull_request", POLICY)["role"], "integration")
        self.assertEqual(scope.classify(pr, "pull_request_target", POLICY)["role"], "integration")

    def test_fork_and_labels_cannot_grant_integration(self):
        pr = event(base="main", head=POLICY["integration_branch"], repo="fork/horizon")
        pr["pull_request"]["labels"] = [{"name": "integrator"}]
        pr["pull_request"]["body"] = "Role: integrator"
        with self.assertRaises(scope.Rejected):
            scope.classify(pr, "pull_request", POLICY)
        fork_task = event(repo="fork/horizon")
        self.assertEqual(scope.classify(fork_task, "pull_request", POLICY)["role"], "task")

    def test_unrelated_or_malformed_events_rejected(self):
        for pr in [event(base="main"), event(base="other")]:
            with self.assertRaises(scope.Rejected):
                scope.classify(pr, "pull_request", POLICY)
        pr = event()
        pr["pull_request"]["head"]["sha"] = "--upload-pack=bad"
        with self.assertRaises(scope.Rejected):
            scope.classify(pr, "pull_request", POLICY)
        for ref in ["refs/heads/main", "refs/tags/arena/a7e96628-horizon"]:
            with self.assertRaises(scope.Rejected):
                scope.classify({"repository": {"full_name": POLICY["repository"]}, "ref": ref}, "push", POLICY)

    def test_protected_changes_including_new_manifests_are_rejected(self):
        for path in ["Cargo.toml", "Cargo.lock", "crates/new/Cargo.toml", "new/Cargo.lock",
                     ".github/workflows/fake.yml", ".github/legacy-gl-policy.json",
                      "tools/ci/legacy_scope.py", "AGENTS.md",
                      "crates/horizon-client-render/src/renderer_facade.rs",
                      "crates/horizon-client-render/src/legacy_gfx/asset_snapshot.rs",
                      "crates/horizon-client-render/src/legacy_gfx/resources.rs",
                      "crates/horizon-client-render/src/frame/skin.rs",
                      "crates/horizon-render-api/src/platform_contract.rs",
                     "crates/horizon-client/src/game.rs",
                     "crates/horizon-legacy-render/src/terrain.frag"]:
            with self.subTest(path=path), self.assertRaises(scope.Rejected):
                scope.inspect_paths(["docs/a.md", path], "task", POLICY)

    def test_docs_light_mode_does_not_bypass_protected_docs(self):
        self.assertEqual(scope.inspect_paths(["docs/design/audit.md"], "task", POLICY), "docs")
        with self.assertRaises(scope.Rejected):
            scope.inspect_paths(["docs/design/legacy-gl-integration.md"], "task", POLICY)
        self.assertEqual(scope.inspect_paths(["tests/test_thing.py"], "task", POLICY), "full")
        self.assertEqual(scope.inspect_paths(["Cargo.toml"], "integration", POLICY), "full")

    def test_metadata_is_required_but_does_not_authorize(self):
        body = event()["pull_request"]["body"]
        scope.metadata(body, POLICY)
        for field in ["Task-ID", "Variant", "Scope", "Depends-on", "Contract-Version"]:
            with self.subTest(field=field), self.assertRaises(scope.Rejected):
                scope.metadata("\n".join(line for line in body.splitlines() if not line.startswith(field + ":")), POLICY)
        with self.assertRaises(scope.Rejected):
            scope.metadata(body.replace("Contract-Version: 1", "Contract-Version: 999"), POLICY)
        with self.assertRaises(scope.Rejected):
            scope.inspect_paths(["Cargo.toml"], "task", POLICY)

    def test_late_scope_result_cannot_certify_an_updated_base(self):
        context = scope.classify(event(), "pull_request", POLICY)
        pr = event()["pull_request"]
        scope.verify_current_pr(context, pr)
        pr["base"]["sha"] = "9" * 40
        with self.assertRaises(scope.Rejected):
            scope.verify_current_pr(context, pr)

    def test_cli_uses_trusted_base_even_when_candidate_replaces_checker(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo = root / "repo"
            repo.mkdir()
            def git(*args):
                return subprocess.check_output(["git", "-C", str(repo), *args], stderr=subprocess.DEVNULL).decode().strip()
            git("init")
            git("config", "user.email", "test@example.invalid")
            git("config", "user.name", "test")
            (repo / "tools/ci").mkdir(parents=True)
            (repo / ".github").mkdir()
            checker = repo / "tools/ci/legacy_scope.py"
            checker.write_text((ROOT / "tools/ci/legacy_scope.py").read_text())
            (repo / ".github/legacy-gl-policy.json").write_text(json.dumps(POLICY))
            git("add", ".")
            git("commit", "-m", "base")
            base = git("rev-parse", "HEAD")
            checker.write_text("raise SystemExit(0)\n")
            (repo / ".github/legacy-gl-policy.json").write_text('{"protected": []}')
            (repo / "Cargo.toml").write_text("changed")
            git("add", ".")
            git("commit", "-m", "try to opt out")
            pr = event()
            pr["pull_request"]["base"]["sha"] = base
            pr["pull_request"]["head"]["sha"] = git("rev-parse", "HEAD")
            (root / "event.json").write_text(json.dumps(pr))
            # Same git-show extraction as the real read-only candidate workflow.
            (root / "trusted.py").write_text(git("show", base + ":tools/ci/legacy_scope.py"))
            (root / "policy.json").write_text(git("show", base + ":.github/legacy-gl-policy.json"))
            env = dict(os.environ)
            env.pop("GITHUB_OUTPUT", None)
            result = subprocess.run([sys.executable, str(root / "trusted.py"),
                "--policy", str(root / "policy.json"), "--repo-dir", str(repo),
                "--event-name", "pull_request", "--event-file", str(root / "event.json"),
                "--report", str(root / "report.json")], capture_output=True, env=env)
            self.assertEqual(result.returncode, 1)
            report = json.loads((root / "report.json").read_text())
            self.assertEqual(report["status"], "failed")
            self.assertIn("Cargo.toml", report["error"])

    def test_actual_diff_detects_rename_out_of_protected_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def git(*args):
                return subprocess.check_output(["git", "-C", str(root), *args], stderr=subprocess.DEVNULL).decode().strip()
            git("init")
            git("config", "user.email", "test@example.invalid")
            git("config", "user.name", "test")
            (root / "Cargo.toml").write_text("owned")
            git("add", ".")
            git("commit", "-m", "base")
            base = git("rev-parse", "HEAD")
            git("mv", "Cargo.toml", "looks-safe.txt")
            git("commit", "-m", "rename")
            paths = scope.changed_paths(root, base, git("rev-parse", "HEAD"))
            self.assertEqual(set(paths), {"Cargo.toml", "looks-safe.txt"})
            with self.assertRaises(scope.Rejected):
                scope.inspect_paths(paths, "task", POLICY)


if __name__ == "__main__":
    unittest.main()
