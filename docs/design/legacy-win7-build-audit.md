# Win7 x86 build audit (O01a)

Status: **draft; measurements and the audit helper are being added**.
Integration baseline: `453966cb6253adf848deb58165eba354693e7acc`.

Scope: reproducible Rust 1.90 build routes for the existing native GL probe and the
full game, plus an isolated PE32/import/CRT audit. This document will distinguish
compile/link evidence from execution on Windows 7 SP1 x86 and Radeon X1650.
No manifest, workflow, shared build policy, renderer, or Game wiring is changed.

The hardware gate is open: no result in this task is a Win7/X1650 qualification
unless it was actually produced on that target. Commands, observed results,
negative fixtures, static-audit limits, and remaining runtime checks follow in
the completed audit.
