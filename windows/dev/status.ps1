<#
.SYNOPSIS
  Shows the state: engine process, pipes, registration, data files, and the tail of the logs. Read only.
.PARAMETER LogLines
  How many lines of each log to show (default 20).
.NOTES
  Does not open the pipes (listing only), does not change anything.
  The TIP writes its log with OutputDebugString (not a file): watch it with Sysinternals DebugView.
 
#>
param([int]$LogLines = 20)

. "$PSScriptRoot\common.ps1"

Write-Step 'Settings used by these scripts'
Write-Note "DataDir:        $DataDir"
Write-Note "Engine exe:     $EngineExe"
Write-Note "TIP DLL (copy): $TipDllInstalled"
Write-Note "User SID / session: $(Get-CurrentUserSid) / $(Get-CurrentSessionId)"

Write-Step 'Engine process'
$ours = @(Get-OurEngineProcesses)
if ($ours.Count -eq 0) { Write-Note 'Not running (this build).' }
foreach ($p in $ours) { Write-Note ("PID {0}  started {1}  working set {2:N0} MB" -f $p.Id, $p.StartTime, ($p.WorkingSet64 / 1MB)) }
$others = @(Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($EngineExeName)) -ErrorAction SilentlyContinue | Where-Object { $ours.Id -notcontains $_.Id })
foreach ($p in $others) { Write-Warn "Another $EngineExeName is running: PID $($p.Id) (not touched by these scripts)" }

Write-Step 'Named pipes (name only; not opened)'
$pipes = @(Get-IkariPipes)
if ($pipes.Count -eq 0) { Write-Note 'No ikari pipe.' }
foreach ($n in $pipes) { Write-Note "\\.\pipe\$n" }
Write-Note "expected engine pipe:   $(Get-PipeName $EnginePipePrefix)"
Write-Note "expected settings pipe: $(Get-PipeName $SettingsPipePrefix)"

Write-Step 'TIP registration (read only)'
$clsidKey = "Registry::HKEY_CLASSES_ROOT\CLSID\$TipClsid\InprocServer32"
if (Test-Path -LiteralPath $clsidKey) {
    $registered = (Get-ItemProperty -LiteralPath $clsidKey).'(default)'
    $model = (Get-ItemProperty -LiteralPath $clsidKey).ThreadingModel
    Write-Note "CLSID registered: $registered   ThreadingModel=$model"
    if (-not (Test-Path -LiteralPath $registered)) { Write-Warn 'The registered DLL file does not exist.' }
}
else { Write-Note 'CLSID not registered.' }

Write-Step 'Data files'
foreach ($rel in @('dict\grammar.bin', 'dict\system.dict', 'dict\vectors.bin', 'dict\emoji.tsv', 'dict\postal.tsv',
        'models\zenz-v3.2-small-Q5_K_M.gguf', 'settings.tsv', 'engine.lock')) {
    $path = Join-Path $DataDir $rel
    if (Test-Path -LiteralPath $path) { Write-Note ("present  {0,-40} {1,14:N0} bytes" -f $rel, (Get-Item -LiteralPath $path).Length) }
    else { Write-Note ("absent   {0}" -f $rel) }
}

foreach ($log in @('logs\start-failure.log', 'logs\engine.log')) {
    $path = Join-Path $DataDir $log
    Write-Step "tail $path"
    if (Test-Path -LiteralPath $path) { Get-Content -LiteralPath $path -Tail $LogLines } else { Write-Note '(none)' }
}
