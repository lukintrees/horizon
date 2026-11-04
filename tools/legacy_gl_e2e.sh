#!/bin/sh
# Standalone developer test; needs Rust and Mesa's libOSMesa.so.8.
# No player data or Cargo dependency downloads are used.
set -eu
cd "$(dirname "$0")/.."
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
rustc --edition 2024 --crate-name horizon_legacy_gl --crate-type lib \
    crates/horizon-legacy-gl/src/lib.rs -o "$tmp/libhorizon_legacy_gl.rlib"
rustc --edition 2024 --test crates/horizon-legacy-gl/src/lib.rs -o "$tmp/gl_tests"
"$tmp/gl_tests"
rustc --edition 2024 --test tools/legacy_gl_e2e.rs \
    --extern "horizon_legacy_gl=$tmp/libhorizon_legacy_gl.rlib" -o "$tmp/cpu_tests"
"$tmp/cpu_tests"
rustc --edition 2024 tools/legacy_gl_e2e.rs \
    --extern "horizon_legacy_gl=$tmp/libhorizon_legacy_gl.rlib" -o "$tmp/gl_e2e"
LIBGL_ALWAYS_SOFTWARE=1 MESA_GL_VERSION_OVERRIDE=2.1 MESA_GLSL_VERSION_OVERRIDE=120 \
    "$tmp/gl_e2e"
