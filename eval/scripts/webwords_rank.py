#!/usr/bin/env python3
"""ADR 0039 PoC: 辞書ごとに、本物の未知語の読みを変換して期待の表記の順位を出す（捨てるスクリプト）。

  python3 -I webwords_rank.py --cli <ikari> --user-dir <空の dir> --dicts name=dir [name=dir …] [--lm <gguf>]

各辞書で、LM なし（--ranker cost と full）と、--lm があれば LM あり（--ranker full --lm … --lm-budget-ms 3000）の順位を出す。
列: 辞書、構成、ケース、順位（期待が上位 60 件になければ >60）、1 位の表記
"""
import argparse
import subprocess

CASES = [
    ("ばーじょにんぐ", "", "バージョニング"),
    ("ばーじょにんぐ", "APIの", "バージョニング"),
    ("しゃーでぃんぐ", "", "シャーディング"),
    ("まいくろさーびす", "", "マイクロサービス"),
]


def rank(a, dict_dir, ranker, lm, reading, ctx, expected):
    cmd = [a.cli, "--dict", dict_dir, "--user-dir", a.user_dir, "--ranker", ranker, "--format", "tsv", "-n", "60"]
    if lm:
        cmd += ["--lm", lm, "--lm-budget-ms", "3000"]
    if ctx:
        cmd += ["--context", ctx]
    cmd += ["--", reading]
    p = subprocess.run(cmd, capture_output=True, text=True)
    rows = [l.split("\t") for l in p.stdout.splitlines() if l.count("\t") >= 4]
    for r in rows:
        if r[1] == expected:
            return r[0], rows[0][1]
    return ">60", rows[0][1] if rows else "?"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", required=True)
    ap.add_argument("--user-dir", required=True)
    ap.add_argument("--dicts", nargs="+", required=True)
    ap.add_argument("--lm", default="")
    a = ap.parse_args()
    print("dict\tconfig\tcase\trank\ttop1")
    for nd in a.dicts:
        name, d = nd.split("=", 1)
        configs = [("cost", "cost", ""), ("full", "full", "")]
        if a.lm:
            configs.append(("full+lm", "full", a.lm))
        for cname, ranker, lm in configs:
            for reading, ctx, exp in CASES:
                r, top = rank(a, d, ranker, lm, reading, ctx, exp)
                label = (ctx + "|" if ctx else "") + reading
                print(f"{name}\t{cname}\t{label}\t{r}\t{top}", flush=True)


if __name__ == "__main__":
    main()
