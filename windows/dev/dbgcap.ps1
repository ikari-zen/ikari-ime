<#
.SYNOPSIS
  Captures OutputDebugString of processes in THIS session (the TIP logs there) into a file, for $Seconds. No DebugView needed.
  Run it in the desktop session (run-in-session.ps1 starts it hidden). Appends one line per message, flushed at once.
#>
param([string]$OutFile = 'C:\ikari-dev\dbg.log', [int]$Seconds = 120)

Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Threading;
using System.IO.MemoryMappedFiles;
public static class DbgCap {
    public static void Run(string path, int seconds) {
        bool c1, c2;
        var ready = new EventWaitHandle(false, EventResetMode.AutoReset, "DBWIN_BUFFER_READY", out c1);
        var data = new EventWaitHandle(false, EventResetMode.AutoReset, "DBWIN_DATA_READY", out c2);
        using (var mmf = MemoryMappedFile.CreateOrOpen("DBWIN_BUFFER", 4096))
        using (var view = mmf.CreateViewAccessor(0, 4096))
        using (var w = new StreamWriter(path, true, new System.Text.UTF8Encoding(true))) {
            w.AutoFlush = true;
            w.WriteLine("capture started");
            var end = DateTime.UtcNow.AddSeconds(seconds);
            while (DateTime.UtcNow < end) {
                ready.Set();
                if (!data.WaitOne(500)) continue;
                int pid = view.ReadInt32(0);
                var b = new byte[4092];
                view.ReadArray(4, b, 0, 4092);
                int n = Array.IndexOf(b, (byte)0); if (n < 0) n = 4092;
                w.WriteLine(pid + " " + System.Text.Encoding.Default.GetString(b, 0, n).TrimEnd());
            }
            w.WriteLine("capture ended");
        }
    }
}
'@
[DbgCap]::Run($OutFile, $Seconds)
