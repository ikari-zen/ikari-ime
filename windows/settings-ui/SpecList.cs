namespace IkariSettings;

/// <summary>核が返した一覧の 1 行（windows/settings の abi.rs の列の順）。中身は解釈しない（描くだけ）。</summary>
internal sealed record SpecRow(string Key, string Kind, string Section, string Screen, string Default, string Reflect);

internal static class SpecList
{
    /// <summary>列の数（キー、種類、節、画面、既定値、反映）</summary>
    private const int Columns = 6;

    /// <summary>TSV を行に分ける。列の数が合わない行は描かない（核と画面の版のずれ）。</summary>
    public static IReadOnlyList<SpecRow> Parse(string tsv)
    {
        var rows = new List<SpecRow>();
        foreach (string line in tsv.Split('\n', StringSplitOptions.RemoveEmptyEntries))
        {
            string[] c = line.TrimEnd('\r').Split('\t');
            if (c.Length != Columns)
            {
                continue;
            }
            rows.Add(new SpecRow(c[0], c[1], c[2], c[3], c[4], c[5]));
        }
        return rows;
    }
}
