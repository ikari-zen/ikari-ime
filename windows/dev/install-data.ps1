<#
.SYNOPSIS
  Puts the dictionary and the model into the data directory ($DataDir in common.ps1:
  IKARI_USER_DIR, else %LOCALAPPDATA%\ikari-ime). The same layout as "cargo xtask data install".
.PARAMETER Src
  A folder laid out like the repository's data\ folder:
      <Src>\build\dict\grammar.bin, system.dict, vectors.bin   (required)
      <Src>\build\dict\emoji.tsv, postal.tsv                   (optional)
      <Src>\raw\zenz\zenz-v3.2-small-Q5_K_M.gguf               (optional; only used when the LM is on)
  Default: <repo>\data. Build them on a Mac (scripts/setup.sh --skip-app, or cargo xtask dict) and copy the
  folders over (see docs\windows-real-device-check.md, "Preparation").
.PARAMETER NoModel
  Do not copy the model.
.PARAMETER DryRun
  Show what would be done; copy nothing.
.PARAMETER Force
  Do not ask before replacing a file that differs from the one in Src.
.NOTES
  Only touches <DataDir>\dict\ and <DataDir>\models\. It never touches the learning files, user dictionary,
  profile or settings.tsv. A file with the same size and SHA-256 is skipped. A different file is written to
  "<name>.new" and then moved over (the engine must be stopped: Windows cannot replace a file that is mapped).
 
#>
param(
    [string]$Src = (Join-Path $PSScriptRoot '..\..\data'),
    [switch]$NoModel,
    [switch]$DryRun,
    [switch]$Force
)

. "$PSScriptRoot\common.ps1"

$ModelFileName = 'zenz-v3.2-small-Q5_K_M.gguf'
$RequiredDict = @('grammar.bin', 'system.dict', 'vectors.bin')
$OptionalDict = @('emoji.tsv', 'postal.tsv')

if (-not (Test-Path -LiteralPath $Src -PathType Container)) { throw "Src folder not found: $Src" }
$Src = (Resolve-Path -LiteralPath $Src).Path
$srcDict = Join-Path $Src 'build\dict'
$srcModel = Join-Path $Src ('raw\zenz\' + $ModelFileName)
$dstDict = Join-Path $DataDir 'dict'
$dstModels = Join-Path $DataDir 'models'

Write-Step 'Plan'
Write-Note "from: $Src"
Write-Note "to:   $DataDir   (dict\ and models\ only)"

foreach ($name in $RequiredDict) {
    if (-not (Test-Path -LiteralPath (Join-Path $srcDict $name) -PathType Leaf)) {
        throw "Missing $name in $srcDict. Build the dictionary on a Mac and copy data\build\dict here."
    }
}

# list of (source, destination) pairs
$jobs = @()
foreach ($name in ($RequiredDict + $OptionalDict)) {
    $s = Join-Path $srcDict $name
    if (Test-Path -LiteralPath $s -PathType Leaf) { $jobs += , @($s, (Join-Path $dstDict $name)) }
    else { Write-Note "(optional file not found, skipped: $name)" }
}
if (-not $NoModel) {
    if (Test-Path -LiteralPath $srcModel -PathType Leaf) { $jobs += , @($srcModel, (Join-Path $dstModels $ModelFileName)) }
    else { Write-Note "(model not found, skipped: $srcModel)" }
}

function Test-SameFile([string]$A, [string]$B) {
    if (-not (Test-Path -LiteralPath $B -PathType Leaf)) { return $false }
    if ((Get-Item -LiteralPath $A).Length -ne (Get-Item -LiteralPath $B).Length) { return $false }
    return ((Get-FileHash -LiteralPath $A -Algorithm SHA256).Hash -eq (Get-FileHash -LiteralPath $B -Algorithm SHA256).Hash)
}

$todo = @()
foreach ($j in $jobs) {
    $s = $j[0]; $d = $j[1]
    if (Test-SameFile $s $d) { Write-Note "same, skipped:    $d"; continue }
    if (Test-Path -LiteralPath $d) { Write-Note "REPLACE (differs): $d" } else { Write-Note "new:              $d" }
    $todo += , $j
}
if ($todo.Count -eq 0) { Write-Step 'Nothing to do.'; return }
if ($DryRun) { Write-Step 'Dry run: nothing was copied.'; return }

$running = @(Get-OurEngineProcesses)
if ($running.Count -gt 0) {
    throw "The engine is running (PID $($running[0].Id)). Stop it first: .\start-engine.ps1 -Action stop"
}

$replacing = @($todo | Where-Object { Test-Path -LiteralPath $_[1] })
if ($replacing.Count -gt 0) {
    if (-not (Confirm-Action "Replace $($replacing.Count) existing file(s) listed above?" $Force.IsPresent)) { Write-Step 'Cancelled.'; return }
}

foreach ($dir in @($dstDict, $dstModels)) {
    if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
}

foreach ($j in $todo) {
    $s = $j[0]; $d = $j[1]
    $tmp = "$d.new"
    Write-Step "copy $s"
    Copy-Item -LiteralPath $s -Destination $tmp -Force
    Move-Item -LiteralPath $tmp -Destination $d -Force
    Write-Note "-> $d"
}
Write-Step 'Data installed. Next: start-engine.ps1'
