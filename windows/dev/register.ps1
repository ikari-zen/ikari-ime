<#
.SYNOPSIS
  Copies ikari_tip.dll (and the keymap tables) to $DevInstallDir and registers it with regsvr32.
  Run it in an ADMINISTRATOR 64-bit PowerShell: DllRegisterServer writes HKCR (= HKLM\Software\Classes).
.PARAMETER DryRun
  Show the plan only.
.PARAMETER Force
  Do not ask for confirmation.
.NOTES
  Changes made on the machine (shown before they are done, and undone by unregister.ps1):
    1. files in $DevInstallDir (ikari_tip.dll, resources\keymap\*.tsv)
    2. registry + TSF profile, written by the DLL itself (DllRegisterServer): HKCR\CLSID\{...}, the Japanese
       (0x0411) input profile "ikari", and the TIP keyboard category.
  Run it inside the VM (a TIP bug can freeze Windows: ADR 0040 s8). Take a VM snapshot first.
  Never run on Windows when written: syntax only.
#>
param([switch]$DryRun, [switch]$Force)

. "$PSScriptRoot\common.ps1"

if (-not (Test-Path -LiteralPath $TipDllBuilt)) { throw "Not built: $TipDllBuilt. Run .\build.ps1 first." }
if (-not (Test-Path -LiteralPath $KeymapSource -PathType Container)) { throw "Keymap tables not found: $KeymapSource" }
if (-not (Test-Path -LiteralPath $Regsvr32)) { throw "regsvr32 not found: $Regsvr32" }
if (-not [Environment]::Is64BitProcess) { throw 'Use a 64-bit PowerShell (the TIP is x64).' }

Write-Step 'Plan'
Write-Note "copy   $TipDllBuilt"
Write-Note "  to   $TipDllInstalled"
Write-Note "copy   $KeymapSource\*.tsv"
Write-Note "  to   $(Join-Path $TipResourcesInstalled 'keymap')"
Write-Note "run    $Regsvr32 /s `"$TipDllInstalled`"   (writes HKCR\CLSID\$TipClsid and the TSF profile)"
if ($DryRun) { Write-Step 'Dry run: nothing was changed.'; return }

if (-not (Test-IsAdmin)) { throw 'Run this script in an administrator PowerShell (regsvr32 writes HKCR).' }
if (-not (Confirm-Action 'Go ahead?' $Force.IsPresent)) { Write-Step 'Cancelled.'; return }

if (-not (Test-Path -LiteralPath $DevInstallDir)) { New-Item -ItemType Directory -Path $DevInstallDir | Out-Null }
$keymapDest = Join-Path $TipResourcesInstalled 'keymap'
if (-not (Test-Path -LiteralPath $keymapDest)) { New-Item -ItemType Directory -Path $keymapDest | Out-Null }

try {
    Copy-Item -LiteralPath $TipDllBuilt -Destination $TipDllInstalled -Force
}
catch {
    Write-Warn "Could not overwrite $TipDllInstalled (an app probably has it loaded; ADR 0040 S9)."
    Write-Note 'Close the apps that used ikari, or sign out and in, or restart the VM. Then run unregister.ps1 and register.ps1 again.'
    throw
}
Copy-Item -Path (Join-Path $KeymapSource '*.tsv') -Destination $keymapDest -Force

Write-Step "regsvr32 /s $TipDllInstalled"
& $Regsvr32 /s $TipDllInstalled
$code = $LASTEXITCODE
if ($code -ne 0) {
    # regsvr32 exit codes: 3 = LoadLibrary failed, 4 = DllRegisterServer not found, 5 = DllRegisterServer failed
    throw "regsvr32 failed with exit code $code (3 = DLL did not load, 4 = no DllRegisterServer, 5 = DllRegisterServer returned an error). See the 'if it does not work' part of check item 1."
}
Write-Step 'Registered. Add the input method: Settings > Time & language > Language & region > Japanese > Language options > Add a keyboard > "ikari".'
Write-Note 'Check with .\status.ps1'
