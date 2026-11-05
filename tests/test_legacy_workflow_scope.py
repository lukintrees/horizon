"""Machine-consumed fork CI allowlist, not a claim about parity/Win7 evidence."""
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
ACTIVE = {"legacy-gl-branch.yml", "legacy-gl-scope.yml"}


def active_workflows(directory):
    return {p.name for p in directory.iterdir() if p.is_file() and p.suffix in (".yml", ".yaml")}


class WorkflowScopeTests(unittest.TestCase):
    def test_only_focused_validation_and_trusted_scope_are_active(self):
        self.assertEqual(active_workflows(ROOT / ".github/workflows"), ACTIVE)

    def test_archive_is_not_an_actions_workflow_directory(self):
        archive = ROOT / ".github/workflow-archive"
        for name in ["ci.yml", "launcher.yml", "parity.yml", "pr-labels.yml", "release.yml"]:
            self.assertTrue((archive / name).is_file(), name)
        self.assertFalse(active_workflows(ROOT / ".github/workflows") & active_workflows(archive))

    def test_new_yaml_files_cannot_hide_behind_an_alternate_extension(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)
            (path / "launcher.yaml").write_text("name: unwanted")
            (path / "notes.md").write_text("not a workflow")
            self.assertEqual(active_workflows(path), {"launcher.yaml"})
