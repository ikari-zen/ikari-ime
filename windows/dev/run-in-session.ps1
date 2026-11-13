<#
.SYNOPSIS
  Runs a .ps1 in the interactive desktop session (session 1) via a scheduled task, waits, and shows its output file.
  Needed because plain SSH runs in session 0: no windows, and the ikari pipes carry the session id.
.EXAMPLE
  .\run-in-session.ps1 -Script C:\ikari-dev\windows\dev\uia-probe.ps1 -OutFile C:\ikari-dev\uia-probe.txt
#>
param([Parameter(Mandatory)][string]$Script, [Parameter(Mandatory)][string]$OutFile, [int]$TimeoutSec = 60, [string]$ScriptArgs = '')

$task = 'ikari-dev-run-' + [guid]::NewGuid().ToString('N').Substring(0, 8)   # a new name each time: a running task would be replaced otherwise
Remove-Item -LiteralPath $OutFile -ErrorAction SilentlyContinue
$cmd = "powershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$Script`" $ScriptArgs"
schtasks /Create /F /TN $task /SC ONCE /ST 00:00 /RL LIMITED /IT /RU $env:USERNAME /TR $cmd 2>&1 | Out-Null
schtasks /Run /TN $task 2>&1 | Out-Null
$deadline = (Get-Date).AddSeconds($TimeoutSec)
while (-not (Test-Path -LiteralPath $OutFile) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
if (Test-Path -LiteralPath $OutFile) { Start-Sleep -Milliseconds 500; Get-Content -LiteralPath $OutFile -Encoding UTF8 } else { Write-Warning "no output within $TimeoutSec s" }
schtasks /Delete /F /TN $task 2>&1 | Out-Null
