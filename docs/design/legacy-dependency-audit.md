# D01a — Legacy GL dependency/feature audit (working draft)

**Task-ID:** D01a  
**Variant:** D01a dependency inventory + ordered feature-cut requests (documentation only)  
**Scope:** this document; no manifest/lockfile, facade/contract, Game/GL wiring, workflow, or policy-script changes.  
**Depends-on:** C01/R01 for implementing cuts; inventory is based on the current integration baseline.  
**Integration base / baseline SHA:** `arena/a7e96628-horizon` / `fa703a67f635999f1f09a6ca669ee48e7bc12c97`.  
**Review:** coordinating Arena.ai integrator. Issues are disabled; this PR records the scoped work item.

> This is an audit, not a Cargo patch or a claim that a no-default-features build is legacy-only. The normal client and the current legacy renderer feature are not separated at the executable boundary yet.

## Initial evidence

The current checkout's root `crates/horizon-client/Cargo.toml` has no `legacy-gl` feature. `horizon-client-render` declares `legacy-gl = ["dep:horizon-legacy-render"]`, but the top-level client does not forward/enable it. Therefore the relevant comparison is the normal client graph versus a client with defaults disabled and the renderer feature enabled (`-p horizon-client --no-default-features --features horizon-client-render/legacy-gl`), not the bare `--no-default-features` graph as a presumed legacy profile.

The manifest edges already show that disabling defaults only removes the client `audio` and `controllers` feature activations. `wgpu`, `winit`, `horizon-hdr`, shader/compiler crates, game/server/client subsystems, `horizon-mods` index support, and live telemetry remain non-optional dependencies. On Windows, the client additionally has target-specific `open`, `wgpu/dx12`, and `winresource` edges; its non-macOS `wgpu/gles` edge also applies. These are candidate cut requests, not changes made here.

Exact resolved `cargo tree` output and source call-site mapping are being collected for the full audit. The current Arena shell has no `cargo`/`rustc` executable on `PATH` or on disk, so Cargo-resolved feature unification and build validation are not yet available locally. I will distinguish manifest/source evidence from any unresolved resolver/build evidence rather than presenting an inferred graph as Cargo output.

## Preliminary request ordering

1. After C01/R01, separate backend-neutral client/game logic from the wgpu renderer and its HDR/window startup path; retain the modern default and test it explicitly.
2. Make genuinely optional runtime subsystems explicit features only after each call site has a defined disabled behavior; do not cut server/shared gameplay dependencies merely because they are not GL-specific.
3. Keep launcher and build-only tools outside the game artifact graph; audit Windows target-specific imports and x86 viability separately from runtime feature cuts.

The detailed dependency paths, consumer sites, and per-request regression checks will replace this preliminary summary before requesting review.

## Tests / limits (initial)

- Read-only manifest/source inspection completed at baseline `fa703a67f635999f1f09a6ca669ee48e7bc12c97`.
- `cargo tree` and Rust compile/test commands are currently blocked because Cargo/Rust are absent in this environment.
- Not checked: exact Cargo resolver output, dependency build scripts and final PE imports, `i686` compilation/runtime, Win7 SP1, or Radeon X1650.
