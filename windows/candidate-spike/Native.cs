using System.Runtime.InteropServices;
using System.Text;

namespace IkariCandidateSpike;

/// <summary>Win32 の呼び出しと、ログ用の読み取り（フォーカス、Z 順、モニタと DPI）。x64 だけ。</summary>
internal static class Native
{
    public const int GwlExStyle = -20;
    public const long WsExNoActivate = 0x08000000;
    public const long WsExToolWindow = 0x00000080;
    public const long WsExTopmost = 0x00000008;
    private const uint SwpNoSize = 0x1, SwpNoMove = 0x2, SwpNoZOrder = 0x4, SwpNoActivate = 0x10, SwpFrameChanged = 0x20;
    private const uint GwHwndPrev = 3;
    private const uint MonitorDefaultToNearest = 2;
    private const int MdtEffectiveDpi = 0;
    private const int DwmwaWindowCornerPreference = 33;
    private const int DwmwaExtendedFrameBounds = 9;
    public const int DwmwcpRoundSmall = 3;
    private const int ZOrderLogLimit = 6;
    private const int TextBuf = 256;

    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; public override string ToString() => $"({Left},{Top})-({Right},{Bottom}) {Right - Left}x{Bottom - Top}"; }

    [StructLayout(LayoutKind.Sequential)]
    private struct GuiThreadInfo
    {
        public int cbSize; public uint flags;
        public IntPtr hwndActive, hwndFocus, hwndCapture, hwndMenuOwner, hwndMoveSize, hwndCaret;
        public Rect rcCaret;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Point { public int X, Y; }

    [StructLayout(LayoutKind.Sequential)]
    private struct MonitorInfo { public int cbSize; public Rect rcMonitor, rcWork; public uint dwFlags; }

    private delegate bool MonitorEnumProc(IntPtr hMonitor, IntPtr hdc, ref Rect rect, IntPtr data);

    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll")] private static extern bool GetGUIThreadInfo(uint idThread, ref GuiThreadInfo info);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetClassName(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetWindowText(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] private static extern IntPtr GetWindowLongPtr(IntPtr hWnd, int index);
    [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW")] private static extern IntPtr SetWindowLongPtr(IntPtr hWnd, int index, IntPtr value);
    [DllImport("user32.dll")] private static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr hWnd, out Rect rect);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr hWnd, uint cmd);
    [DllImport("user32.dll")] private static extern uint GetDpiForWindow(IntPtr hWnd);
    [DllImport("user32.dll")] private static extern IntPtr MonitorFromPoint(Point pt, uint flags);
    [DllImport("user32.dll")] private static extern bool GetMonitorInfo(IntPtr hMonitor, ref MonitorInfo info);
    [DllImport("user32.dll")] private static extern bool EnumDisplayMonitors(IntPtr hdc, IntPtr clip, MonitorEnumProc proc, IntPtr data);
    [DllImport("shcore.dll")] private static extern int GetDpiForMonitor(IntPtr hMonitor, int type, out uint dpiX, out uint dpiY);
    [DllImport("dwmapi.dll")] private static extern int DwmSetWindowAttribute(IntPtr hWnd, int attr, ref int value, int size);
    [DllImport("dwmapi.dll")] private static extern int DwmGetWindowAttribute(IntPtr hWnd, int attr, out int value, int size);
    [DllImport("dwmapi.dll")] private static extern int DwmGetWindowAttribute(IntPtr hWnd, int attr, out Rect value, int size);

    public static uint WindowDpi(IntPtr hwnd) => GetDpiForWindow(hwnd);

    public static string Describe(IntPtr hwnd)
    {
        if (hwnd == IntPtr.Zero) return "0x0";
        var cls = new StringBuilder(TextBuf);
        var title = new StringBuilder(TextBuf);
        GetClassName(hwnd, cls, TextBuf);
        GetWindowText(hwnd, title, TextBuf);
        GetWindowThreadProcessId(hwnd, out uint pid);
        string proc;
        try { proc = System.Diagnostics.Process.GetProcessById((int)pid).ProcessName; } catch (ArgumentException) { proc = "?"; }
        // タイトルは文書の名前などを含み得るので、長さだけ出す（ログに内容を残さない約束）
        return $"0x{hwnd.ToInt64():X} class={cls} proc={proc}({pid}) title_len={title.Length}";
    }

    /// <summary>S-3。前面の窓と、そのスレッドの focus / active / caret。</summary>
    public static void LogFocus(SpikeLog log, string tag, IntPtr self)
    {
        IntPtr fg = GetForegroundWindow();
        uint tid = GetWindowThreadProcessId(fg, out _);
        var gti = new GuiThreadInfo { cbSize = Marshal.SizeOf<GuiThreadInfo>() };
        bool ok = GetGUIThreadInfo(tid, ref gti);
        log.Write($"focus[{tag}] foreground={Describe(fg)} is_self={fg == self}");
        log.Write($"focus[{tag}] gui_thread_info ok={ok} active={Describe(gti.hwndActive)} focus={Describe(gti.hwndFocus)} caret={Describe(gti.hwndCaret)} flags=0x{gti.flags:X}");
    }

    /// <summary>窓の矩形、拡張スタイル、自分より上（前）の窓。</summary>
    public static void LogWindow(SpikeLog log, string tag, IntPtr hwnd)
    {
        GetWindowRect(hwnd, out Rect r);
        DwmGetWindowAttribute(hwnd, DwmwaExtendedFrameBounds, out Rect ext, Marshal.SizeOf<Rect>());
        long ex = GetWindowLongPtr(hwnd, GwlExStyle).ToInt64();
        log.Write($"window[{tag}] hwnd=0x{hwnd.ToInt64():X} visible={IsWindowVisible(hwnd)} rect={r} dwm_frame={ext} dpi={GetDpiForWindow(hwnd)} " +
                  $"exstyle=0x{ex:X} noactivate={(ex & WsExNoActivate) != 0} toolwindow={(ex & WsExToolWindow) != 0} topmost={(ex & WsExTopmost) != 0}");
        IntPtr above = hwnd;
        for (int i = 0; i < ZOrderLogLimit; i++)
        {
            above = GetWindow(above, GwHwndPrev);
            if (above == IntPtr.Zero) break;
            if (IsWindowVisible(above)) log.Write($"window[{tag}] above[{i}] {Describe(above)}");
        }
    }

    public static void AddExStyle(IntPtr hwnd, long add)
    {
        long ex = GetWindowLongPtr(hwnd, GwlExStyle).ToInt64();
        SetWindowLongPtr(hwnd, GwlExStyle, new IntPtr(ex | add));
        SetWindowPos(hwnd, IntPtr.Zero, 0, 0, 0, 0, SwpNoSize | SwpNoMove | SwpNoZOrder | SwpNoActivate | SwpFrameChanged);
    }

    /// <summary>S-5 の一部。角丸の指定の戻り値と、読み直した値。</summary>
    public static void RoundCorners(SpikeLog log, IntPtr hwnd)
    {
        int pref = DwmwcpRoundSmall;
        int set = DwmSetWindowAttribute(hwnd, DwmwaWindowCornerPreference, ref pref, sizeof(int));
        int get = DwmGetWindowAttribute(hwnd, DwmwaWindowCornerPreference, out int back, sizeof(int));
        log.Write($"corner set_hr=0x{set:X8} get_hr=0x{get:X8} readback={back} (want {DwmwcpRoundSmall}; VM では角丸は描かれない設計)");
    }

    /// <summary>S-1。全モニタの矩形、作業領域、DPI と、指定の点のモニタ。</summary>
    public static void LogMonitors(SpikeLog log, int x, int y)
    {
        int n = 0;
        EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, (IntPtr mon, IntPtr hdc, ref Rect rect, IntPtr data) =>
        {
            log.Write($"monitor[{n++}] {MonitorText(mon)}");
            return true;
        }, IntPtr.Zero);
        IntPtr at = MonitorFromPoint(new Point { X = x, Y = y }, MonitorDefaultToNearest);
        log.Write($"monitor_at_anchor ({x},{y}) {MonitorText(at)}");
    }

    private static string MonitorText(IntPtr mon)
    {
        var mi = new MonitorInfo { cbSize = Marshal.SizeOf<MonitorInfo>() };
        GetMonitorInfo(mon, ref mi);
        int hr = GetDpiForMonitor(mon, MdtEffectiveDpi, out uint dx, out uint dy);
        return $"hmon=0x{mon.ToInt64():X} rect={mi.rcMonitor} work={mi.rcWork} primary={(mi.dwFlags & 1) != 0} dpi={dx}x{dy} scale={dx / 96.0:F2} dpi_hr=0x{hr:X8}";
    }
}
