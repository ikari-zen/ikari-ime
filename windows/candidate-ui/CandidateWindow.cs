using Ikari.Candidate.V1;
using Ikari.Shell.V1;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.Graphics;

namespace IkariCandidate;

/// <summary>
/// 候補の窓（骨組み。ADR 0062 §6.2、§7.3）。前面にしない、最前面、タスクバーと Alt+Tab に出さない（スパイク #368 の S-3 で確かめた作法）。
/// 判断は持たない: 何を出すか、並べる順、上下、行の高さ、位置は核が決める。ここは XAML で描いて、核が返した位置に置くだけ。
/// 見た目（ライトとダーク、Acrylic、開発者の行の書式）は骨組みの外（windows/candidate/README.md の「次の段」）。
/// </summary>
internal sealed class CandidateWindow
{
    private const double CellPaddingDip = 8;
    private const double LabelGapDip = 6;
    private const double CandidateFontDip = 15;
    private const double SmallFontDip = 11;
    private const double PanelPaddingDip = 0;

    private readonly Window _window;
    private readonly AppWindow _appWindow;
    private readonly StackPanel _rows;
    private ulong _lastRevision;

    public CandidateWindow()
    {
        _rows = new StackPanel { Padding = new Thickness(PanelPaddingDip) };
        _window = new Window { Content = _rows };
        // Activate() は呼ばない（フォーカスを奪うため）。出すのは Show(false) だけ
        IntPtr hwnd = WinRT.Interop.WindowNative.GetWindowHandle(_window);
        _appWindow = AppWindow.GetFromWindowId(Win32Interop.GetWindowIdFromWindow(hwnd));
        var presenter = OverlappedPresenter.CreateForContextMenu();
        presenter.IsAlwaysOnTop = true;
        _appWindow.SetPresenter(presenter);
        _appWindow.IsShownInSwitchers = false;
        Win32.AddExStyle(hwnd, Win32.WsExToolWindow | Win32.WsExNoActivate);
        Win32.RoundSmallCorners(hwnd);
    }

    /// <summary>核が渡した状態を描く（UI のスレッド）。古い状態は捨てる。</summary>
    public void Render(RenderState state)
    {
        if (state.Revision <= _lastRevision)
        {
            return;
        }
        _lastRevision = state.Revision;
        if (!state.Visible || state.Model is null || state.Anchor is null || state.WorkArea is null)
        {
            _appWindow.Hide();
            return;
        }
        CandidatePanelModel model = state.Model;
        bool above = state.Side == PanelSide.Above;
        double candidateRow = CoreNative.PanelHeightDip(1, false, false);
        double annotationRow = CoreNative.PanelHeightDip(1, true, false) - candidateRow;
        double noteRow = CoreNative.PanelHeightDip(1, false, true) - candidateRow;

        _rows.Children.Clear();
        // 注釈と開発者の行は、アンカーから遠い側に置く（macOS と同じ）
        if (above)
        {
            AddLine(model.DeveloperNote, noteRow);
            AddLine(model.Annotation, annotationRow);
        }
        foreach (uint index in state.DisplayOrder)
        {
            if (index >= model.Candidates.Count)
            {
                continue;
            }
            _rows.Children.Add(CandidateRow(model.Candidates[(int)index], index, index == model.Selected, candidateRow));
        }
        if (!above)
        {
            AddLine(model.Annotation, annotationRow);
            AddLine(model.DeveloperNote, noteRow);
        }

        double heightDip = CoreNative.PanelHeightDip(
            (uint)model.Candidates.Count, model.Annotation.Length > 0, model.DeveloperNote.Length > 0);
        _rows.Measure(new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity));
        // 窓は作業領域より広くしない（長い行は末尾が切れて見える。ADR 0068 §5）
        int width = (int)Math.Min(Math.Ceiling(_rows.DesiredSize.Width * state.Scale), Math.Floor(state.WorkArea.Width));
        int height = (int)Math.Ceiling(heightDip * state.Scale);
        Rect anchor = state.Anchor;
        Rect work = state.WorkArea;
        double x = CoreNative.HorizontalOrigin(anchor.X, width, work.X, work.Width);
        double y = CoreNative.VerticalOrigin(
            anchor.Y, anchor.Height, height, above ? CoreNative.SideAbove : CoreNative.SideBelow, work.Y, work.Height);
        _appWindow.MoveAndResize(new RectInt32((int)Math.Round(x), (int)Math.Round(y), Math.Max(width, 1), Math.Max(height, 1)));
        _appWindow.Show(false);
    }

    private void AddLine(string text, double heightDip)
    {
        if (text.Length == 0)
        {
            return;
        }
        _rows.Children.Add(new Border
        {
            Height = heightDip,
            Padding = new Thickness(CellPaddingDip, 0, CellPaddingDip, 0),
            Child = new TextBlock
            {
                Text = text,
                FontSize = SmallFontDip,
                VerticalAlignment = VerticalAlignment.Center,
                TextWrapping = TextWrapping.NoWrap,
                TextTrimming = TextTrimming.CharacterEllipsis,
            },
        });
    }

    private static Border CandidateRow(CandidateItem item, uint index, bool selected, double heightDip)
    {
        var line = new StackPanel { Orientation = Orientation.Horizontal, Spacing = LabelGapDip };
        line.Children.Add(new TextBlock
        {
            Text = (index + 1).ToString(System.Globalization.CultureInfo.InvariantCulture),
            FontSize = SmallFontDip,
            VerticalAlignment = VerticalAlignment.Center,
        });
        line.Children.Add(new TextBlock
        {
            Text = item.Surface,
            FontSize = CandidateFontDip,
            VerticalAlignment = VerticalAlignment.Center,
            TextWrapping = TextWrapping.NoWrap,
        });
        if (item.Label.Length > 0)
        {
            line.Children.Add(new TextBlock
            {
                Text = item.Label,
                FontSize = SmallFontDip,
                Opacity = 0.6,
                VerticalAlignment = VerticalAlignment.Center,
            });
        }
        var row = new Border
        {
            Height = heightDip,
            Padding = new Thickness(CellPaddingDip, 0, CellPaddingDip, 0),
            Child = line,
        };
        if (selected)
        {
            row.Background = (Brush)Application.Current.Resources["AccentFillColorDefaultBrush"];
            foreach (UIElement child in line.Children)
            {
                if (child is TextBlock text)
                {
                    text.Foreground = (Brush)Application.Current.Resources["TextOnAccentFillColorPrimaryBrush"];
                }
            }
        }
        return row;
    }
}
