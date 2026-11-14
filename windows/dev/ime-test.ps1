<#
.SYNOPSIS
  Drives a Notepad check of ikari without looking at the screen (UI Automation). Run it in the DESKTOP session via run-in-session.ps1.
  -Mode Prepare : launch Notepad if needed, focus + clear its edit control, activate the ikari profile (TSF, this session).
  -Mode Activate: only the activation of the ikari profile (+ open status), on the running Notepad.
  -Mode Release : switch back to Microsoft IME (the ikari TIP is deactivated, which also flushes its latency measurements).
  -Mode Read    : write the edit control's text, and the active profile, to $OutFile (UTF-8).
  Keys are sent from outside (virsh send-key on the host).
#>
param([ValidateSet('Prepare', 'Activate', 'Release', 'Read')][string]$Mode = 'Read', [string]$OutFile = 'C:\ikari-dev\ime-test.txt')

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
[ComImport, Guid("71C6E74C-0F28-11D8-A82A-00065B84435C"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface ITfInputProcessorProfileMgr {
    [PreserveSig] int ActivateProfile(uint profileType, ushort langid, ref Guid clsid, ref Guid profile, IntPtr hkl, uint flags);
    [PreserveSig] int DeactivateProfile(uint profileType, ushort langid, ref Guid clsid, ref Guid profile, IntPtr hkl, uint flags);
    [PreserveSig] int GetProfile(uint profileType, ushort langid, ref Guid clsid, ref Guid profile, IntPtr hkl, out TF_INPUTPROCESSORPROFILE p);
    [PreserveSig] int EnumProfiles(ushort langid, out IntPtr e);
    [PreserveSig] int ReleaseInputProcessor(ref Guid clsid, uint flags);
    [PreserveSig] int RegisterProfile(ref Guid clsid, ushort langid, ref Guid profile, [MarshalAs(UnmanagedType.LPWStr)] string desc, uint descLen, [MarshalAs(UnmanagedType.LPWStr)] string icon, uint iconLen, uint iconIndex, IntPtr hkl, uint flags, int enabled, uint hklSubstitute);
    [PreserveSig] int UnregisterProfile(ref Guid clsid, ushort langid, ref Guid profile, uint flags);
    [PreserveSig] int GetActiveProfile(ref Guid catid, out TF_INPUTPROCESSORPROFILE p);
}
[StructLayout(LayoutKind.Sequential)]
public struct TF_INPUTPROCESSORPROFILE {
    public uint dwProfileType; public ushort langid; public Guid clsid; public Guid guidProfile; public Guid catid;
    public IntPtr hklSubstitute; public uint dwCaps; public IntPtr hkl; public uint dwFlags;
}
public static class Tsf {
    static ITfInputProcessorProfileMgr Mgr() {
        var t = Type.GetTypeFromCLSID(new Guid("33C53A50-F456-4884-B049-85FD643ECFED"));
        return (ITfInputProcessorProfileMgr)Activator.CreateInstance(t);
    }
    public static int Activate(string clsid, string profile) {
        var c = new Guid(clsid); var p = new Guid(profile);
        // TF_PROFILETYPE_INPUTPROCESSOR = 1; TF_IPPMF_FORSESSION | TF_IPPMF_ENABLEPROFILE
        return Mgr().ActivateProfile(1, 0x0411, ref c, ref p, IntPtr.Zero, 0x20000000u | 1u);
    }
    public static string Active() {
        var cat = new Guid("34745C63-B2F0-4784-8B67-5E12C8701A31"); // GUID_TFCAT_TIP_KEYBOARD
        TF_INPUTPROCESSORPROFILE pr;
        int hr = Mgr().GetActiveProfile(ref cat, out pr);
        return hr.ToString("X8") + " " + pr.clsid + " " + pr.guidProfile;
    }
}
'@

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ImeCtl {
    [DllImport("imm32.dll")] static extern IntPtr ImmGetDefaultIMEWnd(IntPtr hwnd);
    [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    // WM_IME_CONTROL (0x283), IMC_SETOPENSTATUS (6) / IMC_GETOPENSTATUS (5): open = Japanese mode, closed = English (TIP mode.rs)
    public static long SetOpen(bool open) {
        IntPtr w = ImmGetDefaultIMEWnd(GetForegroundWindow());
        SendMessage(w, 0x283, (IntPtr)6, (IntPtr)(open ? 1 : 0));
        return (long)SendMessage(w, 0x283, (IntPtr)5, IntPtr.Zero);
    }
    public static long GetOpen() {
        IntPtr w = ImmGetDefaultIMEWnd(GetForegroundWindow());
        return (long)SendMessage(w, 0x283, (IntPtr)5, IntPtr.Zero);
    }
}
'@

$IkariClsid = '{83A140BE-DECB-4358-867B-7B77D6541276}'
$IkariProfile = '{35EAB916-5717-4CFA-8991-7264079FD1C1}'

function Get-NotepadDoc {
    $p = Get-Process notepad -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    if (-not $p) { return $null }
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $p.Id)
    $win = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
    $dc = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Document)
    return $win.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $dc)
}

$lines = New-Object System.Collections.Generic.List[string]
if ($Mode -eq 'Prepare') {
    # Win11 Notepad restores the previous tabs, so a plain "clear" is not enough: close it, drop its saved state, start fresh.
    Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep 1
    Get-ChildItem "$env:LOCALAPPDATA\Packages\Microsoft.WindowsNotepad_*\LocalState" -Recurse -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
    Start-Process notepad.exe | Out-Null
    Start-Sleep 3
    $doc = Get-NotepadDoc
    if (-not $doc) { $lines.Add('no notepad document') } else { $doc.SetFocus(); $lines.Add('fresh notepad') }
    }
if ($Mode -in 'Prepare','Activate') {
    $lines.Add('activate hr=' + ('{0:X8}' -f [Tsf]::Activate($IkariClsid, $IkariProfile)))
    Start-Sleep 1
    $lines.Add('open status=' + [ImeCtl]::SetOpen($true))
}
if ($Mode -eq 'Release') { $lines.Add('release hr=' + ('{0:X8}' -f [Tsf]::Activate('{03B5835F-F03C-411B-9CE2-AA23E1171E36}', '{A76C93D9-5523-4E90-AAFA-4DB112F9AC76}'))); Start-Sleep 1 }
$lines.Add('active: ' + [Tsf]::Active())
if ($Mode -eq 'Read') { $lines.Add('open now=' + [ImeCtl]::GetOpen()) }
$doc = Get-NotepadDoc
if ($doc) {
    $tp = $doc.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern)
    $lines.Add('text: [' + $tp.DocumentRange.GetText(-1) + ']')
    try { $doc.SetFocus() } catch { }   # give the focus back, so the next keys go to Notepad
}
$lines | Set-Content -Encoding UTF8 $OutFile
