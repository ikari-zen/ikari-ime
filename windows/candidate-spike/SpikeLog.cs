using System.Runtime.InteropServices;

namespace IkariCandidateSpike;

/// <summary>標準出力（親のコンソールがあれば）とファイルに同じ行を書く。候補の文字は書かない（個数だけ）。</summary>
internal sealed class SpikeLog
{
    private const int AttachParentProcess = -1;
    private readonly StreamWriter _file;
    private readonly long _t0 = System.Diagnostics.Stopwatch.GetTimestamp();
    private readonly bool _console;

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool AttachConsole(int dwProcessId);

    public SpikeLog(string path)
    {
        // WinExe なので、コンソールから起動されたときだけ親のコンソールに付ける（新しい窓を作るとフォーカスを奪う）
        _console = AttachConsole(AttachParentProcess);
        if (_console)
        {
            Console.SetOut(new StreamWriter(Console.OpenStandardOutput()) { AutoFlush = true });
        }
        _file = new StreamWriter(path, append: false, new System.Text.UTF8Encoding(false)) { AutoFlush = true };
    }

    public void Write(string line)
    {
        double ms = System.Diagnostics.Stopwatch.GetElapsedTime(_t0).TotalMilliseconds;
        string s = $"[{ms,8:F1}ms] {line}";
        _file.WriteLine(s);
        if (_console)
        {
            Console.Out.WriteLine(s);
        }
    }
}
