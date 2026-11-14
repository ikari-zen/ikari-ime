<#
.SYNOPSIS
  Probe for the candidate window spike (windows/candidate-spike, ADR 0062 section 8 / 9.2). Reads, without looking at the screen:
  whether the window exists, its rect / ex-style, which window is in the foreground and has the keyboard focus.
  Must run in the DESKTOP session (session 1); use run-in-session.ps1 to start it there. Writes the result to $OutFile.
.PARAMETER ProcessName
  Process name of the spike exe (without .exe).
.PARAMETER ExpectForeground
  Process name that should still hold the foreground and focus (for example notepad). PASS/FAIL line is written.
.PARAMETER Watch
  Seconds to keep sampling (once per second). 0 = one sample.
#>
param(
    [string]$ProcessName = 'candidate-spike',
    [string]$ExpectForeground = '',
    [int]$Watch = 0,
    [string]$OutFile = 'C:\ikari-dev\candidate-probe.txt'
)

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class CandProbe {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct GTI {
        public int cb; public uint flags; public IntPtr active, focus, capture, menu, moveSize, caret; public RECT rc;
    }
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p, IntPtr l);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] static extern IntPtr GetWindowLongPtr(IntPtr h, int i);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint tid, ref GTI i);
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr h);

    public static string[] TopLevel(uint[] pids) {
        var list = new List<string>();
        var set = new HashSet<uint>(pids);
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (!set.Contains(pid)) return true;
            RECT r; GetWindowRect(h, out r);
            var cls = new StringBuilder(256); GetClassName(h, cls, 256);
            long ex = GetWindowLongPtr(h, -20).ToInt64();
            list.Add(string.Format("hwnd=0x{0:X} pid={1} class={2} visible={3} rect=({4},{5})-({6},{7}) size={8}x{9} dpi={10} exstyle=0x{11:X} noactivate={12} topmost={13} toolwindow={14}",
                h.ToInt64(), pid, cls, IsWindowVisible(h), r.L, r.T, r.R, r.B, r.R - r.L, r.B - r.T, GetDpiForWindow(h), ex,
                (ex & 0x08000000) != 0, (ex & 0x8) != 0, (ex & 0x80) != 0));
            return true;
        }, IntPtr.Zero);
        return list.ToArray();
    }
    // returns: foreground hwnd, its pid, GUI thread focus hwnd, focus pid
    public static long[] Focus() {
        IntPtr fg = GetForegroundWindow();
        uint pid; uint tid = GetWindowThreadProcessId(fg, out pid);
        var g = new GTI(); g.cb = Marshal.SizeOf(typeof(GTI));
        GetGUIThreadInfo(tid, ref g);
        uint fpid = 0; if (g.focus != IntPtr.Zero) GetWindowThreadProcessId(g.focus, out fpid);
        return new long[] { fg.ToInt64(), pid, g.focus.ToInt64(), fpid };
    }
}
'@

$out = New-Object System.Collections.Generic.List[string]
$out.Add("session=" + (Get-Process -Id $PID).SessionId)
$deadline = (Get-Date).AddSeconds($Watch)
$failed = $false
do {
    $out.Add("--- " + (Get-Date).ToString('HH:mm:ss.fff'))
    $procs = @(Get-Process -Name $ProcessName -ErrorAction SilentlyContinue)
    if ($procs.Count -eq 0) {
        $out.Add("window: process '$ProcessName' not running")
    } else {
        $pids = [uint32[]]($procs | ForEach-Object { $_.Id })
        $wins = [CandProbe]::TopLevel($pids)
        $out.Add("window: top-level windows of $ProcessName = $($wins.Count)")
        foreach ($w in $wins) { $out.Add("  $w") }
        # UI Automation: what the accessibility tree sees for the same process (name text is not logged, only its length)
        $root = [System.Windows.Automation.AutomationElement]::RootElement
        $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, [int]$procs[0].Id)
        $uia = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
        $out.Add("uia: top-level elements=$($uia.Count)")
        foreach ($e in $uia) {
            $r = $e.Current.BoundingRectangle
            $out.Add(("  uia class={0} offscreen={1} rect=({2:F0},{3:F0}) {4:F0}x{5:F0} has_focus={6}" -f $e.Current.ClassName, $e.Current.IsOffscreen, $r.X, $r.Y, $r.Width, $r.Height, $e.Current.HasKeyboardFocus))
        }
    }
    $f = [CandProbe]::Focus()
    $fgName = (Get-Process -Id $f[1] -ErrorAction SilentlyContinue).ProcessName
    $focusName = if ($f[3] -ne 0) { (Get-Process -Id $f[3] -ErrorAction SilentlyContinue).ProcessName } else { '' }
    $out.Add("focus: foreground=0x{0:X} proc={1}  keyboard_focus=0x{2:X} proc={3}" -f $f[0], $fgName, $f[2], $focusName)
    if ($ExpectForeground) {
        $ok = ($fgName -eq $ExpectForeground) -and ($focusName -eq $ExpectForeground)
        if (-not $ok) { $failed = $true }
        $out.Add("expect: foreground and focus are '$ExpectForeground' => " + $(if ($ok) { 'PASS' } else { 'FAIL' }))
    }
    if ((Get-Date) -ge $deadline) { break }
    Start-Sleep -Seconds 1
} while ($true)

$dir = Split-Path -Parent $OutFile
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
$out | Set-Content -Encoding UTF8 $OutFile
$out | ForEach-Object { Write-Output $_ }
if ($failed) { exit 1 }
