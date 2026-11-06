#!/usr/bin/env python3
"""ADR 0039 PoC: snapshot.sh の出力ディレクトリ 2 つを、ケース別の 1 位で比べる（捨てるスクリプト）。

  python3 -I webwords_compare.py <基準> <候補> [--show 20]

<dir>/eval/*.tsv の列 1〜7（id kind context reading expected top1 rank）を比べ、rank==1 を 1 位正解とする。
セットごとに「1 位正解 基準 → 候補」「悪化（基準で 1 位、候補で 1 位でない）」「改善」を出し、悪化の全件を列挙する。
終了コード: 悪化が 1 件でもあれば 1。
"""
import argparse
import os
import sys


def load(path):
    rows = {}
    with open(path, encoding="utf-8") as f:
        for i, line in enumerate(f):
            c = line.rstrip("\n").split("\t")
            if len(c) < 7 or c[0] == "id":
                continue
            rows[(c[0], i)] = c[:7]
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("base")
    ap.add_argument("cand")
    ap.add_argument("--show", type=int, default=30)
    ap.add_argument("--all", action="store_true", help="変化のないセットも表に出す")
    a = ap.parse_args()
    bdir, cdir = os.path.join(a.base, "eval"), os.path.join(a.cand, "eval")
    names = sorted(n for n in os.listdir(bdir) if n.endswith(".tsv") and os.path.exists(os.path.join(cdir, n)))
    tot_b = tot_c = tot_reg = tot_imp = tot_top1chg = 0
    regs = []
    print("| セット | 件数 | 1 位正解 基準 | 候補 | 悪化 | 改善 | 1 位の表記が変わった |")
    print("|---|---|---|---|---|---|---|")
    for n in names:
        b, c = load(os.path.join(bdir, n)), load(os.path.join(cdir, n))
        nb = sum(1 for r in b.values() if r[6] == "1")
        nc = sum(1 for k, r in c.items() if r[6] == "1")
        reg = imp = chg = 0
        for k, rb in b.items():
            rc = c.get(k)
            if rc is None:
                continue
            if rb[5] != rc[5]:
                chg += 1
            if rb[6] == "1" and rc[6] != "1":
                reg += 1
                regs.append((n, rb, rc))
            if rb[6] != "1" and rc[6] == "1":
                imp += 1
        tot_b += nb
        tot_c += nc
        tot_reg += reg
        tot_imp += imp
        tot_top1chg += chg
        if a.all or reg or imp or chg:
            print(f"| {n[:-4]} | {len(b)} | {nb} | {nc} | {reg} | {imp} | {chg} |")
    print(f"| 合計（変化のあったセットのみ表示） | | {tot_b} | {tot_c} | {tot_reg} | {tot_imp} | {tot_top1chg} |")
    for n, rb, rc in regs[: a.show]:
        print(f"悪化: {n[:-4]} {rb[0]} 文脈={rb[2]!r} 読み={rb[3]} 期待={rb[4]} 基準 1 位={rb[5]} → 候補 1 位={rc[5]}（順位 {rc[6]}）")
    return 1 if tot_reg else 0


if __name__ == "__main__":
    sys.exit(main())
