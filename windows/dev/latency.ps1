<#
.SYNOPSIS
  Reads the TIP latency log and prints a table: median and p95 per process and kind. Read only (S1, issue #334).
.PARAMETER Last
  How many lines from the end of the log to read (default 2000).
.PARAMETER Kind
  Show only one kind: request, key or first_connect (default: all).
.NOTES
  Log: <DataDir>\logs\tip-latency.log. The TIP writes it only while measuring is on:
    create <DataDir>\latency.on (an empty file) and restart the app, or set IKARI_TIP_LATENCY=1 before the app starts.
  Each line is a summary of the newest 1000 samples of one process and one kind, so only the LAST line of each
  (pid, kind) pair is shown. Medians of different processes cannot be merged, so they are listed one by one.
  Times are in microseconds in the log and in milliseconds here. The log has no input text (numbers only).
 
#>
param(
    [int]$Last = 2000,
    [ValidateSet('all', 'request', 'key', 'first_connect')][string]$Kind = 'all'
)

. "$PSScriptRoot\common.ps1"

$path = Join-Path $DataDir 'logs\tip-latency.log'
$marker = Join-Path $DataDir 'latency.on'

Write-Step 'Measuring switch'
if (Test-Path -LiteralPath $marker) { Write-Note "ON  (found $marker)" }
else { Write-Note "off (no $marker; IKARI_TIP_LATENCY=1 in the app's environment also turns it on)" }

Write-Step "Latest summary per process and kind ($path)"
if (-not (Test-Path -LiteralPath $path)) { Write-Note '(no log yet)'; return }

# key=value fields of one line -> hashtable
function ConvertFrom-LatencyLine([string]$Line) {
    $fields = @{}
    foreach ($part in $Line.Split(' ')) {
        $i = $part.IndexOf('=')
        if ($i -gt 0) { $fields[$part.Substring(0, $i)] = $part.Substring($i + 1) }
    }
    return $fields
}

$latest = @{}
foreach ($line in @(Get-Content -LiteralPath $path -Tail $Last)) {
    $f = ConvertFrom-LatencyLine $line
    if (-not ($f.ContainsKey('pid') -and $f.ContainsKey('kind') -and $f.ContainsKey('p50_us'))) { continue }
    if ($Kind -ne 'all' -and $f['kind'] -ne $Kind) { continue }
    $latest["$($f['pid'])/$($f['kind'])"] = $f
}
if ($latest.Count -eq 0) { Write-Note '(no matching line)'; return }

$rows = foreach ($f in $latest.Values) {
    $app = ''
    try { $app = (Get-Process -Id ([int]$f['pid']) -ErrorAction Stop).ProcessName } catch { $app = '(ended)' }
    [pscustomobject]@{
        Pid    = [int]$f['pid']
        App    = $app
        Kind   = $f['kind']
        N      = [int64]$f['n']
        Window = [int]$f['window']
        Err    = [int64]$f['err']
        MinMs  = [math]::Round([double]$f['min_us'] / 1000, 2)
        P50Ms  = [math]::Round([double]$f['p50_us'] / 1000, 2)
        P95Ms  = [math]::Round([double]$f['p95_us'] / 1000, 2)
        MaxMs  = [math]::Round([double]$f['max_us'] / 1000, 2)
    }
}
$rows | Sort-Object Kind, Pid | Format-Table -AutoSize

Write-Note 'request = pipe round trip + engine work (all request kinds mixed); key = TIP inside (includes request); first_connect = first attempt to first success.'
Write-Note 'Target (NFR-02): Space to the first candidate within 16 ms. The window of 1000 samples shows p95, not p99.'
