#!/usr/bin/env python3
"""ADR 0039 §2.3〜§2.5 の PoC: 一覧から F1〜F5 で絞り、Sudachi と同じ形の lex.csv（品詞 名詞,普通名詞,一般）を作る（捨てるスクリプト）。

  python3 -I webwords_make_lex.py --sudachi <dir> --lists a.tsv … --out lex.csv --min-df 3 --min-len 3 --cost 15000 [--cost-tiers "10:13000,50:11000"] [--f5]

コスト: --cost が一律の値。--cost-tiers "df下限:コスト,…" を渡すと、df がその下限以上の語のうち最も高い段のコストにする（それ未満は --cost）。
左右 ID は Sudachi の「名詞,普通名詞,一般 でカタカナだけの行」の最頻の組（固定しない。ADR 0039 §2.5）。
"""
import argparse
import os
import sys
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import webwords_lib as L  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sudachi", required=True)
    ap.add_argument("--lists", nargs="+", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--min-df", type=int, required=True)
    ap.add_argument("--min-len", type=int, default=3)
    ap.add_argument("--min-tf", type=int, default=0)
    ap.add_argument("--cue-ratio", type=float, default=L.DEFAULT_CUE_RATIO)
    ap.add_argument("--no-f4c", action="store_true")
    ap.add_argument("--f5", action="store_true")
    ap.add_argument("--cost", type=int, required=True)
    ap.add_argument("--cost-tiers", default="")
    ap.add_argument("--stats", default="")
    ap.add_argument("--ids", default="", help="左右 ID を固定する（\"5139,5139\"）。省略すると最頻の組")
    a = ap.parse_args()

    sud = L.Sudachi(a.sudachi)
    words = L.merge([L.read_list(p)[1] for p in a.lists])
    st = Counter()
    res = L.filter_words(words, sud, a.min_df, a.min_len, cue_ratio=a.cue_ratio, f4c=not a.no_f4c, f5=a.f5, min_tf=a.min_tf, stats=st)
    tiers = sorted((int(x.split(":")[0]), int(x.split(":")[1])) for x in a.cost_tiers.split(",") if x)
    left, right = a.ids.split(",") if a.ids else sud.common_ids()

    def cost_of(df):
        c = a.cost
        for lo, v in tiers:
            if df >= lo:
                c = v
        return c

    with open(a.out, "w", encoding="utf-8") as o:
        for w, tf, df, cue in sorted(res):
            o.write(L.lex_row(w, left, right, cost_of(df)))
    msg = f"rows={len(res)} ids=({left},{right}) " + " ".join(f"{k}={v}" for k, v in sorted(st.items()))
    print(msg)
    if a.stats:
        with open(a.stats, "w", encoding="utf-8") as o:
            o.write(msg + "\n")


if __name__ == "__main__":
    main()
