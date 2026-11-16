<#
.SYNOPSIS
  Probe the candidate window (ikari-candidate.exe, ADR 0062 s9.2) without looking at the screen. Must run in the DESKTOP
  session (session 1), not over plain SSH: start it with run-in-session.ps1. Writes the result to $OutFile.
.DESCRIPTION
  For every ikari-candidate process: its exe path, parent (should be the engine), memory, and each top-level window of the
  process: visible, rectangle (physical pixels), DPI of the window, extended styles (TOPMOST, NOACTIVATE, TOOLWINDOW).
  Also the foreground window and its process (it must stay the app you type in, never ikari-candidate), and whether the
  candidate pipe exists. It does not read any text from the window (the candidates are what the user typed).
.PARAMETER OutFile
  Where to write the result (UTF-8).
.PARAMETER Repeat
  Probe this many times, Interval milliseconds apart (to catch the window while you type with virsh send-key).
.PARAMETER Interval
  Milliseconds between probes.
#>
param([string]$OutFile = 'C:\ikari-dev\candidate-probe.txt', [int]$Repeat = 1, [int]$Interval = 500)

. "$PSScriptRoot\common.ps1"

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class IkariProbe {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc, IntPtr lparam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] public static extern IntPtr GetWindowLongPtr(IntPtr hwnd, int index);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    public static List<IntPtr> WindowsOf(uint pid) {
        var found = new List<IntPtr>();
        EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid) found.Add(h); return true; }, IntPtr.Zero);
        return found;
    }
    // Per-Monitor V2 (physical pixels for the rectangles)
    public static void PerMonitor() { SetThreadDpiAwarenessContext(new IntPtr(-4)); }
}
'@

$GwlExStyle = -20
$ExTopmost = 0x8
$ExToolWindow = 0x80
$ExNoActivate = 0x08000000

[IkariProbe]::PerMonitor()
$out = New-Object System.Collections.Generic.List[string]
$out.Add("session=$((Get-Process -Id $PID).SessionId) time=$(Get-Date -Format o)")
$pipe = Get-PipeName $CandidatePipePrefix
$out.Add("candidate pipe $pipe exists=$(@(Get-IkariPipes) -contains $pipe)")

for ($i = 0; $i -lt $Repeat; $i++) {
    $out.Add("--- probe $i  $(Get-Date -Format 'HH:mm:ss.fff')")
    $fg = [IkariProbe]::GetForegroundWindow()
    $fgPid = 0
    [void][IkariProbe]::GetWindowThreadProcessId($fg, [ref]$fgPid)
    $fgName = (Get-Process -Id $fgPid -ErrorAction SilentlyContinue).ProcessName
    $out.Add("foreground hwnd=$fg pid=$fgPid name=$fgName")
    $procs = @(Get-CimInstance Win32_Process -Filter "Name = '$CandidateExeName'")
    if ($procs.Count -eq 0) { $out.Add('no ikari-candidate process') }
    foreach ($p in $procs) {
        $parent = (Get-Process -Id $p.ParentProcessId -ErrorAction SilentlyContinue).ProcessName
        $mem = Get-Process -Id $p.ProcessId -ErrorAction SilentlyContinue
        $out.Add(("process pid={0} parent={1}({2}) path={3} private={4:N0} ws={5:N0}" -f $p.ProcessId, $p.ParentProcessId, $parent,
                $p.ExecutablePath, $mem.PrivateMemorySize64, $mem.WorkingSet64))
        if ($p.ExecutablePath -and ($p.ExecutablePath -ine $DevCandidateExe)) { $out.Add("  NOTE: not the dev install copy ($DevCandidateExe)") }
        foreach ($h in [IkariProbe]::WindowsOf([uint32]$p.ProcessId)) {
            $r = New-Object IkariProbe+RECT
            [void][IkariProbe]::GetWindowRect($h, [ref]$r)
            $ex = [IkariProbe]::GetWindowLongPtr($h, $GwlExStyle).ToInt64()
            $out.Add(("  window hwnd={0} visible={1} rect={2},{3},{4},{5} size={6}x{7} dpi={8} topmost={9} noactivate={10} toolwindow={11} foreground={12}" -f
                    $h, [IkariProbe]::IsWindowVisible($h), $r.Left, $r.Top, $r.Right, $r.Bottom, ($r.Right - $r.Left), ($r.Bottom - $r.Top),
                    [IkariProbe]::GetDpiForWindow($h), (($ex -band $ExTopmost) -ne 0), (($ex -band $ExNoActivate) -ne 0),
                    (($ex -band $ExToolWindow) -ne 0), ($h -eq $fg)))
        }
    }
    if ($i -lt $Repeat - 1) { Start-Sleep -Milliseconds $Interval }
}
$out | Set-Content -Encoding UTF8 $OutFile
Write-Step "wrote $OutFile"
