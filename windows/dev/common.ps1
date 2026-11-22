# Shared settings and helpers for the dev scripts in this folder (ADR 0040 W5-b, issue #331).
# Dot-source it from the top of each script:   . "$PSScriptRoot\common.ps1"
#
# These scripts are ASCII only on purpose: Windows PowerShell 5.1 reads a file without a BOM as the ANSI code page.
# They have NEVER been run on Windows when they were written (no Windows machine at hand). Expect to fix them
# the first time you run them, and write what you changed in docs\windows-real-device-check.md (section "Results").

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------- settings (change them here, in one place)

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

# Data directory (dict\, models\, logs\, settings.tsv, learning files).
# It must be the same place the engine and the TIP use: IKARI_USER_DIR, else %LOCALAPPDATA%\ikari-ime
# (core::user::dir; decided 2026-11-12, #333). Not Roaming: the dictionary is over 600MB and must not sync.
# Data once put in %APPDATA%\ikari-ime for development: move it by hand to %LOCALAPPDATA%\ikari-ime, or delete it.
$DefaultDataDir = Join-Path $env:LOCALAPPDATA 'ikari-ime'
if ($env:IKARI_USER_DIR) { $DataDir = $env:IKARI_USER_DIR } else { $DataDir = $DefaultDataDir }

# cargo output
if ($env:CARGO_TARGET_DIR) { $CargoTarget = $env:CARGO_TARGET_DIR } else { $CargoTarget = Join-Path $RepoRoot 'target' }
$ReleaseDir = Join-Path $CargoTarget 'release'

# The package is ikari-engine-host; there is no [[bin]] rename, so the exe is ikari-engine-host.exe
# (the ADR and the PR text call it "ikari-engine.exe").
$EngineExeName = 'ikari-engine-host.exe'
$EngineExe = Join-Path $ReleaseDir $EngineExeName
# The no-console build (hide-console feature). The CI artifact ships it next to the console one (#372); build.ps1 -HideConsole
# overwrites $EngineExe instead and does not make this file.
$EngineHiddenExe = Join-Path $ReleaseDir 'ikari-engine-host-hidden.exe'
$TipDllName = 'ikari_tip.dll'
$TipDllBuilt = Join-Path $ReleaseDir $TipDllName
$KeymapSource = Join-Path $RepoRoot 'shell-core\resources\keymap'

# Install layout (ADR 0063 s3), the same shape as the installer:
#   <install dir>\ikari-engine-host.exe              the engine (fixed path, not per version; always THIS name)
#   <install dir>\llama*.dll, ggml*.dll, backends\   next to the engine (LM builds only; lm/src/llama.rs)
#   <install dir>\tip\<version>\ikari_tip.dll         the TIP, one folder per version
#   <install dir>\tip\<version>\resources\keymap\     the TIP reads "resources" next to the DLL (windows/tip/src/deploy.rs)
# The TIP accepts only the engine at <install dir>\ikari-engine-host.exe (two folders up from the DLL, the middle one
# named "tip"; #409). So start-engine.ps1 COPIES the built engine there under that name (also the no-console build,
# ikari-engine-host-hidden.exe) and starts the copy; it never runs the engine from target\release.
# <version> is the time of register.ps1, so every register.ps1 makes a NEW folder: it never overwrites a DLL that
# apps have loaded, and the new DllRegisterServer only re-points the registration (no unregister).
$DevInstallDir = Join-Path $env:LOCALAPPDATA 'ikari-ime-dev\install'
$DevEngineExe = Join-Path $DevInstallDir $EngineExeName
# Files the engine loads from next to its exe (copied with it when they are in $ReleaseDir)
$EngineSideDllPatterns = @('llama*.dll', 'ggml*.dll')
$EngineBackendsDirName = 'backends'
$TipVersionsDir = Join-Path $DevInstallDir 'tip'
# The candidate window (ADR 0062): <install dir>\candidate\ikari-candidate.exe with the self-contained .NET and Windows App SDK
# files next to it. The engine starts and watches it (Job Object; windows/engine-host/src/candidate_window.rs); the TIP talks
# only to that exe (ADR 0068 s5). build.ps1 publishes it to $CandidatePublishDir; Install-DevEngine copies it.
$CandidateDirName = 'candidate'
$CandidateExeName = 'ikari-candidate.exe'
$CandidateProject = Join-Path $RepoRoot 'windows\candidate-ui'
$CandidateCoreDll = Join-Path $ReleaseDir 'ikari_candidate.dll'
$CandidatePublishDir = Join-Path $ReleaseDir 'candidate-publish'
$DevCandidateDir = Join-Path $DevInstallDir $CandidateDirName
$DevCandidateExe = Join-Path $DevCandidateDir $CandidateExeName
$TipResourcesDirName = 'resources'
# Folder name of a version made by register.ps1: sortable time (sorting by name = sorting by age)
$TipVersionFormat = 'yyyyMMdd-HHmmss-fff'
$TipVersionPattern = '^\d{8}-\d{6}-\d{3}$'
# Where register.ps1 put the DLL before ADR 0063 (one fixed path). Only shown by status.ps1; delete it by hand.
$LegacyTipDir = Join-Path $env:LOCALAPPDATA 'ikari-ime-dev\tip'

