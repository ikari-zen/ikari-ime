#!/usr/bin/env python3
"""ADR 0039 §6.1 の PoC: 集めた一覧の再現率、絞りごとの件数、本物の未知語、F4 の効きと誤りを数える（捨てるスクリプト）。

  python3 -I webwords_analyze.py --sudachi <data/raw/sudachi-…> --lists a.tsv [b.tsv c.tsv] --out <出力ディレクトリ>

出力: report.md（表）、cands-df<D>.tsv（F1〜F4 を通った候補: 表記 tf df cue）
再現率 = Sudachi の普通名詞のカタカナ語（表記も読みもカタカナ、3 字以上。hidden）のうち、F1 を当てずに F2〜F4 を通って一覧に残る割合。
"""
import argparse
import os
import sys
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import webwords_lib as L  # noqa: E402

REAL = ["バージョニング", "シャーディング", "セマンティックバージョニング", "ロードバランサー", "マイクロサービス", "リファクタリング",
        "コンテナ", "デプロイ", "オーケストレーション", "オブザーバビリティ", "レプリケーション", "キャッシュ", "ストリーミング"]
DFS = [2, 3, 5, 10, 20, 50]


def pct(a, b):
    return f"{100 * a / b:.1f}%" if b else "-"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sudachi", required=True)
    ap.add_argument("--lists", nargs="+", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--min-len", type=int, default=3)
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)

    sud = L.Sudachi(a.sudachi)
    loaded = [L.read_list(p) for p in a.lists]
    out = []
    w = out.append
    w("# webwords PoC 解析（自動生成）\n")
    w(f"- Sudachi: 全カタカナ表記 {len(sud.all)}、普通名詞 {len(sud.common)}、固有名詞だけ {len(sud.proper_only)}、再現率の対象 hidden {len(sud.hidden)}")
    w(f"- 左右 ID（名詞,普通名詞,一般 でカタカナだけの行の最頻）: {sud.common_ids()}")
    for p, (h, ws) in zip(a.lists, loaded):
        w(f"- {os.path.basename(p)}: {h.split(chr(9),1)[1] if chr(9) in h else h}; 語 {len(ws)}")
    w("")

    subsets = [(f"先頭 {k} ファイル", L.merge([ws for _, ws in loaded[:k]])) for k in range(1, len(loaded) + 1)]
    allw = subsets[-1][1]

    # 再現率: 標本の量 × df の下限
    w("## 再現率（hidden のうち一覧に残る割合。F3〜F4b+c を通ったもの。F1 は当てない）\n")
    w("| 標本 | " + " | ".join(f"df>={d}" for d in DFS) + " |")
    w("|---|" + "---|" * len(DFS))
    for name, ws in subsets:
        cells = []
        for d in DFS:
            hit = sum(1 for s in sud.hidden if s in ws and ws[s][1] >= d and not L.f3_reject(s, a.min_len)
                      and not (ws[s][2] / ws[s][0] >= L.DEFAULT_CUE_RATIO) and not L.segmentable(s, sud.proper_only))
            cells.append(f"{hit} ({pct(hit, len(sud.hidden))})")
        w(f"| {name} | " + " | ".join(cells) + " |")
    w("")
    w("F2 だけ（df の下限のみ。F3 以降を当てない）の再現率:\n")
    w("| 標本 | " + " | ".join(f"df>={d}" for d in DFS) + " |")
    w("|---|" + "---|" * len(DFS))
    for name, ws in subsets:
        cells = []
        for d in DFS:
            hit = sum(1 for s in sud.hidden if s in ws and ws[s][1] >= d)
            cells.append(f"{hit} ({pct(hit, len(sud.hidden))})")
        w(f"| {name} | " + " | ".join(cells) + " |")
    w("")

    # 候補の件数（F1 の後）
    w("## 絞りごとの件数（全標本。min_len=%d）\n" % a.min_len)
    w("| df の下限 | 下限を超えた連なり | F1 辞書にある | F3 短い | F3 頭が ー・小書き | F3 ヵヶヷ… | F3 反復 | F4b 手がかり | F4c 固有名詞の部分 | 残り（F5 なし） | F5 既知の普通名詞の連なり | 残り（F5 あり） |")
    w("|---|---|---|---|---|---|---|---|---|---|---|---|")
    cand = {}
    for d in DFS:
        st = Counter()
        res = L.filter_words(allw, sud, d, a.min_len, stats=st)
        st5 = Counter()
        res5 = L.filter_words(allw, sud, d, a.min_len, f5=True, stats=st5)
        cand[d] = res
        w(f"| {d} | {st['after_f2']} | {st['f1_in_sudachi']} | {st['f3_short']} | {st['f3_lead']} | {st['f3_badchar']} | {st['f3_redup']} | {st['f4b_cue']} | {st['f4c_proper_parts']} | {st['kept']} | {st5['f5_known_parts']} | {st5['kept']} |")
        with open(os.path.join(a.out, f"cands-df{d}.tsv"), "w", encoding="utf-8") as o:
            for x in sorted(res):
                o.write("\t".join(map(str, x)) + "\n")
    w("")

    # F4 の効き（固有名詞だけの Sudachi 表記を代理の固有名詞とし、hidden を代理の普通名詞とする）
    w("## F4 の効きと誤り（Sudachi の表記を代理にした見積もり。df>=3、全標本）\n")
    w("効き = 一覧にある「固有名詞だけ」の表記（Sudachi）のうち除かれる割合。誤り = 一覧にある hidden（普通名詞）のうち除かれる割合。\n")
    d = 3
    prop = [s for s in sud.proper_only if s in allw and allw[s][1] >= d and len(s) >= a.min_len]
    hid = [s for s in sud.hidden if s in allw and allw[s][1] >= d]
    w(f"対象: 固有名詞だけ {len(prop)} 語、hidden {len(hid)} 語\n")
    w("| 規則 | 効き（固有名詞が除かれる） | 誤り（普通名詞が除かれる） |")
    w("|---|---|---|")
    for th in (0.1, 0.2, 0.3, 0.5):
        e = sum(1 for s in prop if allw[s][2] / allw[s][0] >= th)
        m = sum(1 for s in hid if allw[s][2] / allw[s][0] >= th)
        w(f"| F4b cue/tf >= {th} | {e} ({pct(e, len(prop))}) | {m} ({pct(m, len(hid))}) |")
    # F4c は「自分自身を除いた」固有名詞の部分で分けられるか。hidden の側は proper_only に自分が入っていないので自然に除かれる
    e = sum(1 for s in prop if L.segmentable(s, sud.proper_only))
    m = sum(1 for s in hid if L.segmentable(s, sud.proper_only))
    w(f"| F4c 固有名詞だけの部分に分かれる | {e} ({pct(e, len(prop))}) | {m} ({pct(m, len(hid))}) |")
    w("")

    # 本物の未知語
    w("## 本物の未知語（全標本と先頭 1 ファイル）\n")
    w("| 表記 | Sudachi | 全標本 tf | df | cue | 先頭 1 ファイル df | 全標本 F3〜F4 を通るか |")
    w("|---|---|---|---|---|---|---|")
    first = subsets[0][1]
    for s in REAL:
        e = allw.get(s)
        f = first.get(s)
        why = "-"
        if e:
            r = L.f3_reject(s, a.min_len)
            why = r or ("F4b" if e[2] / e[0] >= L.DEFAULT_CUE_RATIO else ("F4c" if L.segmentable(s, sud.proper_only) else "通る"))
        w(f"| {s} | {'あり' if s in sud.all else 'なし'} | {e[0] if e else 0} | {e[1] if e else 0} | {e[2] if e else 0} | {f[1] if f else 0} | {why} |")
    w("")

    # 上位の例（df の降順。F1 の後）
    w("## 候補の上位 40 語（df>=3、df の降順。公開の語だけを見るために人が目で確かめる）\n")
    top = sorted(cand[3], key=lambda x: (-x[2], x[0]))[:40]
    w("、".join(f"{x[0]}(df{x[2]})" for x in top))
    w("")
    with open(os.path.join(a.out, "report.md"), "w", encoding="utf-8") as o:
        o.write("\n".join(out) + "\n")
    print("\n".join(out))


if __name__ == "__main__":
    main()
