# Inactive workflow archive (legacy integration fork)

These YAML files are preserved for reference, not loaded by GitHub Actions.
The user requested a focused CI surface for `lukintrees/horizon`: no parity,
launcher, registry, labels/claims/triage, nightly, release or duplicate GL jobs.
No source scripts or checks are weakened to hide failures.

Active workflows live in `../workflows/`:
- `legacy-gl-branch.yml`: PR/aggregate/reusable validation, including the default
  modern client, native GL2, actual Frame GPU fixtures and Windows x86 contracts.
- `legacy-gl-scope.yml`: trusted metadata-only ownership/status control plane.

Already registered unwanted workflows were also disabled through the GitHub API;
archiving stops future branch pushes from registering new copies. Repository
workflow state is separate from Git and is not changed by this README. Older GL
workflows registered from other histories were disabled without reading their
contents. In-flight runs, if any, are a separate cancellation operation.

Restore a file only deliberately, with an explicit need and permissions review.
Paths between archived workflows may still refer to their original locations;
this is an archive, not an alternative executable CI directory. No upstream
repository settings or workflows outside this fork were changed.

The existing Python workflow-security tests read the archived definitions at
this location (and the runner-routing test checks both active and archived
files). Assertions are retained, not skipped after moving the YAML files.
The focused policy job continues running these regression tests.
