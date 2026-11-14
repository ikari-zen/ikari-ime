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

# The DLL that regsvr32 registers is a COPY in this folder, so that a rebuild never has to overwrite a DLL
# that apps have loaded. The TIP looks for "resources" next to the DLL (windows/tip/src/com/host.rs).
$DevInstallDir = Join-Path $env:LOCALAPPDATA 'ikari-ime-dev\tip'
$TipDllInstalled = Join-Path $DevInstallDir $TipDllName
$TipResourcesInstalled = Join-Path $DevInstallDir 'resources'

# COM class of the TIP (windows/tip/src/com/guids.rs CLSID_TEXT_SERVICE)
$TipClsid = '{83A140BE-DECB-4358-867B-7B77D6541276}'

# Name prefixes of the named pipes (pipe/src/name.rs). Full name: <prefix><user SID>-s<session id>
$EnginePipePrefix = 'ikari-ime-engine-v1-'
$SettingsPipePrefix = 'ikari-ime-settings-v1-'

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

# Engine processes started from OUR build only (matched by full exe path). Never touches other processes.
function Get-OurEngineProcesses {
    $found = @()
    foreach ($exe in @($EngineExe, $EngineHiddenExe)) {
        $procName = [IO.Path]::GetFileNameWithoutExtension($exe)
        foreach ($p in @(Get-Process -Name $procName -ErrorAction SilentlyContinue)) {
            $path = $null
            try { $path = $p.Path } catch { $path = $null }
            if ($path -and ($path -ieq $exe)) { $found += $p }
        }
    }
    return $found
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

# Names of pipes that start with an ikari prefix (listing \\.\pipe\ does not open any of them)
function Get-IkariPipes {
    $names = [IO.Directory]::GetFiles('\\.\pipe\')
    $result = @()
    foreach ($n in $names) {
        $leaf = Split-Path -Leaf $n
        if ($leaf.StartsWith($EnginePipePrefix) -or $leaf.StartsWith($SettingsPipePrefix)) { $result += $leaf }
    }
    return $result
}
