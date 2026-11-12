<#
.SYNOPSIS
  Builds the engine (ikari-engine-host.exe) and the TIP (ikari_tip.dll) in release mode, without the LM.
.PARAMETER HideConsole
  Build the engine with the "hide-console" feature (no console window; failures go to logs\start-failure.log).
  Leave it off while testing: you want the console.
.NOTES
  Needs Rust (stable-x86_64-pc-windows-msvc) and Visual Studio Build Tools (C++ workload). cmake is NOT needed
  (no LM). Never run on Windows when written: syntax only.
#>
param([switch]$HideConsole)

. "$PSScriptRoot\common.ps1"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'cargo not found. Install Rust with rustup (stable-x86_64-pc-windows-msvc).'
}

$cargoArgs = @('build', '--release', '-p', 'ikari-engine-host', '-p', 'ikari-tip')
if ($HideConsole) { $cargoArgs += @('--features', 'ikari-engine-host/hide-console') }

Write-Step "cargo $($cargoArgs -join ' ')   (in $RepoRoot)"
Push-Location $RepoRoot
try {
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit code $LASTEXITCODE)" }
}
finally {
    Pop-Location
}

foreach ($f in @($EngineExe, $TipDllBuilt)) {
    if (-not (Test-Path -LiteralPath $f)) { throw "build finished but $f does not exist" }
    $item = Get-Item -LiteralPath $f
    Write-Note ("{0}  {1:N0} bytes  {2}" -f $item.FullName, $item.Length, $item.LastWriteTime)
}
Write-Step 'Build done. Next: install-data.ps1, then register.ps1 (as administrator), then start-engine.ps1'
