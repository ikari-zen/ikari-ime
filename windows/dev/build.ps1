<#
.SYNOPSIS
  Builds the engine (ikari-engine-host.exe) and the TIP (ikari_tip.dll) in release mode. Without the LM unless -Lm.
  With -Lm or -LmVulkan it also builds the LM child process (ikari-lm-host.exe, next to the engine exe): the engine
  asks it to run the model (ADR 0060 Q4). The engine itself never contains llama.cpp (ADR 0072 section 5): without that
  file next to it, the engine runs without the LM.
.PARAMETER NoCandidate
  Do not build the candidate window (ikari_candidate.dll and ikari-candidate.exe, ADR 0062). The window needs the .NET SDK
  (version in windows\candidate-ui\global.json); without the SDK this step is skipped with a warning.
.PARAMETER HideConsole
  Build the engine with the "hide-console" feature (no console window; failures go to logs\start-failure.log).
  Leave it off while testing: you want the console.
.PARAMETER Lm
  Build the LM child process with the "llama" feature (zenz via llama.cpp, CPU). Needs cmake. The LM stays OFF at run time until
  you turn it on (settings.tsv in the data directory: engine.lm=true) and the model is in <DataDir>\models\.
.PARAMETER LmVulkan
  Build the LM child process with the "vulkan" feature (zenz via llama.cpp, on the GPU with Vulkan; falls back to the CPU on a
  machine without a GPU or without vulkan-1.dll). Cannot be combined with -Lm. Needs cmake, Ninja and the Vulkan SDK
  (VULKAN_SDK, glslc). The backend DLLs (ggml-vulkan.dll and the CPU variants) are copied next to the engine exe (and the child), in
  <release>\backends. Turn it off at run time with "engine.lm_gpu<TAB>off" in settings.tsv (ADR 0059).
.NOTES
  This is the development build: it always turns on the "dev-knobs" features (ikari-engine-host, ikari-tip, and the LM
  child process). The scripts in this folder rely on them: IKARI_USER_DIR, IKARI_TIP_LATENCY / latency.on, the flat TIP
  layout, "--data-dir" for the engine, and the TIP's debug output (OutputDebugString). The product build
  (cargo build --release -p ikari-engine-host -p ikari-tip, with no features) has none of them.
  Needs Rust (stable-x86_64-pc-windows-msvc) and Visual Studio Build Tools (C++ workload). cmake is needed only with
  -Lm and -LmVulkan.
  -LmVulkan: if the build stops in rc.exe with RC2136, the build folder path is too long: set CARGO_TARGET_DIR to a short
  path (for example C:\t) and run it again.
#>
param([switch]$HideConsole, [switch]$Lm, [switch]$LmVulkan, [switch]$NoCandidate)

. "$PSScriptRoot\common.ps1"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'cargo not found. Install Rust with rustup (stable-x86_64-pc-windows-msvc).'
}

if ($Lm -and $LmVulkan) { throw '-Lm and -LmVulkan cannot be combined (-LmVulkan already includes the LM).' }

# Development knobs (ADR 0067 section 6). Always on in this folder's scripts; never in the product build.
$features = @('ikari-engine-host/dev-knobs', 'ikari-tip/dev-knobs')
if ($HideConsole) { $features += 'ikari-engine-host/hide-console' }
if ($Lm) {
    if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
        throw 'cmake not found. The LM build (llama.cpp) needs cmake: install it (https://cmake.org/download/) and reopen the shell.'
    }
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
}

$cargoArgs = @('build', '--release', '-p', 'ikari-engine-host', '-p', 'ikari-tip')
# The candidate window's core (ikari_candidate.dll; the C# screen calls only this DLL). It has no dev-knobs.
if (-not $NoCandidate) { $cargoArgs += @('-p', 'ikari-candidate') }
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

if ($Lm -or $LmVulkan) {
    # The LM child process: the only place llama.cpp is linked.
    $lmFeature = if ($LmVulkan) { 'vulkan' } else { 'llama' }
    $hostArgs = @('build', '--release', '-p', 'ikari-lm', '--bin', 'ikari-lm-host', '--features', "$lmFeature,dev-knobs")
    Write-Step "cargo $($hostArgs -join ' ')   (in $RepoRoot)"
    Push-Location $RepoRoot
    try {
        & cargo @hostArgs
        if ($LASTEXITCODE -ne 0) { throw "cargo build (ikari-lm-host) failed (exit code $LASTEXITCODE)" }
    }
    finally {
        Pop-Location
    }
    $lmHostExe = Join-Path $ReleaseDir 'ikari-lm-host.exe'
    if (-not (Test-Path -LiteralPath $lmHostExe)) { throw "build finished but $lmHostExe does not exist" }
    Write-Note "LM child process: $lmHostExe"
}

if ($LmVulkan) {
    # llama-cpp-sys-2 builds ggml as DLLs. The backends (ggml-vulkan.dll, ggml-cpu-*.dll) are in <OUT_DIR>\backends and are
    # NOT copied to target. The LM child process loads them from "backends" next to its exe (lm/src/llama.rs). ggml*.dll and
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

if (-not $NoCandidate) {
    if (-not (Get-Command dotnet -ErrorAction SilentlyContinue)) {
        Write-Warn 'dotnet not found: the candidate window is not built (typing works without it). Install the .NET SDK (windows\candidate-ui\global.json).'
    }
    else {
        if (-not (Test-Path -LiteralPath $CandidateCoreDll)) { throw "build finished but $CandidateCoreDll does not exist" }
        # Self-contained, unpackaged (ADR 0062 s3). The NuGet packages must match windows\candidate-ui\packages.lock.json.
        $publishArgs = @('publish', $CandidateProject, '-c', 'Release', '-o', $CandidatePublishDir, "-p:IkariCoreDll=$CandidateCoreDll", '-p:RestoreLockedMode=true')
        Write-Step "dotnet $($publishArgs -join ' ')"
        & dotnet @publishArgs
        if ($LASTEXITCODE -ne 0) { throw "dotnet publish failed (exit code $LASTEXITCODE)" }
        $candidateExe = Join-Path $CandidatePublishDir $CandidateExeName
        if (-not (Test-Path -LiteralPath $candidateExe)) { throw "publish finished but $candidateExe does not exist" }
        Write-Note "candidate window: $candidateExe"
    }
}

foreach ($f in @($EngineExe, $TipDllBuilt)) {
    if (-not (Test-Path -LiteralPath $f)) { throw "build finished but $f does not exist" }
    $item = Get-Item -LiteralPath $f
    Write-Note ("{0}  {1:N0} bytes  {2}" -f $item.FullName, $item.Length, $item.LastWriteTime)
}
Write-Step 'Build done. Next: install-data.ps1, then register.ps1 (as administrator), then start-engine.ps1'
