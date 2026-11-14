namespace IkariCandidateSpike;

/// <summary>コマンドラインの引数。`--x 100 --y 200 [--seconds 5] [--png p] [--log p] [--ws-ex-noactivate on|off] 候補...`</summary>
internal sealed class Options
{
    public const int DefaultSeconds = 5;
    public const string DefaultLogFileName = "candidate-spike.log";

    public int X { get; private set; }
    public int Y { get; private set; }
    public int Seconds { get; private set; } = DefaultSeconds;
    public string? PngPath { get; private set; }
    public string LogPath { get; private set; } = Path.Combine(AppContext.BaseDirectory, DefaultLogFileName);
    /// <summary>S-3 の比較用。WS_EX_NOACTIVATE を足すか（既定は足す）。</summary>
    public bool NoActivateStyle { get; private set; } = true;
    public List<string> Candidates { get; } = new();

    public static Options Parse(string[] args)
    {
        var o = new Options();
        for (int i = 0; i < args.Length; i++)
        {
            string a = args[i];
            string Next() => i + 1 < args.Length ? args[++i] : throw new ArgumentException($"{a} needs a value");
            switch (a)
            {
                case "--x": o.X = int.Parse(Next()); break;
                case "--y": o.Y = int.Parse(Next()); break;
                case "--seconds": o.Seconds = int.Parse(Next()); break;
                case "--png": o.PngPath = Next(); break;
                case "--log": o.LogPath = Next(); break;
                case "--ws-ex-noactivate": o.NoActivateStyle = Next() != "off"; break;
                default: o.Candidates.Add(a); break;
            }
        }
        if (o.Candidates.Count == 0)
        {
            o.Candidates.AddRange(new[] { "変換", "返還", "変官", "変感" });
        }
        return o;
    }
}
