#!/usr/bin/env python3
"""ADR 0037 §6.1.1 / §7: leave-one-out の準備（S0 用の捨てるスクリプト。本体には入れない）。

Sudachi の small・core・notcore の CSV から、表記も読みもカタカナで、読みが 6 字以上、末尾が
〜ング・〜ション・〜ジョン の (表記, 読み) を抜き出して除外表にし、
  - 除外表（exclusions.tsv）
  - 評価用 TSV（loo.tsv。eval/sets/katakana-suffix.tsv と同じ列）
  - 除外表の行を除いた CSV の写し（<out>/csv/{small,core,notcore}_lex.csv）
を <out> に作る。同じ (表記, 読み) の行は品詞が違ってもすべて外す。同じ読みの別表記の行は残す。

使い方: katakana_s0_prepare.py --sudachi <sudachi の dir> --out <出力 dir>
"""
import argparse
import csv
import os
import sys
from collections import defaultdict

LEX_FILES = ["small_lex.csv", "core_lex.csv", "notcore_lex.csv"]
COL_SURFACE, COL_READING, COL_A_UNIT, COL_B_UNIT = 0, 11, 15, 16
MIN_READING_CHARS = 6
# 末尾の型 -> tags の型名。長い順に判定する（〜ション と 〜ジョン と 〜ング は末尾が重ならない）
SUFFIXES = [("ング", "ng"), ("ション", "shon"), ("ジョン", "jon")]
FNV_OFFSET, FNV_PRIME, MASK = 0xCBF29CE484222325, 0x100000001B3, (1 << 64) - 1
SPLIT_MODULUS = 5


def is_katakana_word(s: str) -> bool:
    # ァ-ヺ と 長音。・ は含めない（複合語は対象外）
    return bool(s) and all("ァ" <= c <= "ヺ" or c == "ー" for c in s)


def to_hiragana(s: str) -> str:
    return "".join(chr(ord(c) - 0x60) if "ァ" <= c <= "ヶ" else c for c in s)


def fnv1a64(data: bytes) -> int:
    h = FNV_OFFSET
    for b in data:
        h = ((h ^ b) * FNV_PRIME) & MASK
    return h


def split_of(case_id: str) -> str:
    return "test" if fnv1a64(case_id.encode()) % SPLIT_MODULUS == 0 else "dev"


def suffix_type(reading: str):
    for suf, name in SUFFIXES:
        if reading.endswith(suf):
            return name
    return None


def parse(line: str):
    return next(csv.reader([line]))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--sudachi", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    os.makedirs(os.path.join(a.out, "csv"), exist_ok=True)

    # パス 1: 除外する (表記, 読み) を決める。同じ読みの行（残る側の判定用）の数も数える
    pair_rows = defaultdict(int)  # (surface, reading) -> 行数
    pair_unsplit = defaultdict(lambda: True)  # すべての行が A・B の印なし
    reading_surfaces = defaultdict(set)  # reading -> 表記の集合（カタカナ読みの全行）
    for name in LEX_FILES:
        with open(os.path.join(a.sudachi, name), encoding="utf-8", newline="") as f:
            for line in f:
                row = parse(line)
                if len(row) < 17:
                    continue
                s, r = row[COL_SURFACE], row[COL_READING]
                reading_surfaces[r].add(s)
                if not (is_katakana_word(s) and is_katakana_word(r)):
                    continue
                if len(r) < MIN_READING_CHARS or suffix_type(r) is None:
                    continue
                pair_rows[(s, r)] += 1
                if not (row[COL_A_UNIT] == "*" and row[COL_B_UNIT] == "*"):
                    pair_unsplit[(s, r)] = False

    pairs = sorted(pair_rows)
    excl = set(pairs)
    with open(os.path.join(a.out, "exclusions.tsv"), "w", encoding="utf-8") as f:
        f.write("surface\treading\trows\ttype\tunsplit\n")
        for s, r in pairs:
            f.write(f"{s}\t{r}\t{pair_rows[(s, r)]}\t{suffix_type(r)}\t{int(pair_unsplit[(s, r)])}\n")

    header = "id\tkind\ttags\tprofile\tcontext\treading\texpected\tnote\tsplit\tsource\n"
    n_homophone = 0
    with open(os.path.join(a.out, "loo.tsv"), "w", encoding="utf-8") as f:
        f.write(header)
        for s, r in pairs:
            hira = to_hiragana(r)
            case_id = f"loo-{hira}-{s}"
            # 同じ読みで別の表記の行が残る（除外表に入らない行がある）
            left = [x for x in reading_surfaces[r] if (x, r) not in excl]
            tags = ["loo", suffix_type(r), "unsplit" if pair_unsplit[(s, r)] else "split"]
            if left:
                tags.append("homophone-left")
                n_homophone += 1
            note = "leave-one-out の疑似の未知語（ADR 0037 §6.1.1）"
            f.write("\t".join([case_id, "カタカナ語(LOO)", ",".join(tags), "", "", hira, s, note, split_of(case_id), "s0-loo"]) + "\n")

    # パス 2: CSV の写し
    removed = defaultdict(int)
    for name in LEX_FILES:
        src = os.path.join(a.sudachi, name)
        dst = os.path.join(a.out, "csv", name)
        with open(src, encoding="utf-8", newline="") as fi, open(dst, "w", encoding="utf-8", newline="") as fo:
            for line in fi:
                row = parse(line)
                if len(row) >= 17 and (row[COL_SURFACE], row[COL_READING]) in excl:
                    removed[name] += 1
                    continue
                fo.write(line)

    by_type = defaultdict(int)
    by_type_unsplit = defaultdict(int)
    for s, r in pairs:
        by_type[suffix_type(r)] += 1
        by_type_unsplit[suffix_type(r)] += int(pair_unsplit[(s, r)])
    print(f"除外する (表記, 読み): {len(pairs)}（うち印なし {sum(by_type_unsplit.values())}）、同音の別表記が残る {n_homophone}")
    for k in by_type:
        print(f"  {k}: {by_type[k]}（印なし {by_type_unsplit[k]}）")
    print("CSV から外した行:", dict(removed))
    return 0


if __name__ == "__main__":
    sys.exit(main())