# COM class of the TIP (windows/tip/src/com/guids.rs CLSID_TEXT_SERVICE)
$TipClsid = '{83A140BE-DECB-4358-867B-7B77D6541276}'

# Name prefixes of the named pipes (pipe/src/name.rs). Full name: <prefix><user SID>-s<session id>
$EnginePipePrefix = 'ikari-ime-engine-v1-'
$SettingsPipePrefix = 'ikari-ime-settings-v1-'
$CandidatePipePrefix = 'ikari-ime-candidate-v1-'

$Regsvr32 = Join-Path $env:windir 'System32\regsvr32.exe'

# ---------------------------------------------------------------- helpers

function Write-Step([string]$Message) { Write-Host "==> $Message" -ForegroundColor Cyan }
function Write-Note([string]$Message) { Write-Host "    $Message" }
function Write-Warn([string]$Message) { Write-Host "WARNING: $Message" -ForegroundColor Yellow }

function Test-IsAdmin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# Show what is about to happen and ask. -Force skips the question (the plan is still shown by the caller).
# Returns $true to go on.
function Confirm-Action([string]$Question, [bool]$Force) {
    if ($Force) { return $true }
    $answer = Read-Host "$Question [y/N]"
    return ($answer -eq 'y' -or $answer -eq 'Y')
}

# Engine processes started from OUR install copy or OUR build only (matched by full exe path). Never touches other
# processes. The build paths stay in the list so that an engine started by an older start-engine.ps1 can be stopped.
function Get-OurEngineProcesses {
    $found = @()
    foreach ($exe in @($DevEngineExe, $EngineExe, $EngineHiddenExe)) {
        $procName = [IO.Path]::GetFileNameWithoutExtension($exe)
        foreach ($p in @(Get-Process -Name $procName -ErrorAction SilentlyContinue)) {
            $path = $null
            try { $path = $p.Path } catch { $path = $null }
            if ($path -and ($path -ieq $exe) -and -not ($found | Where-Object { $_.Id -eq $p.Id })) { $found += $p }
        }
    }
    return $found
}

# Copies the engine $SourceExe to $DevEngineExe (always named ikari-engine-host.exe, whatever $SourceExe is called),
# and the DLLs and backends\ next to it. Call it only when our engine is not running (a running exe cannot be
# overwritten). The exe is not copied when it is the same file (sha256).
function Install-DevEngine([string]$SourceExe) {
    if (-not (Test-Path -LiteralPath $DevInstallDir)) { New-Item -ItemType Directory -Path $DevInstallDir | Out-Null }
    $same = (Test-Path -LiteralPath $DevEngineExe) -and
        ((Get-FileHash -LiteralPath $SourceExe).Hash -eq (Get-FileHash -LiteralPath $DevEngineExe).Hash)
    if ($same) { Write-Note "engine unchanged: $DevEngineExe" }
    else {
        Copy-Item -LiteralPath $SourceExe -Destination $DevEngineExe -Force
        Write-Note "copied $SourceExe"
        Write-Note "    to $DevEngineExe"
    }
    $sourceDir = Split-Path -Parent $SourceExe
    # The LM host (ADR 0060 Q4) must sit next to the engine, otherwise the engine runs the LM in its own process.
    $lmHost = Join-Path $sourceDir 'ikari-lm-host.exe'
    if (Test-Path -LiteralPath $lmHost) {
        Copy-Item -LiteralPath $lmHost -Destination (Join-Path $DevInstallDir 'ikari-lm-host.exe') -Force
    }
    foreach ($pattern in $EngineSideDllPatterns) {
        foreach ($f in @(Get-ChildItem -Path (Join-Path $sourceDir $pattern) -File -ErrorAction SilentlyContinue)) {
            Copy-Item -LiteralPath $f.FullName -Destination (Join-Path $DevInstallDir $f.Name) -Force
        }
    }
    # The candidate window (published by build.ps1, or the "candidate" folder of the CI artifact). It is stopped with the
    # engine (Job Object), so it can be overwritten here. Without it, typing works without the window.
    $candidateSource = $null
    foreach ($dir in @($CandidatePublishDir, (Join-Path $sourceDir $CandidateDirName))) {
        if (Test-Path -LiteralPath (Join-Path $dir $CandidateExeName)) { $candidateSource = $dir; break }
    }
    if ($candidateSource) {
        if (-not (Test-Path -LiteralPath $DevCandidateDir)) { New-Item -ItemType Directory -Path $DevCandidateDir | Out-Null }
        Copy-Item -Path (Join-Path $candidateSource '*') -Destination $DevCandidateDir -Recurse -Force
        Write-Note "candidate window: $DevCandidateExe"
    }
    else { Write-Warn "no candidate window built ($CandidatePublishDir). Typing works, but without the window." }
    $backends = Join-Path $sourceDir $EngineBackendsDirName
    if (Test-Path -LiteralPath $backends -PathType Container) {
        $backendsDest = Join-Path $DevInstallDir $EngineBackendsDirName
        if (-not (Test-Path -LiteralPath $backendsDest)) { New-Item -ItemType Directory -Path $backendsDest | Out-Null }
        Copy-Item -Path (Join-Path $backends '*') -Destination $backendsDest -Recurse -Force
    }
}

