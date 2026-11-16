namespace IkariSettings;

/// <summary>
/// 核のつながり（ikari_settings_core_open の戻り値）を持つ。核の呼び出しはパイプを待つ（取り込みは 30 秒まで）ので、
/// UI のスレッドの外（Task.Run）で呼ぶ。判断は核。ここは呼ぶ順と、つながりを閉じることだけ。
/// </summary>
internal sealed class CoreSession : IDisposable
{
    private readonly object _lock = new();
    private IntPtr _session;

    /// <summary>今の状態の文（状態の欄）</summary>
    public string StatusText { get; private set; } = string.Empty;

    public bool IsConnected
    {
        get { lock (_lock) { return _session != IntPtr.Zero; } }
    }

    /// <summary>つなぐ（つなぎ直す）。前のつながりは閉じる。</summary>
    public unsafe Task ConnectAsync() => Task.Run(() =>
    {
        int status = 0;
        IntPtr opened = CoreNative.Open(&status);
        lock (_lock)
        {
            CloseLocked();
            _session = opened;
        }
        StatusText = CoreNative.StatusText(status);
    });

    private Task<T> Run<T>(Func<IntPtr, T> call) => Task.Run(() =>
    {
        lock (_lock)
        {
            return call(_session);
        }
    });

    /// <summary>つながりが切れたと核が言ったら、状態の欄をその文にする</summary>
    private void Observe(int code, string message)
    {
        if (code == CoreNative.ErrDisconnected)
        {
            StatusText = message;
        }
    }

    public async Task<(IReadOnlyList<ViewRow> Rows, string Message)> ViewAsync()
    {
        var (code, tsv, message) = await Run(s => CoreNative.View(s));
        Observe(code, message);
        return (ViewRow.Parse(tsv), message);
    }

    public async Task<string> EditAsync(string key, int widget, long number, string choice)
    {
        var (code, message) = await Run(s => s == IntPtr.Zero ? (CoreNative.ErrDisconnected, StatusText) : CoreNative.Edit(s, key, widget, number, choice));
        Observe(code, message);
        return message;
    }

    public async Task<string> ResetAsync(string key)
    {
        var (code, message) = await Run(s => s == IntPtr.Zero ? (CoreNative.ErrDisconnected, StatusText) : CoreNative.Reset(s, key));
        Observe(code, message);
        return message;
    }

    public async Task<string> ExportAsync(string path, bool replace)
    {
        var (code, message) = await Run(s => s == IntPtr.Zero ? (CoreNative.ErrDisconnected, StatusText) : CoreNative.Export(s, path, replace));
        Observe(code, message);
        return message;
    }

    /// <summary>取り込み。Ok は、取り込める（dryRun）・取り込んだ</summary>
    public async Task<(bool Ok, string Message)> ImportAsync(string path, bool dryRun)
    {
        var (code, message) = await Run(s => s == IntPtr.Zero ? (CoreNative.ErrDisconnected, StatusText) : CoreNative.Import(s, path, dryRun));
        Observe(code, message);
        return (code == CoreNative.Ok, message);
    }

    private void CloseLocked()
    {
        if (_session != IntPtr.Zero)
        {
            CoreNative.Close(_session);
            _session = IntPtr.Zero;
        }
    }

    public void Dispose()
    {
        lock (_lock)
        {
            CloseLocked();
        }
    }
}
