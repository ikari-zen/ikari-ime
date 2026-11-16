using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;

namespace IkariSettings;

/// <summary>
/// 核（ikari_settings_core.dll）の C の ABI（windows/settings/include/ikari_settings_core.h と同じに保つ）。
/// 判断は核が持つ。ここは呼び出しを写すことと、文字列を UTF-8 のバイト列にすることだけ。
/// </summary>
internal static unsafe partial class CoreNative
{
    private const string Dll = "ikari_settings_core.dll";

    /// <summary>この画面が合わせた ABI の版（IKARI_SETTINGS_CORE_ABI_VERSION）</summary>
    public const uint AbiVersion = 1;
    public const int Ok = 0;
    public const int ErrDisconnected = -5;
    public const int WidgetBool = 1;
    public const int WidgetInt = 2;
    public const int WidgetChoice = 3;

    [StructLayout(LayoutKind.Sequential)]
    public struct Buf
    {
        public byte* Ptr;
        public nuint Len;
    }

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_abi_version")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static partial uint GetAbiVersion();

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_check_self")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static partial int CheckSelf();

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_buf_free")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial void BufFree(Buf buf);

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_open")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static partial IntPtr Open(int* outStatus);

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_close")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static partial void Close(IntPtr session);

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_status_text")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial int StatusText(int status, Buf* output);

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_view")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial int View(IntPtr session, Buf* output, Buf* msg);

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_edit")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial int Edit(IntPtr session, byte* key, nuint keyLen, int widget, long number, byte* choice, nuint choiceLen, Buf* msg);

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_reset")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial int Reset(IntPtr session, byte* key, nuint keyLen, Buf* msg);

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_export")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial int Export(IntPtr session, byte* path, nuint pathLen, [MarshalAs(UnmanagedType.U1)] bool replace, Buf* msg);

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_import")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial int Import(IntPtr session, byte* path, nuint pathLen, [MarshalAs(UnmanagedType.U1)] bool dryRun, Buf* msg);

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_export_file_name")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial int ExportFileName(uint year, uint month, uint day, Buf* output);

    /// <summary>核が返したバイト列を文字列にして返す（null は空）</summary>
    private static string Take(Buf buf)
    {
        try
        {
            return buf.Ptr == null ? string.Empty : Encoding.UTF8.GetString(buf.Ptr, checked((int)buf.Len));
        }
        finally
        {
            BufFree(buf);
        }
    }

    public static string StatusText(int status)
    {
        Buf buf = default;
        StatusText(status, &buf);
        return Take(buf);
    }

    /// <summary>画面の行（TSV）と、結果の文。session は IntPtr.Zero でよい（既定値）。</summary>
    public static (int Code, string Tsv, string Message) View(IntPtr session)
    {
        Buf output = default;
        Buf msg = default;
        int code = View(session, &output, &msg);
        return (code, Take(output), Take(msg));
    }

    public static (int Code, string Message) Edit(IntPtr session, string key, int widget, long number, string choice)
    {
        byte[] k = Encoding.UTF8.GetBytes(key);
        byte[] c = Encoding.UTF8.GetBytes(choice);
        Buf msg = default;
        int code;
        fixed (byte* kp = k)
        fixed (byte* cp = c)
        {
            code = Edit(session, kp, (nuint)k.Length, widget, number, cp, (nuint)c.Length, &msg);
        }
        return (code, Take(msg));
    }

    public static (int Code, string Message) Reset(IntPtr session, string key)
    {
        byte[] k = Encoding.UTF8.GetBytes(key);
        Buf msg = default;
        int code;
        fixed (byte* kp = k)
        {
            code = Reset(session, kp, (nuint)k.Length, &msg);
        }
        return (code, Take(msg));
    }

    public static (int Code, string Message) Export(IntPtr session, string path, bool replace)
    {
        byte[] p = Encoding.UTF8.GetBytes(path);
        Buf msg = default;
        int code;
        fixed (byte* pp = p)
        {
            code = Export(session, pp, (nuint)p.Length, replace, &msg);
        }
        return (code, Take(msg));
    }

    public static (int Code, string Message) Import(IntPtr session, string path, bool dryRun)
    {
        byte[] p = Encoding.UTF8.GetBytes(path);
        Buf msg = default;
        int code;
        fixed (byte* pp = p)
        {
            code = Import(session, pp, (nuint)p.Length, dryRun, &msg);
        }
        return (code, Take(msg));
    }

    public static string ExportFileName(DateTime localDate)
    {
        Buf buf = default;
        ExportFileName((uint)localDate.Year, (uint)localDate.Month, (uint)localDate.Day, &buf);
        return Take(buf);
    }
}
