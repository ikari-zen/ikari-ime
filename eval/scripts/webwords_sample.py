#!/usr/bin/env python3
"""ADR 0039 §6.1 混入率: F1〜F4 を通った候補から、df の段ごとに決定的に抜き取る（捨てるスクリプト）。判定は人が下書きする。

  python3 -I webwords_sample.py --sudachi <dir> --lists … --per-band 30 --seed ikari-webwords-poc-v1
FNV-1a64(表記 + 種) の小さい順に抜く。F5（既知の普通名詞だけの連なり）に当たるかを 'F5' 列に出す。
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import webwords_lib as L  # noqa: E402

BANDS = [(3, 4), (5, 9), (10, 19), (20, 49), (50, 10**9)]
FNV_OFFSET, FNV_PRIME, MASK = 0xCBF29CE484222325, 0x100000001B3, (1 << 64) - 1


def fnv(s):
    h = FNV_OFFSET
    for b in s.encode():
        h = ((h ^ b) * FNV_PRIME) & MASK
    return h


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sudachi", required=True)
    ap.add_argument("--lists", nargs="+", required=True)
    ap.add_argument("--per-band", type=int, default=30)
    ap.add_argument("--seed", default="ikari-webwords-poc-v1")
    ap.add_argument("--min-len", type=int, default=3)
    a = ap.parse_args()
    sud = L.Sudachi(a.sudachi)
    words = L.merge([L.read_list(p)[1] for p in a.lists])
    res = L.filter_words(words, sud, BANDS[0][0], a.min_len)
    for lo, hi in BANDS:
        band = [x for x in res if lo <= x[2] <= hi]
        pick = sorted(band, key=lambda x: (fnv(x[0] + a.seed), x[0]))[: a.per_band]
        print(f"## df {lo}-{hi if hi < 10**9 else ''}（この段の候補 {len(band)} 語、うち F5 に当たる {sum(1 for x in band if L.segmentable(x[0], sud.common))}）")
        for w, tf, df, cue in pick:
            print(f"{w}\t{tf}\t{df}\t{'F5' if L.segmentable(w, sud.common) else ''}")


if __name__ == "__main__":
    main()
