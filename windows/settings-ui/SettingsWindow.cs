using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Storage.Pickers;

namespace IkariSettings;

/// <summary>
/// 設定の窓（W4-s3 の第 2 段。ADR 0071 §2）。上から、状態の欄（ADR 0068 §7）、結果の欄、学習の書き出しと取り込み、
/// 画面ごと・節ごとの設定。部品（スイッチ、数、選択肢、読むだけの文字）は核が決めた種類のまま出し、変えたら核に渡すだけ
/// （書き方、範囲への丸め、検査は核）。文字はすべて TextBlock などで文字として出す（リンクにしない。ADR 0071 §2.3）。
/// 文言の表（多言語化）と見た目の仕上げは次の段。
/// </summary>
internal sealed class SettingsWindow
{
    private const double PaddingDip = 24;
    private const double ScreenFontDip = 24;
    private const double SectionFontDip = 16;
    private const double SectionGapDip = 16;
    private const double RowGapDip = 4;
    private const double ControlWidthDip = 240;
    private const string WindowTitle = "ikari の設定";
    private const string ExportFileTypeLabel = "ikari の書き出し";
    private const string ExportFileExtension = ".tsv";

    private readonly Window _window;
    private readonly CoreSession _core;
    private readonly TextBlock _status = new() { TextWrapping = TextWrapping.Wrap };
    private readonly TextBlock _result = new() { TextWrapping = TextWrapping.Wrap };
    private readonly StackPanel _settings = new();
    private bool _busy;

    public SettingsWindow(CoreSession core)
    {
        _core = core;
        var reconnect = new Button { Content = "つなぎ直す" };
        reconnect.Click += async (_, _) => await ConnectAndRefreshAsync();
        var export = new Button { Content = "学習を書き出す…" };
        export.Click += async (_, _) => await ExportAsync();
        var import = new Button { Content = "学習を取り込む…" };
        import.Click += async (_, _) => await ImportAsync();
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        actions.Children.Add(reconnect);
        actions.Children.Add(export);
        actions.Children.Add(import);

        var root = new StackPanel { Padding = new Thickness(PaddingDip), Spacing = RowGapDip };
        root.Children.Add(_status);
        root.Children.Add(actions);
        root.Children.Add(_result);
        // 第 4 弾の契約: レーン D-4 はモデルの欄（取得、ファイルからの取り込み、自由指定の確認の窓）を ModelPanel.cs に作り、ここに 1 行で足す
        root.Children.Add(_settings);
        // 第 4 弾の契約: レーン D-2d は版の確認の欄を VersionPanel.cs に作り、ここに 1 行で足す
        // 画面の入力欄で ikari を使えるかを VM で確かめるための欄（ADR 0071 §6。保存しない）
        root.Children.Add(new TextBox { Header = "入力の確認用（保存しない）", Margin = new Thickness(0, SectionGapDip, 0, 0) });
        _window = new Window { Title = WindowTitle, Content = new ScrollViewer { Content = root } };
        _window.Closed += (_, _) => _core.Dispose();
    }

    public async Task ShowAsync()
    {
        _window.Activate();
        await ConnectAndRefreshAsync();
    }

    private async Task ConnectAndRefreshAsync()
    {
        await _core.ConnectAsync();
        await RefreshAsync();
    }

    private async Task RefreshAsync()
    {
        var (rows, message) = await _core.ViewAsync();
        _status.Text = _core.StatusText;
        if (message.Length > 0)
        {
            _result.Text = message;
        }
        Render(rows);
    }

    private void Render(IReadOnlyList<ViewRow> rows)
    {
        _busy = true;
        _settings.Children.Clear();
        bool editable = _core.IsConnected;
        string? screen = null;
        string? section = null;
        foreach (ViewRow row in rows)
        {
            if (row.Screen != screen)
            {
                screen = row.Screen;
                section = null;
                _settings.Children.Add(new TextBlock { Text = row.Screen, FontSize = ScreenFontDip, Margin = new Thickness(0, SectionGapDip, 0, 0) });
            }
            if (row.Section != section)
            {
                section = row.Section;
                _settings.Children.Add(new TextBlock { Text = row.Section, FontSize = SectionFontDip, Margin = new Thickness(0, SectionGapDip, 0, 0) });
            }
            _settings.Children.Add(RowControl(row, editable));
        }
        _busy = false;
    }

