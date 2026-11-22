<#
.SYNOPSIS
  Unregisters the TIP (regsvr32 /u on the DLL that is registered now, which register.ps1 put in
  $TipVersionsDir\<time>\). Run it in an ADMINISTRATOR 64-bit PowerShell.
.PARAMETER RemoveFiles
  Also try to delete every version folder in $TipVersionsDir (and the folder itself if it ends up empty).
  A folder whose DLL an app still has loaded is kept and reported (close the apps or sign out, then run it again).
  Nothing else is deleted: not the data directory, not the learning, not the build output.
.PARAMETER DryRun
  Show the plan only.
.PARAMETER Force
  Do not ask for confirmation.
.NOTES
  The registry keys are removed by the DLL itself (DllUnregisterServer), not by this script. This script never
  runs reg delete or Remove-Item on the registry.
  To delete only the old versions and keep the TIP registered, use clean-tip.ps1.

#>
param([switch]$RemoveFiles, [switch]$DryRun, [switch]$Force)

. "$PSScriptRoot\common.ps1"

$registered = Get-RegisteredTipDll
if ($registered) {
    if (-not (Get-TipVersionName $registered)) {
        throw "The registered DLL is not one made by register.ps1: $registered. Run regsvr32 /u on that path yourself."
    }
    if (-not (Test-Path -LiteralPath $registered)) {
        throw "The registered DLL does not exist: $registered (regsvr32 /u needs it). Restore the VM snapshot, or put the same build back there."
    }
}
elseif (-not $RemoveFiles) {
    throw 'The TIP is not registered. Nothing to do (add -RemoveFiles to delete the version folders).'
}
$dirs = @(Get-TipVersionDirs)

Write-Step 'Plan'
if ($registered) { Write-Note "run    $Regsvr32 /u /s `"$registered`"   (the DLL removes HKLM\Software\Classes\CLSID\$TipClsid and its TSF profile)" }
else { Write-Note 'the TIP is not registered: no regsvr32 /u' }
if ($RemoveFiles) {
    if ($dirs.Count -eq 0) { Write-Note "no version folder in $TipVersionsDir" }
    foreach ($d in $dirs) { Write-Note "delete $($d.FullName)   (kept if an app has its DLL loaded)" }
    Write-Note "delete $TipVersionsDir only if it is empty"
}
if ($DryRun) { Write-Step 'Dry run: nothing was changed.'; return }

if ($registered -and -not (Test-IsAdmin)) { throw 'Run this script in an administrator PowerShell.' }
if (-not (Confirm-Action 'Go ahead?' $Force.IsPresent)) { Write-Step 'Cancelled.'; return }

if ($registered) {
    Write-Step "regsvr32 /u /s $registered"
    # regsvr32 is a GUI-subsystem exe: `&` does not set $LASTEXITCODE (see register.ps1).
    $code = (Start-Process -FilePath $Regsvr32 -ArgumentList '/u', '/s', ('"{0}"' -f $registered) -Wait -PassThru).ExitCode
    if ($code -ne 0) {
        throw "regsvr32 /u failed with exit code $code (3 = DLL did not load, 4 = no DllUnregisterServer, 5 = it returned an error)."
    }
    Write-Note 'Unregistered. The input method disappears from the list; apps that are already open may keep it until they restart.'
}

if ($RemoveFiles) {
    Write-Step "Delete the version folders in $TipVersionsDir"
    $kept = @(Remove-TipVersionDirs $dirs)
    if ((Test-Path -LiteralPath $TipVersionsDir) -and -not (Get-ChildItem -LiteralPath $TipVersionsDir -Force)) {
        Remove-Item -LiteralPath $TipVersionsDir
    }
    if ($kept.Count -gt 0) {
        Write-Warn "$($kept.Count) folder(s) kept because an app still has the DLL loaded (see tasklist /m $TipDllName)."
        Write-Note 'Close those apps, or sign out and in, then run this script with -RemoveFiles again.'
    }
}
Write-Step 'Done. Check with .\status.ps1'
