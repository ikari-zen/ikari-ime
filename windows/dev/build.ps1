<#
.SYNOPSIS
  Builds the engine (ikari-engine-host.exe) and the TIP (ikari_tip.dll) in release mode. Without the LM unless -Lm.
.PARAMETER HideConsole
  Build the engine with the "hide-console" feature (no console window; failures go to logs\start-failure.log).
  Leave it off while testing: you want the console.
.PARAMETER Lm
  Build the engine with the "lm" feature (zenz via llama.cpp, CPU). Needs cmake. The LM stays OFF at run time until
  you turn it on (settings.tsv in the data directory: engine.lm=true) and the model is in <DataDir>\models\.
.PARAMETER LmVulkan
  Build the engine with the "lm-vulkan" feature (zenz via llama.cpp, on the GPU with Vulkan; falls back to the CPU on a
  machine without a GPU or without vulkan-1.dll). Cannot be combined with -Lm. Needs cmake, Ninja and the Vulkan SDK
  (VULKAN_SDK, glslc). The backend DLLs (ggml-vulkan.dll and the CPU variants) are copied next to the engine exe, in
  <release>\backends. Turn it off at run time with "engine.lm_gpu<TAB>off" in settings.tsv (ADR 0059).
.NOTES
  Needs Rust (stable-x86_64-pc-windows-msvc) and Visual Studio Build Tools (C++ workload). cmake is needed only with
  -Lm and -LmVulkan.
  -LmVulkan: if the build stops in rc.exe with RC2136, the build folder path is too long: set CARGO_TARGET_DIR to a short
  path (for example C:\t) and run it again.
#>
param([switch]$HideConsole, [switch]$Lm, [switch]$LmVulkan)

. "$PSScriptRoot\common.ps1"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'cargo not found. Install Rust with rustup (stable-x86_64-pc-windows-msvc).'
}

if ($Lm -and $LmVulkan) { throw '-Lm and -LmVulkan cannot be combined (-LmVulkan already includes the LM).' }

$features = @()
if ($HideConsole) { $features += 'ikari-engine-host/hide-console' }
if ($Lm) {
    if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
        throw 'cmake not found. The LM build (llama.cpp) needs cmake: install it (https://cmake.org/download/) and reopen the shell.'
    }
    $features += 'ikari-engine-host/lm'
}
if ($LmVulkan) {
    if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
        throw 'cmake not found. The LM build (llama.cpp) needs cmake: install it (https://cmake.org/download/) and reopen the shell.'
    }
    if (-not $env:VULKAN_SDK -or -not (Test-Path -LiteralPath (Join-Path $env:VULKAN_SDK 'Bin\glslc.exe'))) {
        throw 'The Vulkan SDK was not found (VULKAN_SDK is not set, or Bin\glslc.exe is missing). Install it from https://vulkan.lunarg.com/sdk/home#windows and reopen the shell.'
    }
    # The Visual Studio generator failed to install vulkan-shaders-gen (ggml-vulkan); Ninja works (found in CI).
    if (-not (Get-Command ninja -ErrorAction SilentlyContinue)) {
        throw 'ninja not found. The Vulkan build needs Ninja (winget install Ninja-build.Ninja) and a Developer PowerShell (cl.exe on PATH).'
    }
    if (-not $env:CMAKE_GENERATOR) { $env:CMAKE_GENERATOR = 'Ninja' }
    $features += 'ikari-engine-host/lm-vulkan'
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

if ($LmVulkan) {
    # llama-cpp-sys-2 builds ggml as DLLs. The backends (ggml-vulkan.dll, ggml-cpu-*.dll) are in <OUT_DIR>\backends and are
    # NOT copied to target. The engine loads them from "backends" next to its exe (lm/src/llama.rs). ggml*.dll and
    # llama.dll are in <OUT_DIR>\bin and go next to the exe.
    $sysOut = Get-ChildItem -LiteralPath (Join-Path $ReleaseDir 'build') -Directory -Filter 'llama-cpp-sys-2-*' |
        ForEach-Object { Join-Path $_.FullName 'out' } |
        Where-Object { Test-Path -Path (Join-Path $_ 'backends\ggml-vulkan*.dll') } |
        Select-Object -First 1
    if (-not $sysOut) { throw "no ggml-vulkan DLL under $ReleaseDir\build\llama-cpp-sys-2-*\out\backends" }
    $backendsOut = Join-Path $ReleaseDir 'backends'
    New-Item -ItemType Directory -Force -Path $backendsOut | Out-Null
    Copy-Item -Path (Join-Path $sysOut 'backends\*.dll') -Destination $backendsOut -Force
    Copy-Item -Path (Join-Path $sysOut 'bin\*.dll') -Destination $ReleaseDir -Force
    if (-not (Get-ChildItem -LiteralPath $ReleaseDir -File -Filter 'llama*.dll')) { throw "llama*.dll was not found in $sysOut" }
    Write-Note "backends: $backendsOut ($((Get-ChildItem -LiteralPath $backendsOut -Filter '*.dll').Count) DLLs)"
}

foreach ($f in @($EngineExe, $TipDllBuilt)) {
    if (-not (Test-Path -LiteralPath $f)) { throw "build finished but $f does not exist" }
    $item = Get-Item -LiteralPath $f
    Write-Note ("{0}  {1:N0} bytes  {2}" -f $item.FullName, $item.Length, $item.LastWriteTime)
}
Write-Step 'Build done. Next: install-data.ps1, then register.ps1 (as administrator), then start-engine.ps1'
