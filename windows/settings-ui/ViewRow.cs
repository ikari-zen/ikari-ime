namespace IkariSettings;

/// <summary>
/// 核が返した画面の 1 行（windows/settings の view.rs の列の順）。部品、範囲、選択肢は核が決めたもの。中身は解釈しない。
/// </summary>
internal sealed record ViewRow(
    string Key, string Widget, string Section, string Screen, string Value,
    string Min, string Max, IReadOnlyList<string> Choices, string Reflect)
{
    /// <summary>列の数</summary>
    private const int Columns = 9;

    /// <summary>TSV を行に分ける。列の数が合わない行は描かない（核と画面の版のずれ）。</summary>
    public static IReadOnlyList<ViewRow> Parse(string tsv)
    {
        var rows = new List<ViewRow>();
        foreach (string line in tsv.Split('\n', StringSplitOptions.RemoveEmptyEntries))
        {
            string[] c = line.TrimEnd('\r').Split('\t');
            if (c.Length != Columns)
            {
                continue;
            }
            string[] choices = c[7].Length == 0 ? Array.Empty<string>() : c[7].Split('|');
            rows.Add(new ViewRow(c[0], c[1], c[2], c[3], c[4], c[5], c[6], choices, c[8]));
        }
        return rows;
    }
}
