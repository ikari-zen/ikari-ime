<#
.SYNOPSIS
  Probe: can UI Automation read Notepad's edit control? Must run in the DESKTOP session (session 1), not over plain SSH.
  Use run-in-session.ps1 to start it there. Writes the result to $OutFile.
#>
param([string]$OutFile = 'C:\ikari-dev\uia-probe.txt', [switch]$NoLaunch)

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$out = New-Object System.Collections.Generic.List[string]
$out.Add("session=" + (Get-Process -Id $PID).SessionId)
if (-not $NoLaunch) { Start-Process notepad.exe | Out-Null }
Start-Sleep 3
$p = Get-Process notepad -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { $out.Add('notepad not running'); $out | Set-Content -Encoding UTF8 $OutFile; return }
$out.Add("notepad pid=$($p.Id) title=$($p.MainWindowTitle) hwnd=$($p.MainWindowHandle)")
$root = [System.Windows.Automation.AutomationElement]::RootElement
$cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $p.Id)
$win = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
if (-not $win) { $out.Add('no UIA window for notepad'); $out | Set-Content -Encoding UTF8 $OutFile; return }
$out.Add("window: " + $win.Current.Name)
$all = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
foreach ($e in $all) {
    $out.Add(("  {0} | {1} | {2}" -f $e.Current.ControlType.ProgrammaticName, $e.Current.ClassName, $e.Current.Name))
}
$out | Set-Content -Encoding UTF8 $OutFile
