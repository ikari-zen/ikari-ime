using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;

namespace IkariSettings;

/// <summary>
/// 核（ikari_settings_core.dll）の C の ABI（windows/settings/include/ikari_settings_core.h と同じに保つ）。
/// 判断は核が持つ。ここは呼び出しを写すだけ。
/// </summary>
internal static unsafe partial class CoreNative
{
    private const string Dll = "ikari_settings_core.dll";

    /// <summary>この画面が合わせた ABI の版（IKARI_SETTINGS_CORE_ABI_VERSION）</summary>
    public const uint AbiVersion = 1;
    public const int Ok = 0;

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

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_specs")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial int Specs(Buf* output);

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_buf_free")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial void BufFree(Buf buf);

    /// <summary>設定の一覧（TSV）。取れなければ null。</summary>
    public static string? SpecsTsv()
    {
        Buf buf = default;
        if (Specs(&buf) != Ok)
        {
            return null;
        }
        try
        {
            return buf.Ptr == null ? string.Empty : Encoding.UTF8.GetString(buf.Ptr, checked((int)buf.Len));
        }
        finally
        {
            BufFree(buf);
        }
    }
}