    /// <summary>1 行の部品。値を入れてから知らせを付ける（入れたことで書き換えを送らない）</summary>
    private FrameworkElement RowControl(ViewRow row, bool editable)
    {
        switch (row.Widget)
        {
            case "bool":
                var toggle = new ToggleSwitch { Header = row.Key, IsOn = row.Value == "1", IsEnabled = editable };
                toggle.Toggled += async (_, _) => await EditAsync(row.Key, CoreNative.WidgetBool, toggle.IsOn ? 1 : 0, string.Empty);
                return toggle;
            case "int" when double.TryParse(row.Min, out double min) && double.TryParse(row.Max, out double max):
                var number = new NumberBox
                {
                    Header = row.Key,
                    Minimum = min,
                    Maximum = max,
                    Value = double.TryParse(row.Value, out double v) ? v : min,
                    SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline,
                    Width = ControlWidthDip,
                    HorizontalAlignment = HorizontalAlignment.Left,
                    IsEnabled = editable,
                };
                number.ValueChanged += async (_, e) =>
                {
                    if (!double.IsNaN(e.NewValue))
                    {
                        await EditAsync(row.Key, CoreNative.WidgetInt, (long)Math.Round(e.NewValue), string.Empty);
                    }
                };
                return number;
            case "choice":
                var combo = new ComboBox { Header = row.Key, ItemsSource = row.Choices, SelectedItem = row.Value, Width = ControlWidthDip, IsEnabled = editable };
                combo.SelectionChanged += async (_, _) =>
                {
                    if (combo.SelectedItem is string choice)
                    {
                        await EditAsync(row.Key, CoreNative.WidgetChoice, 0, choice);
                    }
                };
                return combo;
            default:
                // この段では読むだけ（Bundle ID の並び、キー割り当て）
                return new TextBlock { Text = $"{row.Key} = {row.Value}", TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };
        }
    }

    private async Task EditAsync(string key, int widget, long number, string choice)
    {
        if (_busy)
        {
            return;
        }
        string message = await _core.EditAsync(key, widget, number, choice);
        if (message.Length > 0)
        {
            _result.Text = message;
        }
        await RefreshAsync();
    }

    private void InitializePicker(object picker)
    {
        IntPtr hwnd = WinRT.Interop.WindowNative.GetWindowHandle(_window);
        WinRT.Interop.InitializeWithWindow.Initialize(picker, hwnd);
    }

    /// <summary>書き出し先を選ばせて、パスだけを核に渡す（ADR 0068 §4）。上書きの確認は OS の保存の窓が済ませる</summary>
    private async Task ExportAsync()
    {
        var picker = new FileSavePicker { SuggestedFileName = Path.GetFileNameWithoutExtension(CoreNative.ExportFileName(DateTime.Now)) };
        picker.FileTypeChoices.Add(ExportFileTypeLabel, new List<string> { ExportFileExtension });
        InitializePicker(picker);
        var file = await picker.PickSaveFileAsync();
        if (file is null)
        {
            return;
        }
        // 保存の窓は、既にあるファイルを選んだときに上書きを確かめている（確かめたので置き換えてよい）
        bool replace = File.Exists(file.Path);
        _result.Text = await _core.ExportAsync(file.Path, replace);
    }

    /// <summary>取り込むファイルを選ばせ、先に確かめて件数を見せ、確認のあとに取り込む</summary>
    private async Task ImportAsync()
    {
        var picker = new FileOpenPicker();
        picker.FileTypeFilter.Add(ExportFileExtension);
        InitializePicker(picker);
        var file = await picker.PickSingleFileAsync();
        if (file is null)
        {
            return;
        }
        var (ok, preview) = await _core.ImportAsync(file.Path, dryRun: true);
        _result.Text = preview;
        if (!ok)
        {
            return;
        }
        var dialog = new ContentDialog
        {
            Title = "学習を取り込みますか",
            Content = new TextBlock { Text = preview, TextWrapping = TextWrapping.Wrap },
            PrimaryButtonText = "取り込む",
            CloseButtonText = "やめる",
            DefaultButton = ContentDialogButton.Close,
            XamlRoot = _window.Content.XamlRoot,
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary)
        {
            return;
        }
        var (_, message) = await _core.ImportAsync(file.Path, dryRun: false);
        _result.Text = message;
        await RefreshAsync();
    }
}
