# O01a — Win7 x86 build and PE/import audit

Status: draft investigation, 2026-11-04. **Not Win7/X1650 qualification.**

- Task-ID: **O01a**; Variant: **A — independent, Python-stdlib PE audit**.
- Integration draft: [#4](https://github.com/lukintrees/horizon/pull/4).
- Integration base: `arena/a7e96628-horizon`.
- Fetched baseline SHA: `453966cb6253adf848deb58165eba354693e7acc`.
- Owner/reviewer: lukintrees's Arena.ai task agent / coordinating Arena.ai integrator.
- Depends-on: existing native probe at the baseline; no new facade contract.
- Scope: this document, `tools/legacy_win7_audit.py`,
  `tests/test_legacy_win7_audit.py` only.
- Central contracts, Game/GL wiring, manifests/lockfiles, workflows and policy scripts: unchanged.

## Investigation / implementation plan

1. Distinguish Rust target recognition from downloadable target std and a linked PE.
   Investigate a pinned >=1.90 game toolchain and, if needed, pinned nightly + rust-src/build-std.
2. Keep standalone native-probe evidence separate from full-game dependency/build evidence.
3. Implement a bounded PE reader with architecture, normal/delay imports and CRT inventory;
   exercise it with original positive and negative synthetic fixtures.
4. Document dynamic loading, allocator, CPU ISA and deployment risks beyond static imports.
5. Publish exact commands, observed results, a recommended build path and remaining gates.

## Initial environment and open gates

The Linux sandbox initially has Python 3 but no Rust/rustup/MSVC/Windows SDK on PATH.
Outbound downloads are restricted to GitHub, npm and PyPI hosts. Official Rust distribution
availability must not be inferred from mirror packages or a target name alone.
No Windows 7 SP1 x86 / Radeon X1650 hardware is available. Native WGL execution,
CRT deployment, actual CPU/ISA support and the full playable Game scenario remain **open**.
No successful build, PE audit or hardware run is claimed by this initial draft.
