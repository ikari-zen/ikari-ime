#!/usr/bin/env python3
"""ADR 0038 §6.2 S1: 評価セットの context の末尾から「確定の単位」を 1 つ取り除いた stale セットを作る（捨てるスクリプト）。

使い方: make_stale.py <出力ディレクトリ>
出力: <out>/stale-{A,B}/<セット名>.tsv（context だけ差し替え）と <out>/units-{A,B}.tsv（セット名, id, 取り除いた単位の長さ, 単位）
単位の切り方（issue #1 の測定 (a) と同じ規則による近似）:
  A 文節の近似: 「かな以外の連続（漢字・カタカナ・英数）+ 続くひらがなの連続」を 1 単位。句読点は直前の単位に付ける。
  B 字種の変わり目: 漢字・ひらがな・カタカナ・英数の変わり目で切る。句読点は直前の単位に付ける。
単位が 1 つしかない context と、取り除く単位が MAX_UNIT 字を超えるものは除く。
"""
import glob
import os
import sys

MAX_UNIT = 12


def klass(c):
    o = ord(c)
    if 0x3041 <= o <= 0x3096 or c in "ゝゞ":
        return "hira"
    if 0x30A1 <= o <= 0x30FA or c in "ヽヾ":
        return "kata"
    if 0x4E00 <= o <= 0x9FFF or 0x3400 <= o <= 0x4DBF or c in "々〆〇":
        return "kanji"
    if (c.isascii() and c.isalnum()) or 0xFF10 <= o <= 0xFF19 or 0xFF21 <= o <= 0xFF3A or 0xFF41 <= o <= 0xFF5A:
        return "alnum"
    return "other"


def classes(ctx):
    """各文字の字種。ー は直前の字種に続ける。句読点・記号は 'other'（直前の単位に付く）。"""
    out = []
    for c in ctx:
        k = klass(c)
        if c == "ー":
            k = out[-1] if out and out[-1] != "other" else "kata"
        out.append(k)
    return out


def split_b(ctx):
    units, cur, cur_k = [], "", None
    for c, k in zip(ctx, classes(ctx)):
        if k == "other" or cur_k is None or k == cur_k:
            cur += c
            if k != "other" and cur_k is None:
                cur_k = k
        else:
            units.append(cur)
            cur, cur_k = c, k
    if cur:
        units.append(cur)
    return units


def split_a(ctx):
    units, cur, state = [], "", None  # state: None / "non"（かな以外の連続中）/ "hira"
    for c, k in zip(ctx, classes(ctx)):
        if k == "other":
            cur += c
            continue
        if k == "hira":
            cur += c
            state = "hira"
        else:
            if state == "hira":
                units.append(cur)
                cur = ""
            cur += c
            state = "non"
    if cur:
        units.append(cur)
    return units


def main():
    out = sys.argv[1]
    for scheme, fn in (("A", split_a), ("B", split_b)):
        os.makedirs(f"{out}/stale-{scheme}", exist_ok=True)
        ulines = ["set\tid\tunit_len\tunit"]
        for path in sorted(glob.glob("eval/sets/*.tsv")):
            name = os.path.basename(path)[:-4]
            lines = open(path, encoding="utf-8").read().split("\n")
            hi = next(i for i, l in enumerate(lines) if l and not l.startswith("#"))
            header = lines[hi].split("\t")
            if "context" not in header:
                continue
            ci, ii = header.index("context"), header.index("id")
            kept = lines[: hi + 1]
            for l in lines[hi + 1:]:
                if not l or l.startswith("#"):
                    continue
                f = l.split("\t")
                ctx = f[ci] if ci < len(f) else ""
                if not ctx:
                    continue
                us = fn(ctx)
                if len(us) < 2 or len(us[-1]) > MAX_UNIT:
                    continue
                f[ci] = ctx[: len(ctx) - len(us[-1])]
                kept.append("\t".join(f))
                ulines.append(f"{name}\t{f[ii]}\t{len(us[-1])}\t{us[-1]}")
            if len(kept) > hi + 1:
                with open(f"{out}/stale-{scheme}/{name}.tsv", "w", encoding="utf-8") as fh:
                    fh.write("\n".join(kept) + "\n")
        with open(f"{out}/units-{scheme}.tsv", "w", encoding="utf-8") as fh:
            fh.write("\n".join(ulines) + "\n")
        print(scheme, len(ulines) - 1)


main()
