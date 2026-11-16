using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace IkariCandidate;

/// <summary>
/// ikari-candidate.exe（ADR 0062 §3）。常駐し、核が受けた命令の「描く状態」を描く。起動と見張りは engine-host（L4）。
/// 核が受け付けを始められなければ（同じセッションの候補窓がもう動いている、高い権限、ABI の版の違い）、何もせずに終わる。
/// ログは出さない（候補の文字を残さない。診断の口は次の段）。
/// </summary>
public partial class App : Application
{
    /// <summary>核を起動できなかった</summary>
    private const int ExitCoreUnavailable = 2;
    /// <summary>扱えない例外</summary>
    private const int ExitUnhandled = 3;

    private CandidateWindow? _window;
    private CoreHost? _core;

    public App()
    {
        InitializeComponent();
        UnhandledException += (_, e) =>
        {
            e.Handled = true;
            Shutdown(ExitUnhandled);
        };
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        var window = new CandidateWindow();
        _window = window;
        _core = CoreHost.Start(DispatcherQueue.GetForCurrentThread(), window.Render);
        if (_core is null)
        {
            Shutdown(ExitCoreUnavailable);
        }
    }

    private void Shutdown(int code)
    {
        _core?.Dispose();
        _core = null;
        Environment.Exit(code);
    }
}
