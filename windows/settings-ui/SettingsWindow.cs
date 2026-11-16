using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace IkariSettings;

/// <summary>
/// 設定の窓（第 1 段の骨組み。ADR 0071 §2）。核が渡した一覧を、画面ごと・節ごとに、並びのまま出すだけ（読むだけ）。
/// 文字はすべて TextBlock で出す（リンクにしない。ADR 0071 §2.3）。値の表示と書き換え、文言の表は第 2 段。
/// </summary>
internal sealed class SettingsWindow
{
    private const double PaddingDip = 24;
    private const double ScreenFontDip = 24;
    private const double SectionFontDip = 16;
    private const double RowFontDip = 13;
    private const double SectionGapDip = 16;
    private const double RowIndentDip = 12;
    private const string WindowTitle = "ikari の設定";

    private readonly Window _window;

    public SettingsWindow(IReadOnlyList<SpecRow> rows)
    {
        var list = new StackPanel { Padding = new Thickness(PaddingDip) };
        string? screen = null;
        string? section = null;
        foreach (SpecRow row in rows)
        {
            if (row.Screen != screen)
            {
                screen = row.Screen;
                section = null;
                list.Children.Add(new TextBlock { Text = row.Screen, FontSize = ScreenFontDip });
            }
            if (row.Section != section)
            {
                section = row.Section;
                list.Children.Add(new TextBlock
                {
                    Text = row.Section,
                    FontSize = SectionFontDip,
                    Margin = new Thickness(0, SectionGapDip, 0, 0),
                });
            }
            list.Children.Add(new TextBlock
            {
                Text = $"{row.Key}  ({row.Kind})  = {row.Default}",
                FontSize = RowFontDip,
                Margin = new Thickness(RowIndentDip, 0, 0, 0),
                IsTextSelectionEnabled = true,
            });
        }
        // 第 1 段は入力欄を持たないが、画面の入力欄で ikari を使えるかを VM で確かめるための欄を 1 つ置く（ADR 0071 §6）
        list.Children.Add(new TextBox
        {
            Header = "入力の確認用（保存しない）",
            Margin = new Thickness(0, SectionGapDip, 0, 0),
        });
        _window = new Window
        {
            Title = WindowTitle,
            Content = new ScrollViewer { Content = list },
        };
    }

    public void Show() => _window.Activate();
}
