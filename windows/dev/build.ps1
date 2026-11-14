<#
.SYNOPSIS
  Builds the engine (ikari-engine-host.exe) and the TIP (ikari_tip.dll) in release mode. Without the LM unless -Lm.
.PARAMETER HideConsole
  Build the engine with the "hide-console" feature (no console window; failures go to logs\start-failure.log).
  Leave it off while testing: you want the console.
.PARAMETER Lm
  Build the engine with the "lm" feature (zenz via llama.cpp, CPU). Needs cmake. The LM stays OFF at run time until
  you turn it on (settings.tsv in the data directory: engine.lm=true) and the model is in <DataDir>\models\.
.NOTES
  Needs Rust (stable-x86_64-pc-windows-msvc) and Visual Studio Build Tools (C++ workload). cmake is needed only with
  -Lm.
#>
param([switch]$HideConsole, [switch]$Lm)

. "$PSScriptRoot\common.ps1"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'cargo not found. Install Rust with rustup (stable-x86_64-pc-windows-msvc).'
}

$features = @()
if ($HideConsole) { $features += 'ikari-engine-host/hide-console' }
if ($Lm) {
    if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
        throw 'cmake not found. The LM build (llama.cpp) needs cmake: install it (https://cmake.org/download/) and reopen the shell.'
    }
    $features += 'ikari-engine-host/lm'
}

$cargoArgs = @('build', '--release', '-p', 'ikari-engine-host', '-p', 'ikari-tip')
if ($features.Count -gt 0) { $cargoArgs += @('--features', ($features -join ',')) }

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
