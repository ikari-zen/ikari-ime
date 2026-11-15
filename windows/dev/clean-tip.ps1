<#
.SYNOPSIS
  Deletes the TIP version folders OLDER than the registered one ($TipVersionsDir\<time>\, made by register.ps1).
  Keeps the registered version and any newer folder. Does not touch the registration.
.PARAMETER DryRun
  Show the plan only.
.PARAMETER Force
  Do not ask for confirmation.
.NOTES
  A folder whose DLL an app still has loaded (explorer and Windows Terminal usually do) cannot be deleted: it is
  kept whole and reported. Close those apps, or sign out and in, then run this script again. No delete at the next
  reboot is scheduled (MOVEFILE_DELAY_UNTIL_REBOOT needs an administrator; that is the installer's job, ADR 0063 s3-4).
  A kept folder does no harm: nothing points to it, so apps started from now on do not load it.
  No administrator needed (the folders are under %LOCALAPPDATA%, and inherit its permissions).

#>
param([switch]$DryRun, [switch]$Force)

. "$PSScriptRoot\common.ps1"

$registered = Get-RegisteredTipDll
if (-not $registered) {
    throw 'The TIP is not registered, so no version is "current". To delete every version folder, run .\unregister.ps1 -RemoveFiles.'
}
$current = Get-TipVersionName $registered
if (-not $current) {
    throw "The registered DLL is not one made by register.ps1: $registered. Nothing deleted (delete the folders by hand if you are sure)."
}

$all = @(Get-TipVersionDirs)
$older = @($all | Where-Object { [string]::CompareOrdinal($_.Name, $current) -lt 0 })
$newer = @($all | Where-Object { [string]::CompareOrdinal($_.Name, $current) -gt 0 })

Write-Step 'Plan'
Write-Note "registered (kept): $current"
foreach ($d in $newer) { Write-Note "newer than the registered one (kept): $($d.Name)" }
if ($older.Count -eq 0) { Write-Step 'No older version folder. Nothing to do.'; return }
foreach ($d in $older) { Write-Note "delete $($d.FullName)   (kept if an app has its DLL loaded)" }
if ($DryRun) { Write-Step 'Dry run: nothing was changed.'; return }
if (-not (Confirm-Action 'Go ahead?' $Force.IsPresent)) { Write-Step 'Cancelled.'; return }

$kept = @(Remove-TipVersionDirs $older)
if ($kept.Count -gt 0) {
    Write-Warn "$($kept.Count) old folder(s) kept because an app still has the DLL loaded (see tasklist /m $TipDllName)."
    Write-Note 'Close those apps, or sign out and in, then run this script again.'
}
Write-Step 'Done. Check with .\status.ps1'
