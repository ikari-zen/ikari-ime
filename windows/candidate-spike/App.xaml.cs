using System.Runtime.InteropServices.WindowsRuntime;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;
using Windows.Graphics;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace IkariCandidateSpike;

/// <summary>
/// ADR 0062 §8 の S-1・S-3 のスパイク。指定の物理座標に、フォーカスを奪わず最前面で候補を並べた小さな窓を出し、
/// 起きたことをログに書く。判断（何をどう並べるか）は持たない（描くだけ）。捨てるコード。
/// </summary>
public partial class App : Application
{
    private const double RowHeightDip = 28;
    private const double RowPaddingDip = 12;
    private const double FontSizeDip = 14;
    private const int SettleDelayMs = 300;
    private const int PngBitsDpi = 96;
    private const double DipPerInch = 96.0;

    private Window? _window;
    private SpikeLog? _log;

    public App()
    {
        InitializeComponent();
    }

    protected override async void OnLaunched(LaunchActivatedEventArgs args)
    {
        Options opt;
        try
        {
            opt = Options.Parse(Environment.GetCommandLineArgs().Skip(1).ToArray());
            _log = new SpikeLog(opt.LogPath);
        }
        catch (Exception e)
        {
            Environment.Exit(Fail(e));
            return;
        }
        UnhandledException += (_, ev) => { _log!.Write($"unhandled: {ev.Exception}"); ev.Handled = true; };

        try
        {
            await RunAsync(opt);
        }
        catch (Exception e)
        {
            _log!.Write($"error: {e}");
            Environment.Exit(1);
        }
    }

    private static int Fail(Exception e)
    {
        File.WriteAllText(Path.Combine(AppContext.BaseDirectory, "candidate-spike.fatal.txt"), e.ToString());
        return 2;
    }

    private async Task RunAsync(Options opt)
    {
        SpikeLog log = _log!;
        log.Write($"start anchor_physical=({opt.X},{opt.Y}) candidates={opt.Candidates.Count} seconds={opt.Seconds} ws_ex_noactivate={opt.NoActivateStyle} png={(opt.PngPath is null ? "off" : "on")}");
        Native.LogMonitors(log, opt.X, opt.Y);
        Native.LogFocus(log, "before", IntPtr.Zero);

        FrameworkElement root = BuildContent(opt.Candidates);
        _window = new Window { Content = root };
        // Activate() は呼ばない（フォーカスを奪うため）。Show(false) だけで出す
        IntPtr hwnd = WinRT.Interop.WindowNative.GetWindowHandle(_window);
        AppWindow appWindow = AppWindow.GetFromWindowId(Win32Interop.GetWindowIdFromWindow(hwnd));

        var presenter = OverlappedPresenter.CreateForContextMenu();
        presenter.IsAlwaysOnTop = true;
        appWindow.SetPresenter(presenter);
        appWindow.IsShownInSwitchers = false;
        long add = Native.WsExToolWindow | (opt.NoActivateStyle ? Native.WsExNoActivate : 0);
        Native.AddExStyle(hwnd, add);
        Native.RoundCorners(log, hwnd);

        // 位置は物理のピクセル。いったんアンカーに置いて、その画面の DPI で大きさを決める
        appWindow.MoveAndResize(new RectInt32(opt.X, opt.Y, 1, 1));
        uint dpi = Native.WindowDpi(hwnd);
        double scale = dpi / DipPerInch;
        root.Measure(new Size(double.PositiveInfinity, double.PositiveInfinity));
        int pw = (int)Math.Ceiling(root.DesiredSize.Width * scale);
        int ph = (int)Math.Ceiling(root.DesiredSize.Height * scale);
        log.Write($"layout window_dpi={dpi} scale={scale:F2} desired_dip={root.DesiredSize.Width:F1}x{root.DesiredSize.Height:F1} -> physical={pw}x{ph} " +
                  $"anchor_logical=({opt.X * DipPerInch / dpi:F1},{opt.Y * DipPerInch / dpi:F1})");
        appWindow.MoveAndResize(new RectInt32(opt.X, opt.Y, pw, ph));

        Native.LogWindow(log, "hidden", hwnd);
        appWindow.Show(false);
        Native.LogFocus(log, "after_show", hwnd);
        Native.LogWindow(log, "after_show", hwnd);

        await Task.Delay(SettleDelayMs);
        log.Write($"appwindow position=({appWindow.Position.X},{appWindow.Position.Y}) size={appWindow.Size.Width}x{appWindow.Size.Height} is_visible={appWindow.IsVisible} " +
                  $"window_dpi={Native.WindowDpi(hwnd)} rasterization_scale={root.XamlRoot?.RasterizationScale.ToString("F2") ?? "n/a"}");
        Native.LogFocus(log, "settled", hwnd);
        Native.LogWindow(log, "settled", hwnd);
        if (opt.PngPath is not null)
        {
            await WritePngAsync(root, opt.PngPath, log);
        }

        await Task.Delay(Math.Max(0, opt.Seconds * 1000 - SettleDelayMs));
        Native.LogFocus(log, "end", hwnd);
        Native.LogWindow(log, "end", hwnd);
        log.Write("exit");
        Exit();
    }

    private static FrameworkElement BuildContent(List<string> candidates)
    {
        var panel = new StackPanel { Padding = new Thickness(4) };
        for (int i = 0; i < candidates.Count; i++)
        {
            panel.Children.Add(new Border
            {
                Height = RowHeightDip,
                Padding = new Thickness(RowPaddingDip, 0, RowPaddingDip, 0),
                Child = new TextBlock
                {
                    Text = $"{i + 1}  {candidates[i]}",
                    FontSize = FontSizeDip,
                    VerticalAlignment = VerticalAlignment.Center,
                },
            });
        }
        return panel;
    }

    /// <summary>S-6。XAML の絵をそのまま PNG にする（画面を撮らない）。ログには色の数だけ出す。</summary>
    private static async Task WritePngAsync(FrameworkElement root, string path, SpikeLog log)
    {
        var rtb = new Microsoft.UI.Xaml.Media.Imaging.RenderTargetBitmap();
        await rtb.RenderAsync(root);
        IBuffer pixels = await rtb.GetPixelsAsync();
        byte[] bgra = pixels.ToArray();
        using var mem = new InMemoryRandomAccessStream();
        BitmapEncoder enc = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, mem);
        enc.SetPixelData(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied, (uint)rtb.PixelWidth, (uint)rtb.PixelHeight, PngBitsDpi, PngBitsDpi, bgra);
        await enc.FlushAsync();
        mem.Seek(0);
        using (var fs = File.Create(path))
        {
            await mem.AsStreamForRead().CopyToAsync(fs);
        }
        var distinct = new HashSet<uint>();
        for (int i = 0; i + 3 < bgra.Length; i += 4)
        {
            distinct.Add(BitConverter.ToUInt32(bgra, i));
        }
        log.Write($"png path={path} size={rtb.PixelWidth}x{rtb.PixelHeight} bytes={new FileInfo(path).Length} distinct_colors={distinct.Count}");
    }
}
