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
$env:HORIZON_MAP_TRACE = Join-Path $env:RUNNER_TEMP 'win7-client.map'
$env:HORIZON_INPUT_INSPECTOR = Join-Path $PSScriptRoot 'win7_link_inputs.py'
$sysroot = (& rustc +nightly --print sysroot).Trim()
$env:HORIZON_LLVM_NM = Join-Path $sysroot 'lib/rustlib/x86_64-pc-windows-msvc/bin/llvm-nm.exe'
$wrapper = Join-Path $env:RUNNER_TEMP 'horizon-trace-link.cmd'
$binary = 'target/i686-win7-windows-msvc/debug/horizon-client.exe'
$originalHash = (Get-FileHash $binary -Algorithm SHA256).Hash
$originalImport = @'
import sys
from pathlib import Path
from tools.pe_imports import imports
entries = imports(Path(sys.argv[1]).read_bytes())
print(int(any(name == 'GetSystemTimePreciseAsFileTime' for _, names in entries for name in names)))
'@
$beforeImport = (& python -c $originalImport $binary | Select-Object -Last 1)
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect original client imports' }
@'
@echo off
call "%HORIZON_VCVARS%" x86 >nul 2>&1
if errorlevel 1 exit /b %errorlevel%
python "%HORIZON_INPUT_INSPECTOR%" %* > "%HORIZON_INPUT_TRACE%" 2>&1
link.exe /VERBOSE:LIB /MAP:"%HORIZON_MAP_TRACE%" %* > "%HORIZON_LINK_TRACE%" 2>&1
exit /b %errorlevel%
'@ | Set-Content -Encoding ASCII $wrapper
& cargo +nightly rustc -Z build-std=std,panic_abort --locked -p horizon-client --no-default-features --features legacy-live --bin horizon-client --target i686-win7-windows-msvc -- -C "linker=$wrapper"
$code = $LASTEXITCODE
if (-not (Test-Path $env:HORIZON_LINK_TRACE)) { throw 'Linker did not write its trace' }
if ($code -eq 0) {
    $afterImport = (& python -c $originalImport $binary | Select-Object -Last 1)
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect relinked client imports' }
    $newHash = (Get-FileHash $binary -Algorithm SHA256).Hash
    Write-Host "::notice::Relinked client: time import before=$beforeImport after=$afterImport; hash changed=$($newHash -ne $originalHash)"
    if (Test-Path $env:HORIZON_MAP_TRACE) {
        $references = & python tools/pe_import_references.py $binary $env:HORIZON_MAP_TRACE 2>&1
        if ($LASTEXITCODE -eq 0) {
            $summary = (($references | Select-Object -First 25) -join "`n").Replace('%', '%25').Replace("`r", '%0D').Replace("`n", '%0A')
            Write-Host "::notice::IAT candidate references: $summary"
        } else {
            Write-Host "::warning::Could not inspect time import references: $references"
        }
    } else {
        Write-Host "::warning::Linker map was not generated"
    }
}
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
    $lines = @(Get-Content $env:HORIZON_INPUT_TRACE)
    $focus = @($lines | Where-Object { $_ -match '^(Input search roots|Linker inputs|Examined|DIRECT:|llvm-nm:|Traceback|Error)' } | Select-Object -First 20)
    $summary = (($focus -join "`n") + "`n" + (($lines | Select-Object -Last 8) -join "`n"))
    $escaped = $summary.Replace('%', '%25').Replace("`r", '%0D').Replace("`n", '%0A')
    Write-Host "::notice::Win7 link inputs: $escaped"
}
if ($code -ne 0) {
    Get-Content $env:HORIZON_LINK_TRACE -Tail 60
    exit $code
}
