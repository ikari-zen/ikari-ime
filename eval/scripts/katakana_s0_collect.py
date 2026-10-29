#!/usr/bin/env python3
"""ADR 0037 §6.1 S0: 読みを単独で変換して、読みごとのコストを集める（捨てるスクリプト。本体には入れない）。

`ikari --format tsv` を読みごとに呼び（辞書は読むだけ。ユーザーデータは空の一時ディレクトリ）、
候補の base_cost と origin から次を出す。
  n, mora            読みのかなの文字数 / モーラ数
  dict_min           origin=dictionary の候補の最小 base_cost（カタカナ・ひらがなの候補を除く）
  kana_cost          ひらがなの候補の base_cost（= 辞書の最大 + 1。build_first_pool）
  kata_cost / kata_rank  カタカナの候補（origin=katakana）の base_cost と順位（辞書の同じ表記の 1 語はここに出ない）
  exp_rank / exp_origin  期待のどれかと表記が一致する最初の候補の順位と origin
  top1_*             1 位の表記・origin・文節の数・base_cost
  --lm を付けると score 列（LM あり）も見て、kata_score と kata_rank を LM ありの並びで出す。

使い方: katakana_s0_collect.py --cli <ikari> --dict <dir> --user-dir <空の dir> --cases <TSV: group id context reading expected> --out <TSV> [--lm <gguf>]
"""
import argparse
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

SMALL = set("ゃゅょぁぃぅぇぉゎ")
NUM = 60


def moras(s: str) -> int:
    return sum(1 for c in s if c not in SMALL)


def run_one(a, case):
    group, cid, ctx, reading, expected = case
    cmd = [a.cli, "--dict", a.dict, "--user-dir", a.user_dir, "--ranker", "full", "--format", "tsv", "-n", str(NUM)]
    if ctx:
        cmd += ["--context", ctx]
    if a.lm:
        cmd += ["--lm", a.lm, "--lm-budget-ms", "2000"]
    cmd += ["--", reading]
    p = subprocess.run(cmd, capture_output=True, text=True)
    rows = []
    for line in p.stdout.splitlines():
        f = line.split("\t")
        if len(f) >= 5:
            rows.append(f)
    exp = [e for e in expected.split("|") if e]
    out = dict(group=group, id=cid, context=ctx, reading=reading, n=len(reading), mora=moras(reading), expected=expected)
    dict_costs = [int(r[3]) for r in rows if r[4] == "dictionary"]
    out["dict_min"] = min(dict_costs) if dict_costs else ""
    out["n_cand"] = len(rows)
    hira = [r for r in rows if r[4] == "hiragana"]
    kata = [r for r in rows if r[4] == "katakana"]
    out["kana_cost"] = hira[0][3] if hira else ""
    out["kata_cost"] = kata[0][3] if kata else ""
    out["kata_rank"] = kata[0][0] if kata else ""
    out["kata_score"] = kata[0][2] if kata else ""
    er = [r for r in rows if r[1] in exp]
    out["exp_rank"] = er[0][0] if er else ""
    out["exp_origin"] = er[0][4] if er else ""
    out["exp_cost"] = er[0][3] if er else ""
    if rows:
        t = rows[0]
        out["top1"] = t[1]
        out["top1_origin"] = t[4]
        out["top1_cost"] = t[3]
        out["top1_nseg"] = (t[5].count(" + ") + 1) if len(t) > 5 and t[5] else 1
    else:
        out.update(top1="", top1_origin="", top1_cost="", top1_nseg="")
    # LM 確認用: 先頭 NUM 件の score と base_cost の差があるか、カタカナ・ひらがなの score
    out["hira_score"] = hira[0][2] if hira else ""
    out["scores_differ"] = int(any(r[2] != r[3] for r in rows))
    return out


COLS = ["group", "id", "context", "reading", "n", "mora", "expected", "dict_min", "kana_cost", "kata_cost", "kata_rank", "kata_score",
        "hira_score", "scores_differ", "exp_rank", "exp_origin", "exp_cost", "top1", "top1_origin", "top1_cost", "top1_nseg", "n_cand"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", required=True)
    ap.add_argument("--dict", required=True)
    ap.add_argument("--user-dir", required=True)
    ap.add_argument("--cases", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--lm")
    ap.add_argument("--jobs", type=int, default=8)
    a = ap.parse_args()
    cases = []
    with open(a.cases, encoding="utf-8") as f:
        for line in f:
            p = line.rstrip("\n").split("\t")
            if len(p) == 5:
                cases.append(tuple(p))
    with ThreadPoolExecutor(a.jobs) as ex, open(a.out, "w", encoding="utf-8") as fo:
        fo.write("\t".join(COLS) + "\n")
        for i, r in enumerate(ex.map(lambda c: run_one(a, c), cases)):
            fo.write("\t".join(str(r.get(c, "")) for c in COLS) + "\n")
            if i % 1000 == 0:
                print(i, len(cases), file=sys.stderr, flush=True)


if __name__ == "__main__":
    main()
