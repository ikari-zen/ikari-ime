using System.Runtime.InteropServices;

namespace IkariCandidate;

/// <summary>窓の拡張スタイルと角丸（OS の入口だけ。スパイク #368 で確かめた作法）。x64 だけ。</summary>
[DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
internal static partial class Win32
{
    private const int GwlExStyle = -20;
    public const long WsExNoActivate = 0x08000000;
    public const long WsExToolWindow = 0x00000080;
    private const uint SwpNoSize = 0x1, SwpNoMove = 0x2, SwpNoZOrder = 0x4, SwpNoActivate = 0x10, SwpFrameChanged = 0x20;
    private const int DwmwaWindowCornerPreference = 33;
    private const int DwmwcpRoundSmall = 3;

    [LibraryImport("user32.dll", EntryPoint = "GetWindowLongPtrW")]
    private static partial IntPtr GetWindowLongPtr(IntPtr hWnd, int index);

    [LibraryImport("user32.dll", EntryPoint = "SetWindowLongPtrW")]
    private static partial IntPtr SetWindowLongPtr(IntPtr hWnd, int index, IntPtr value);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);

    [LibraryImport("dwmapi.dll")]
    private static partial int DwmSetWindowAttribute(IntPtr hWnd, int attr, ref int value, int size);

    /// <summary>拡張スタイルを足し、枠を描き直させる（前面にしない）。</summary>
    public static void AddExStyle(IntPtr hwnd, long add)
    {
        long ex = GetWindowLongPtr(hwnd, GwlExStyle).ToInt64();
        SetWindowLongPtr(hwnd, GwlExStyle, new IntPtr(ex | add));
        SetWindowPos(hwnd, IntPtr.Zero, 0, 0, 0, 0, SwpNoSize | SwpNoMove | SwpNoZOrder | SwpNoActivate | SwpFrameChanged);
    }

    /// <summary>小さい角丸を頼む（VM の中では角丸にならない設計。戻り値だけを見る。ADR 0062 §10 Q5）。</summary>
    public static int RoundSmallCorners(IntPtr hwnd)
    {
        int pref = DwmwcpRoundSmall;
        return DwmSetWindowAttribute(hwnd, DwmwaWindowCornerPreference, ref pref, sizeof(int));
    }
}
