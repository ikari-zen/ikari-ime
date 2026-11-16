using Microsoft.UI.Xaml;

namespace IkariSettings;

/// <summary>
/// ikari-settings.exe（ADR 0071 §2）。核がエンジンの設定の口につなぎ（相手を照らしてから）、画面は核が渡す行を描く。
/// 核の ABI の版が合わない、または高い権限で動いているときは、何もせずに終わる。通信しない（NFR-17）。
/// </summary>
public partial class App : Application
{
    /// <summary>核を使えない（ABI の版の違い、高い権限）</summary>
    private const int ExitCoreUnavailable = 2;
    /// <summary>扱えない例外</summary>
    private const int ExitUnhandled = 3;

    private SettingsWindow? _window;

    public App()
    {
        InitializeComponent();
        UnhandledException += (_, e) =>
        {
            e.Handled = true;
            Environment.Exit(ExitUnhandled);
        };
    }

    protected override async void OnLaunched(LaunchActivatedEventArgs args)
    {
        if (CoreNative.GetAbiVersion() != CoreNative.AbiVersion || CoreNative.CheckSelf() != CoreNative.Ok)
        {
            Environment.Exit(ExitCoreUnavailable);
            return;
        }
        // つながらなくても窓は出す（状態の欄に理由を出し、値は既定値で読むだけにする。ADR 0068 §7）
        _window = new SettingsWindow(new CoreSession());
        await _window.ShowAsync();
    }
}
