# Re-link an already-built debug client with MSVC's library-resolution log.
# Diagnostic only: the result is not a supported Windows 7 release.
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'Visual Studio C++ tools were not found' }
$env:HORIZON_VCVARS = Join-Path $vs 'VC\Auxiliary\Build\vcvarsall.bat'
$env:HORIZON_LINK_TRACE = Join-Path $env:RUNNER_TEMP 'win7-link-trace.txt'
$env:HORIZON_INPUT_TRACE = Join-Path $env:RUNNER_TEMP 'win7-link-inputs.txt'
$env:HORIZON_INPUT_INSPECTOR = Join-Path $PSScriptRoot 'win7_link_inputs.py'
$sysroot = (& rustc +nightly --print sysroot).Trim()
$env:HORIZON_LLVM_NM = Join-Path $sysroot 'lib/rustlib/x86_64-pc-windows-msvc/bin/llvm-nm.exe'
$wrapper = Join-Path $env:RUNNER_TEMP 'horizon-trace-link.cmd'
@'
@echo off
call "%HORIZON_VCVARS%" x86 >nul 2>&1
if errorlevel 1 exit /b %errorlevel%
python "%HORIZON_INPUT_INSPECTOR%" > "%HORIZON_INPUT_TRACE%" 2>&1
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
    $file = Get-Item $env:HORIZON_LINK_TRACE
    Write-Host "::warning::Time symbol was not found in the linker trace ($($file.Length) bytes)"
    $sample = ((Get-Content $env:HORIZON_LINK_TRACE -TotalCount 15) -join "`n").Replace('%', '%25').Replace("`r", '%0D').Replace("`n", '%0A')
    Write-Host "::notice::Link trace starts: $sample"
}
if (Test-Path $env:HORIZON_INPUT_TRACE) {
    foreach ($line in (Get-Content $env:HORIZON_INPUT_TRACE | Select-Object -First 30)) {
        $escaped = $line.Replace('%', '%25').Replace("`r", '%0D').Replace("`n", '%0A')
        Write-Host "::notice::$escaped"
    }
}
if ($code -ne 0) {
    Get-Content $env:HORIZON_LINK_TRACE -Tail 60
    exit $code
}
