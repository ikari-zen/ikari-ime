<#
.SYNOPSIS
  Copies ikari_tip.dll (and the keymap tables) to a NEW version folder $TipVersionsDir\<time>\ and registers it with
  regsvr32. Run it in an ADMINISTRATOR 64-bit PowerShell: DllRegisterServer writes HKCR (= HKLM\Software\Classes).
.PARAMETER DryRun
  Show the plan only.
.PARAMETER Force
  Do not ask for confirmation.
.NOTES
  Changes made on the machine (shown before they are done, and undone by unregister.ps1):
    1. a new folder $TipVersionsDir\<time>\ (ikari_tip.dll, resources\keymap\*.tsv)
    2. registry + TSF profile, written by the DLL itself (DllRegisterServer): HKCR\CLSID\{...}, the Japanese
       (0x0411) input profile "ikari", and the TIP keyboard category.
  Updating (ADR 0063 s3): the old version is NOT unregistered and its folder is not touched (an app that has it loaded
  could not let it be overwritten anyway). The new DllRegisterServer re-points InprocServer32 to the new folder; apps
  started after that load the new version, apps already open keep the old one until they exit (sign out to switch
  all of them). Delete the old folders later with clean-tip.ps1.
  Run it inside the VM (a TIP bug can freeze Windows: ADR 0040 s8). Take a VM snapshot first.

#>
param([switch]$DryRun, [switch]$Force)

. "$PSScriptRoot\common.ps1"

if (-not (Test-Path -LiteralPath $TipDllBuilt)) { throw "Not built: $TipDllBuilt. Run .\build.ps1 first." }
if (-not (Test-Path -LiteralPath $KeymapSource -PathType Container)) { throw "Keymap tables not found: $KeymapSource" }
if (-not (Test-Path -LiteralPath $Regsvr32)) { throw "regsvr32 not found: $Regsvr32" }
if (-not [Environment]::Is64BitProcess) { throw 'Use a 64-bit PowerShell (the TIP is x64).' }

$versionName = (Get-Date).ToString($TipVersionFormat)
$versionDir = Join-Path $TipVersionsDir $versionName
$dllDest = Join-Path $versionDir $TipDllName
$keymapDest = Join-Path (Join-Path $versionDir $TipResourcesDirName) 'keymap'
$previous = Get-RegisteredTipDll

Write-Step 'Plan'
if ($previous) { Write-Note "registered now: $previous   (kept as it is; not unregistered)" }
else { Write-Note 'registered now: (nothing)' }
Write-Note "mkdir  $versionDir"
Write-Note "copy   $TipDllBuilt"
Write-Note "  to   $dllDest"
Write-Note "copy   $KeymapSource\*.tsv"
Write-Note "  to   $keymapDest"
Write-Note "run    $Regsvr32 /s `"$dllDest`"   (points HKCR\CLSID\$TipClsid to the new DLL, writes the TSF profile)"
if ($DryRun) { Write-Step 'Dry run: nothing was changed.'; return }

if (-not (Test-IsAdmin)) { throw 'Run this script in an administrator PowerShell (regsvr32 writes HKCR).' }
if (Test-Path -LiteralPath $versionDir) { throw "Already exists: $versionDir. Run the script again." }
if (-not (Confirm-Action 'Go ahead?' $Force.IsPresent)) { Write-Step 'Cancelled.'; return }

# The new folder is not loaded by any app yet, so it can be removed again when something below fails.
function Undo-NewVersion {
    try { Remove-Item -LiteralPath $versionDir -Recurse -Force }
    catch { Write-Warn "Could not delete $versionDir`: $($_.Exception.Message). Delete it by hand or with clean-tip.ps1." }
}

try {
    New-Item -ItemType Directory -Path $keymapDest -Force | Out-Null
    Copy-Item -LiteralPath $TipDllBuilt -Destination $dllDest
    Copy-Item -Path (Join-Path $KeymapSource '*.tsv') -Destination $keymapDest
}
catch {
    Write-Warn "Could not copy into the new folder $versionDir`: $($_.Exception.Message)"
    Undo-NewVersion
    throw
}

Write-Step "regsvr32 /s $dllDest"
# regsvr32 is a GUI-subsystem exe: `&` does not wait for it and does not set $LASTEXITCODE.
$code = (Start-Process -FilePath $Regsvr32 -ArgumentList '/s', ('"{0}"' -f $dllDest) -Wait -PassThru).ExitCode
if ($code -ne 0) {
    # When the TIP was registered before, a failed DllRegisterServer keeps that registration (windows/tip/src/deploy.rs).
    $now = Get-RegisteredTipDll
    if ($now -and ($now -ieq $dllDest)) {
        Write-Warn "InprocServer32 already points to the new DLL; keeping $versionDir."
    }
    else { Undo-NewVersion }
    # regsvr32 exit codes: 3 = LoadLibrary failed, 4 = DllRegisterServer not found, 5 = DllRegisterServer failed
    throw "regsvr32 failed with exit code $code (3 = DLL did not load, 4 = no DllRegisterServer, 5 = DllRegisterServer returned an error). Registered now: $now. See the 'if it does not work' part of check item 1."
}
Write-Step "Registered version $versionName."
if ($previous) {
    Write-Note 'Apps started from now on load the new version. Apps already open keep the old one until they exit;'
    Write-Note 'sign out and in to switch all of them. Then delete the old folders with .\clean-tip.ps1.'
}
else {
    Write-Note 'Add the input method: Settings > Time & language > Language & region > Japanese > Language options > Add a keyboard > "ikari".'
}
if (-not (Test-Path -LiteralPath $DevEngineExe)) {
    Write-Note "No engine at $DevEngineExe yet: .\start-engine.ps1 copies it there (the TIP talks only to that path; #409)."
}
Write-Note 'Check with .\status.ps1'
