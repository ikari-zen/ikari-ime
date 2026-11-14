<#
.SYNOPSIS
  Unregisters the TIP (regsvr32 /u on the copy made by register.ps1). Run it in an ADMINISTRATOR 64-bit PowerShell.
.PARAMETER RemoveFiles
  Also delete the copy (ikari_tip.dll and resources\keymap\*.tsv inside $DevInstallDir, and the folders if empty).
  Nothing else is deleted: not the data directory, not the learning, not the build output.
.PARAMETER DryRun
  Show the plan only.
.PARAMETER Force
  Do not ask for confirmation.
.NOTES
  The registry keys are removed by the DLL itself (DllUnregisterServer), not by this script. This script never
  runs reg delete or Remove-Item on the registry.
  If an app has the DLL loaded, the files cannot be deleted until the app is closed (or you sign out).
 
#>
param([switch]$RemoveFiles, [switch]$DryRun, [switch]$Force)

. "$PSScriptRoot\common.ps1"

if (-not (Test-Path -LiteralPath $TipDllInstalled)) {
    throw "Not found: $TipDllInstalled. Nothing registered by register.ps1 (if you registered another path, run regsvr32 /u on that path yourself)."
}

Write-Step 'Plan'
Write-Note "run    $Regsvr32 /u /s `"$TipDllInstalled`"   (the DLL removes HKCR\CLSID\$TipClsid and its TSF profile)"
if ($RemoveFiles) {
    Write-Note "delete $TipDllInstalled"
    Write-Note "delete $(Join-Path $TipResourcesInstalled 'keymap\*.tsv')"
    Write-Note 'delete the folders resources\keymap, resources and tip only if they are empty'
}
if ($DryRun) { Write-Step 'Dry run: nothing was changed.'; return }

if (-not (Test-IsAdmin)) { throw 'Run this script in an administrator PowerShell.' }
if (-not (Confirm-Action 'Go ahead?' $Force.IsPresent)) { Write-Step 'Cancelled.'; return }

Write-Step "regsvr32 /u /s $TipDllInstalled"
# regsvr32 is a GUI-subsystem exe: `&` does not set $LASTEXITCODE (see register.ps1).
$code = (Start-Process -FilePath $Regsvr32 -ArgumentList '/u', '/s', ('"{0}"' -f $TipDllInstalled) -Wait -PassThru).ExitCode
if ($code -ne 0) {
    throw "regsvr32 /u failed with exit code $code (3 = DLL did not load, 4 = no DllUnregisterServer, 5 = it returned an error)."
}
Write-Note 'Unregistered. The input method disappears from the list; apps that are already open may keep it until they restart.'

if ($RemoveFiles) {
    $keymapDir = Join-Path $TipResourcesInstalled 'keymap'
    try {
        Remove-Item -LiteralPath $TipDllInstalled -Force
        if (Test-Path -LiteralPath $keymapDir) {
            Get-ChildItem -LiteralPath $keymapDir -Filter '*.tsv' -File | Remove-Item -Force
        }
        foreach ($dir in @($keymapDir, $TipResourcesInstalled, $DevInstallDir)) {
            if ((Test-Path -LiteralPath $dir) -and -not (Get-ChildItem -LiteralPath $dir -Force)) {
                Remove-Item -LiteralPath $dir
            }
        }
        Write-Note 'Copy deleted.'
    }
    catch {
        Write-Warn "Could not delete the files (an app probably still has the DLL loaded): $($_.Exception.Message)"
        Write-Note 'Close the apps or sign out, then run this script again with -RemoveFiles.'
    }
}
Write-Step 'Done. Check with .\status.ps1'
