# Re-link an already-built debug client with MSVC's library-resolution log.
# Diagnostic only: the result is not a supported Windows 7 release.
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'Visual Studio C++ tools were not found' }
$env:HORIZON_VCVARS = Join-Path $vs 'VC\Auxiliary\Build\vcvarsall.bat'
$env:HORIZON_LINK_TRACE = Join-Path $env:RUNNER_TEMP 'win7-link-trace.txt'
$wrapper = Join-Path $env:RUNNER_TEMP 'horizon-trace-link.cmd'
@'
@echo off
call "%HORIZON_VCVARS%" x86 >nul 2>&1
if errorlevel 1 exit /b %errorlevel%
link.exe /VERBOSE:LIB %* > "%HORIZON_LINK_TRACE%" 2>&1
exit /b %errorlevel%
'@ | Set-Content -Encoding ASCII $wrapper
& cargo +nightly rustc -Z build-std=std,panic_abort --locked -p horizon-client --no-default-features --features legacy-scene --bin horizon-client --target i686-win7-windows-msvc -- -C "linker=$wrapper"
$code = $LASTEXITCODE
if (-not (Test-Path $env:HORIZON_LINK_TRACE)) { throw 'Linker did not write its trace' }
# Print context around direct users, without filling CI's log with the whole trace.
$matches = Select-String -Path $env:HORIZON_LINK_TRACE -Pattern 'GetSystemTimePreciseAsFileTime' -Context 3,4
if ($matches) {
    foreach ($match in $matches | Select-Object -First 12) {
        $message = $match.ToString().Replace('%', '%25').Replace("`r", '%0D').Replace("`n", '%0A')
        Write-Host "::notice::$message"
    }
} else {
    Write-Host '::warning::Time symbol was not found in the linker trace'
}
if ($code -ne 0) {
    Get-Content $env:HORIZON_LINK_TRACE -Tail 60
    exit $code
}
