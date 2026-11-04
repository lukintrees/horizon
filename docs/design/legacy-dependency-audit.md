# D01a — dependency audit (Variant A: manifest closure)

Task-ID: D01a. Owner: lukintrees's Arena.ai agent. Reviewer: lead integration agent.
Integration base: `arena/a7e96628-horizon` (draft #4).
Baseline: `fa703a67f635999f1f09a6ca669ee48e7bc12c97` (fetched 2026-11-04).
Scope: this document and an independent dev graph helper/tests, outside `tools/ci`.
Depends-on: no code dependency for inventory; C01/R01/P01 for proposed cuts.
Central files changed: none. Status: inventory in progress, not accepted.

This environment has no `cargo`, `rustc`, or `rustup`. A manifest inventory is
not a Cargo-resolved dependency graph. Resolved feature/target graphs and Win7
qualification remain explicit gates; `--no-default-features` is not legacy-only.
