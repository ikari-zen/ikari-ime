using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace IkariCandidate;

/// <summary>
/// 核（ikari_candidate.dll）の C の ABI（windows/candidate/include/ikari_candidate.h と同じに保つ）。
/// 判断は核が持つ。ここは呼び出しを写すだけ。
/// </summary>
internal static unsafe partial class CoreNative
{
    private const string Dll = "ikari_candidate.dll";

    /// <summary>この画面が合わせた ABI の版（IKARI_CANDIDATE_ABI_VERSION）</summary>
    public const uint AbiVersion = 1;
    public const int Ok = 0;
    public const int SideBelow = 1;
    public const int SideAbove = 2;

    [LibraryImport(Dll, EntryPoint = "ikari_candidate_abi_version")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static partial uint GetAbiVersion();

    /// <summary>核を起動する。失敗（パイプを取れない、高い権限など）は IntPtr.Zero。</summary>
    [LibraryImport(Dll, EntryPoint = "ikari_candidate_start")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static partial IntPtr Start(delegate* unmanaged[Cdecl]<byte*, nuint, IntPtr, void> callback, IntPtr userData);

    [LibraryImport(Dll, EntryPoint = "ikari_candidate_stop")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static partial int Stop(IntPtr core);

    [LibraryImport(Dll, EntryPoint = "ikari_candidate_panel_height_dip")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static partial double PanelHeightDip(
        uint candidateRows,
        [MarshalAs(UnmanagedType.U1)] bool hasAnnotation,
        [MarshalAs(UnmanagedType.U1)] bool hasDeveloperNote);

    [LibraryImport(Dll, EntryPoint = "ikari_candidate_horizontal_origin")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static partial double HorizontalOrigin(double anchorX, double panelWidth, double workX, double workWidth);

    [LibraryImport(Dll, EntryPoint = "ikari_candidate_vertical_origin")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static partial double VerticalOrigin(
        double anchorY, double anchorHeight, double panelHeight, int side, double workY, double workHeight);
}
