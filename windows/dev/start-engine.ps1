<#
.SYNOPSIS
  Starts or stops the engine (ikari-engine-host.exe, the one built by build.ps1).
.PARAMETER Action
  start (default), stop, or restart.
.PARAMETER Foreground
  start only: run the engine in THIS console (Ctrl+C here stops it; this is how to test Ctrl+C).
  Without it, the engine starts WITHOUT a console window when ikari-engine-host-hidden.exe exists (the CI artifact has it,
  #372): no window to close by mistake, no Ctrl+C. Otherwise (a source build) it starts in its own console window.
.PARAMETER Console
  start only: use the console exe (ikari-engine-host.exe) in its own console window, even if the no-console exe exists.
.PARAMETER Force
  stop/restart only: do not ask before killing.
.NOTES
  "stop" is a forced kill (Stop-Process = TerminateProcess), the same as a crash: learning is saved every
  5 seconds, the lock is released by the OS. For a normal stop, press Ctrl+C in the engine's console window.
  Only processes whose exe path is exactly the one built here are touched.
  The data directory is $DataDir (common.ps1). The engine needs dict\ there (run install-data.ps1 first).
 
#>
param(
    [ValidateSet('start', 'stop', 'restart')][string]$Action = 'start',
    [switch]$Foreground,
    [switch]$Console,
    [switch]$Force
)

. "$PSScriptRoot\common.ps1"

function Stop-OurEngine {
    $procs = @(Get-OurEngineProcesses)
    if ($procs.Count -eq 0) { Write-Note 'The engine is not running.'; return }
    foreach ($p in $procs) { Write-Note "will kill PID $($p.Id)  $EngineExe" }
    if (-not (Confirm-Action 'Kill it (forced, like a crash)?' $Force.IsPresent)) { Write-Step 'Cancelled.'; return }
    foreach ($p in $procs) { Stop-Process -Id $p.Id -Force }
    Write-Step 'Stopped.'
}

function Start-OurEngine {
    # Foreground and -Console need the console exe. Otherwise prefer the no-console exe when there is one.
    $exe = $EngineExe
    if (-not $Foreground -and -not $Console -and (Test-Path -LiteralPath $EngineHiddenExe)) { $exe = $EngineHiddenExe }
    if (-not (Test-Path -LiteralPath $exe)) { throw "Not built: $exe. Run .\build.ps1 first." }
    if (-not (Test-Path -LiteralPath (Join-Path $DataDir 'dict\system.dict'))) {
        throw "No dictionary in $DataDir\dict. Run .\install-data.ps1 first."
    }
    $running = @(Get-OurEngineProcesses)
    if ($running.Count -gt 0) { Write-Note "Already running (PID $($running[0].Id))."; return }

    if ($DataDir -ne $DefaultDataDir -and -not $env:IKARI_USER_DIR) {
        Write-Warn 'DataDir differs from the default but IKARI_USER_DIR is not set. The TIP in other apps will not see it.'
    }
    Write-Step "$exe --data-dir $DataDir"
    if ($Foreground) {
        & $exe --data-dir $DataDir
        Write-Note "engine exited with code $LASTEXITCODE (0 = normal or already running, 2 = could not start, 64 = bad arguments)"
        return
    }
    $proc = Start-Process -FilePath $exe -ArgumentList @('--data-dir', "`"$DataDir`"") -WorkingDirectory $ReleaseDir -PassThru
    # wait up to 10 seconds for the pipe (opening a 600 MB dictionary can take a while: not measured)
    $pipe = Get-PipeName $EnginePipePrefix
    $deadline = (Get-Date).AddSeconds(10)
    while ((Get-Date) -lt $deadline) {
        if ($proc.HasExited) { break }
        if (@(Get-IkariPipes) -contains $pipe) { break }
        Start-Sleep -Milliseconds 200
    }
    if ($proc.HasExited) {
        Write-Warn "The engine exited right away (exit code $($proc.ExitCode); 0 = another engine already holds the lock, 2 = could not start)."
        $failure = Join-Path $DataDir 'logs\start-failure.log'
        if (Test-Path -LiteralPath $failure) { Write-Note "--- $failure"; Get-Content -LiteralPath $failure -Tail 20 }
        return
    }
    if (@(Get-IkariPipes) -contains $pipe) { Write-Step "Running (PID $($proc.Id)); pipe $pipe exists." }
    else { Write-Warn "Running (PID $($proc.Id)) but the pipe is not there yet. Check .\status.ps1 in a few seconds." }
}

switch ($Action) {
    'start' { Start-OurEngine }
    'stop' { Stop-OurEngine }
    'restart' { Stop-OurEngine; Start-Sleep -Seconds 1; Start-OurEngine }
}
