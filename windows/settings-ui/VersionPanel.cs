using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace IkariSettings;

/// <summary>
/// 版の確認の欄（ADR 0076 §6）。ボタンを押したときだけ確かめる（自動では確かめない）。UI だけ: 通信は核が起こす
/// ikari-data（低い整合性の子）がする。比べ方と表示の文（新しい版があります・最新です・この版のほうが新しい）は核が決め、
/// ここは様子の文をそのまま出す。取得も導入もしない。文字はすべて TextBlock で文字として出す（リンクにしない）。
/// </summary>
internal sealed class VersionPanel
{
    private const double RowGapDip = 4;
    private const double SectionGapDip = 16;
    private const double SectionFontDip = 16;
    private const int PollIntervalMs = 300;
    private const string SectionTitle = "版の確認";
    private const string CheckLabel = "新しい版を確認する";
    private const string RunningText = "確かめています…";
    // 核が文を返せなかったとき（呼び出しそのものの失敗）の文
    private const string UnknownText = "確かめられませんでした";

    private readonly StackPanel _root = new() { Spacing = RowGapDip, Margin = new Thickness(0, SectionGapDip, 0, 0) };
    private readonly Button _check = new() { Content = CheckLabel };
    private readonly TextBlock _result = new() { TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };
    private readonly DispatcherTimer _timer = new() { Interval = TimeSpan.FromMilliseconds(PollIntervalMs) };
    private readonly object _lock = new();
    private IntPtr _fetch;
    private bool _disposed;

    public VersionPanel()
    {
        _root.Children.Add(new TextBlock { Text = SectionTitle, FontSize = SectionFontDip });
        _root.Children.Add(_check);
        _root.Children.Add(_result);
        _check.Click += async (_, _) => await StartAsync();
        _timer.Tick += (_, _) => Poll();
    }

    public FrameworkElement Element => _root;

    /// <summary>窓を閉じるとき。動いている確認があれば、核に返す（fetch_free が止める）</summary>
    public void Dispose()
    {
        _timer.Stop();
        lock (_lock)
        {
            _disposed = true;
            if (_fetch != IntPtr.Zero)
            {
                VersionNative.Free(_fetch);
                _fetch = IntPtr.Zero;
            }
        }
    }

    private async Task StartAsync()
    {
        if (_fetch != IntPtr.Zero)
        {
            return;
        }
        _check.IsEnabled = false;
        _result.Text = RunningText;
        var (code, handle, message) = await Task.Run(VersionNative.Start);
        if (code != CoreNative.Ok || handle == IntPtr.Zero)
        {
            _result.Text = message.Length > 0 ? message : UnknownText;
            _check.IsEnabled = true;
            return;
        }
        lock (_lock)
        {
            if (_disposed)
            {
                // await の間に窓が閉じた。戻ったハンドルはその場で核に返し、タイマーは始めない
                VersionNative.Free(handle);
                return;
            }
            _fetch = handle;
        }
        _timer.Start();
    }

    private void Poll()
    {
        if (_fetch == IntPtr.Zero)
        {
            _timer.Stop();
            return;
        }
        var (code, phase, text) = VersionNative.Poll(_fetch);
        if (code == CoreNative.Ok && phase == VersionNative.PhaseRunning)
        {
            return;
        }
        // 終わった（成功、失敗、取り消し）か、核の呼び出しが失敗した。文は核の言うとおり
        _timer.Stop();
        VersionNative.Free(_fetch);
        _fetch = IntPtr.Zero;
        _result.Text = code == CoreNative.Ok && text.Length > 0 ? text : UnknownText;
        _check.IsEnabled = true;
    }
}

/// <summary>版の確認の呼び出し（ikari_settings_core.h の「取得」の欄と同じに保つ）。写すだけ</summary>
internal static unsafe partial class VersionNative
{
    private const string Dll = "ikari_settings_core.dll";

    /// <summary>IKARI_SETTINGS_FETCH_PHASE_RUNNING</summary>
    public const int PhaseRunning = 0;

    [StructLayout(LayoutKind.Sequential)]
    private struct Status
    {
        public int Phase;
        public ulong Received;
        public ulong Total;
        public byte HasTotal;
    }

    [LibraryImport(Dll, EntryPoint = "ikari_settings_version_check_start")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial int VersionCheckStart(IntPtr* output, CoreNative.Buf* msg);

    [LibraryImport(Dll, EntryPoint = "ikari_settings_fetch_poll")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial int FetchPoll(IntPtr fetch, Status* status, CoreNative.Buf* text);

    [LibraryImport(Dll, EntryPoint = "ikari_settings_fetch_free")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial void FetchFree(IntPtr fetch);

    [LibraryImport(Dll, EntryPoint = "ikari_settings_core_buf_free")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static partial void BufFree(CoreNative.Buf buf);

    private static string Take(CoreNative.Buf buf)
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

    /// <summary>始める。Ok なら handle を Free で必ず返す。失敗の文は message</summary>
    public static (int Code, IntPtr Handle, string Message) Start()
    {
        IntPtr handle = IntPtr.Zero;
        CoreNative.Buf msg = default;
        int code = VersionCheckStart(&handle, &msg);
        return (code, handle, Take(msg));
    }

    /// <summary>様子（phase と、核が作った文）</summary>
    public static (int Code, int Phase, string Text) Poll(IntPtr fetch)
    {
        Status status = default;
        CoreNative.Buf text = default;
        int code = FetchPoll(fetch, &status, &text);
        return (code, status.Phase, Take(text));
    }

    public static void Free(IntPtr fetch) => FetchFree(fetch);
}