function Get-CurrentUserSid {
    return [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
}

function Get-CurrentSessionId {
    return (Get-Process -Id $PID).SessionId
}

function Get-PipeName([string]$Prefix) {
    return "$Prefix$(Get-CurrentUserSid)-s$(Get-CurrentSessionId)"
}

# ---------------------------------------------------------------- TIP versions (ADR 0063)

# The DLL that HKLM\Software\Classes\CLSID\{TIP}\InprocServer32 points to, or $null when the TIP is not registered. Read only.
function Get-RegisteredTipDll {
    $key = "Registry::HKEY_CLASSES_ROOT\CLSID\$TipClsid\InprocServer32"
    if (-not (Test-Path -LiteralPath $key)) { return $null }
    $value = (Get-ItemProperty -LiteralPath $key).'(default)'
    if ([string]::IsNullOrEmpty($value)) { return $null }
    return $value
}

# The version folder name when $DllPath is <TipVersionsDir>\<name>\ikari_tip.dll, else $null.
function Get-TipVersionName([string]$DllPath) {
    if ([string]::IsNullOrEmpty($DllPath)) { return $null }
    if ((Split-Path -Leaf $DllPath) -ine $TipDllName) { return $null }
    $versionDir = Split-Path -Parent $DllPath
    $parent = Split-Path -Parent $versionDir
    if (-not $parent) { return $null }
    $expected = [IO.Path]::GetFullPath($TipVersionsDir).TrimEnd('\')
    if ([IO.Path]::GetFullPath($parent).TrimEnd('\') -ine $expected) { return $null }
    return (Split-Path -Leaf $versionDir)
}

# The version folders under $TipVersionsDir, oldest first (by name, which is the time of register.ps1).
function Get-TipVersionDirs {
    if (-not (Test-Path -LiteralPath $TipVersionsDir -PathType Container)) { return @() }
    return @(Get-ChildItem -LiteralPath $TipVersionsDir -Directory |
            Where-Object { $_.Name -match $TipVersionPattern } | Sort-Object -Property Name)
}

# Tries to delete each version folder. The DLL goes first: when an app still has it loaded, deleting it fails and
# the whole folder is kept, so the old TIP in that app can still read its resources (it reloads the keymap when the
# settings change). Returns the folders that were kept (with the reason). No reboot-time delete is scheduled
# (MOVEFILE_DELAY_UNTIL_REBOOT needs an administrator; that is the installer's job, ADR 0063 s3-4).
function Remove-TipVersionDirs([IO.DirectoryInfo[]]$Dirs) {
    $kept = @()
    foreach ($dir in $Dirs) {
        $dll = Join-Path $dir.FullName $TipDllName
        try {
            if (Test-Path -LiteralPath $dll) { Remove-Item -LiteralPath $dll -Force }
            Remove-Item -LiteralPath $dir.FullName -Recurse -Force
            Write-Note "deleted $($dir.FullName)"
        }
        catch {
            $kept += [pscustomobject]@{ Path = $dir.FullName; Reason = $_.Exception.Message }
            Write-Warn "kept $($dir.FullName) (probably still loaded by an app): $($_.Exception.Message)"
        }
    }
    return $kept
}

# Names of pipes that start with an ikari prefix (listing \\.\pipe\ does not open any of them)
function Get-IkariPipes {
    $names = [IO.Directory]::GetFiles('\\.\pipe\')
    $result = @()
    foreach ($n in $names) {
        $leaf = Split-Path -Leaf $n
        if ($leaf.StartsWith($EnginePipePrefix) -or $leaf.StartsWith($SettingsPipePrefix) -or
            $leaf.StartsWith($CandidatePipePrefix)) { $result += $leaf }
    }
    return $result
}
