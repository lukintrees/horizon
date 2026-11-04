# Experimental cross-build on a supported Windows development machine.
# This builds the existing wgpu client, NOT a client with legacy GL integrated.
# Rust's Win7 target is tier 3; neither a successful build nor the incomplete PE
# import audit proves that the binary will run on Windows 7.
param(
    [switch]$Release,
    [switch]$AllowKnownIncompatibleImports
)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

# Win7 std has no prebuilt rustup component. Use rust-src for the exact nightly.
if ($env:RUSTFLAGS -notmatch 'target-feature=\+crt-static(?:\s|,|$)') {
    $env:RUSTFLAGS = (($env:RUSTFLAGS, '-C target-feature=+crt-static' | Where-Object { $_ }) -join ' ')
}
if (-not $Release -and -not $env:CARGO_PROFILE_DEV_DEBUG) { $env:CARGO_PROFILE_DEV_DEBUG = '0' }
$cargoArgs = @('+nightly', 'build', '-Z', 'build-std=std,panic_abort', '--locked',
    '-p', 'horizon-client', '--no-default-features', '--features', 'legacy-scene', '--bin', 'horizon-client',
    '--target', 'i686-win7-windows-msvc')
$buildProfile = 'debug'
if ($Release) { $cargoArgs += '--release'; $buildProfile = 'release' }
& cargo @cargoArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$binary = "target/i686-win7-windows-msvc/$buildProfile/horizon-client.exe"
$audit = @('tools/pe_imports.py', $binary)
if (-not $AllowKnownIncompatibleImports) { $audit += '--check-win7' }
& python @audit
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Host "Built $binary. This is not a compatibility or game-play verification."
