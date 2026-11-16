using Microsoft.UI.Xaml;

namespace IkariSettings;

/// <summary>
/// ikari-settings.exe（ADR 0071 §2）。第 1 段は、核が渡す設定の一覧を出すだけ。エンジンにはつながない（第 2 段）。
/// 核の ABI の版が合わない、または高い権限で動いているときは、何もせずに終わる。通信しない（NFR-17）。
/// </summary>
public partial class App : Application
{
    /// <summary>核を使えない（ABI の版の違い、高い権限、一覧を取れない）</summary>
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

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        if (CoreNative.GetAbiVersion() != CoreNative.AbiVersion || CoreNative.CheckSelf() != CoreNative.Ok)
        {
            Environment.Exit(ExitCoreUnavailable);
            return;
        }
        string? tsv = CoreNative.SpecsTsv();
        if (tsv is null)
        {
            Environment.Exit(ExitCoreUnavailable);
            return;
        }
        _window = new SettingsWindow(SpecList.Parse(tsv));
        _window.Show();
    }
}
