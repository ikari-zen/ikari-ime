#!/usr/bin/env python3
"""ADR 0037 §6.1 S0: 測る読みの一覧を作る（捨てるスクリプト）。出力は collect 用の TSV（group id context reading expected）。

  guard.tsv   守る群: eval/sets のうち期待がカタカナでないケース全部 + Sudachi Core の漢字・かなの語の読みの乱数抽出
              + 補助の 3 群（ぐ で終わる和語、反復の読み＝擬音語・擬態語、長音を含む和語）
  target.tsv  狙う群（ふつうの辞書で測る）: katakana-like の カタカナ語(弱い) 13 本 + 本物の未知語
  loo.tsv     狙う群（疑似。除外表で絞った辞書で測る）: katakana_s0_prepare.py の loo.tsv から

乱数のライブラリは使わない。選び方は FNV-1a64(読み + 種の文字列) の小さい順に N 件。
"""
import argparse
import csv
import glob
import os

SEED = "ikari-katakana-s0-v1"
N_CORE = 3000
N_SUB = 300
FNV_OFFSET, FNV_PRIME, MASK = 0xCBF29CE484222325, 0x100000001B3, (1 << 64) - 1
CORE = "core_lex.csv"
ALL_LEX = ["small_lex.csv", "core_lex.csv", "notcore_lex.csv"]
# 本物の未知語（ADR 0037 §6.4 の狙う群。文脈つきは公開の一般的な文）
REAL = [("real-01", "", "ばーじょにんぐ", "バージョニング"), ("real-02", "APIの", "ばーじょにんぐ", "バージョニング"),
        ("real-03", "", "しゃーでぃんぐ", "シャーディング")]


def fnv(s: str) -> int:
    h = FNV_OFFSET
    for b in s.encode():
        h = ((h ^ b) * FNV_PRIME) & MASK
    return h


def is_hira(s):
    return bool(s) and all("ぁ" <= c <= "ゖ" or c == "ー" for c in s)


def is_kata_only(s):
    return bool(s) and all("ァ" <= c <= "ヺ" or c in "ー・" for c in s)


def to_hira(s):
    return "".join(chr(ord(c) - 0x60) if "ァ" <= c <= "ヶ" else c for c in s)


def is_redup(r):
    # 反復: 全体が同じ半分の繰り返し（ぴかぴか）、または 3 字以上の同一語の繰り返し（ふふふ）
    n = len(r)
    if n >= 4 and n % 2 == 0 and r[: n // 2] == r[n // 2:]:
        return True
    return n >= 3 and len(set(r)) == 1


def lex_words(sudachi, names):
    """(表記, ひらがなの読み) の集合のうち、表記がカタカナだけでも英字だけでもない和語・漢語の側"""
    seen = set()
    for name in names:
        with open(os.path.join(sudachi, name), encoding="utf-8", newline="") as f:
            for line in f:
                row = next(csv.reader([line]))
                if len(row) < 17:
                    continue
                s, r = row[0], to_hira(row[11])
                if is_kata_only(s) or not is_hira(r) or len(r) < 2:
                    continue
                if all(ord(c) < 128 for c in s):
                    continue
                seen.add((s, r))
    return seen


def pick(readings, n, taken):
    cand = sorted((r for r in set(readings) if r not in taken), key=lambda r: (fnv(r + SEED), r))
    return cand[:n]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True)
    ap.add_argument("--sudachi", required=True)
    ap.add_argument("--loo", required=True, help="katakana_s0_prepare.py が作った loo.tsv")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()

    # --- 守る群 1: eval/sets
    guard, seen = [], set()
    n_skipped = 0
    for path in sorted(glob.glob(os.path.join(a.repo, "eval/sets/*.tsv"))):
        name = os.path.basename(path)[:-4]
        with open(path, encoding="utf-8") as f:
            lines = [l.rstrip("\n") for l in f if not l.startswith("#") and l.strip()]
        head = lines[0].split("\t")
        for l in lines[1:]:
            p = dict(zip(head, l.split("\t")))
            exp = [e for e in p.get("expected", "").split("|") if e]
            reading, ctx = p.get("reading", ""), p.get("context", "")
            if not exp or any(is_kata_only(e) for e in exp) or not is_hira(reading):
                n_skipped += 1
                continue
            key = (ctx, reading)
            if key in seen:
                continue
            seen.add(key)
            guard.append((f"guard-set:{name}", p.get("id", ""), ctx, reading, "|".join(exp)))
    n_sets = len(guard)

    # --- 守る群 2/3: Sudachi の乱数抽出
    core = lex_words(a.sudachi, [CORE])
    allw = lex_words(a.sudachi, ALL_LEX)
    taken = {r for (_, _, _, r, _) in guard}
    surf_of = {}
    for s, r in sorted(allw):
        surf_of.setdefault(r, s)
    core_readings = [r for (_, r) in core if len(r) >= 2]
    sample = pick(core_readings, N_CORE, taken)
    taken |= set(sample)
    for r in sample:
        guard.append(("guard-core", f"core-{r}", "", r, surf_of.get(r, "")))
    subs = {
        "guard-gu": [r for r in surf_of if r.endswith("ぐ") and len(r) >= 3],
        "guard-redup": [r for r in surf_of if is_redup(r) and len(r) >= 3],
        "guard-choon": [r for r in surf_of if "ー" in r and len(r) >= 3],
    }
    sub_n = {}
    for g, rs in subs.items():
        chosen = pick(rs, N_SUB, taken)
        sub_n[g] = (len(set(rs)), len(chosen))
        taken |= set(chosen)
        for r in chosen:
            guard.append((g, f"{g}-{r}", "", r, surf_of.get(r, "")))

    # --- 狙う群（ふつうの辞書）
    target = list(REAL_CASES(REAL))
    with open(os.path.join(a.repo, "eval/sets/katakana-like.tsv"), encoding="utf-8") as f:
        lines = [l.rstrip("\n") for l in f if not l.startswith("#") and l.strip()]
    head = lines[0].split("\t")
    for l in lines[1:]:
        p = dict(zip(head, l.split("\t")))
        if p["kind"] == "カタカナ語(弱い)":
            target.append(("target-weak", p["id"], p["context"], p["reading"], p["expected"]))

    # --- 疑似
    loo = []
    with open(a.loo, encoding="utf-8") as f:
        head = f.readline().rstrip("\n").split("\t")
        for l in f:
            p = dict(zip(head, l.rstrip("\n").split("\t")))
            loo.append(("loo+" + p["tags"].replace(",", "+"), p["id"], "", p["reading"], p["expected"]))

    def write(name, rows):
        with open(os.path.join(a.out, name), "w", encoding="utf-8") as fo:
            for r in rows:
                fo.write("\t".join(r) + "\n")

    write("guard.tsv", guard)
    write("target.tsv", target)
    write("loo.tsv", loo)
    print(f"guard: sets {n_sets}（期待がカタカナ・読みがかな以外で外したもの {n_skipped}）、core {len(sample)}（母集団 {len(core_readings)}）、補助 {sub_n}")
    print(f"target: {len(target)}、loo: {len(loo)}")


def REAL_CASES(real):
    for cid, ctx, r, e in real:
        yield ("target-real", cid, ctx, r, e)


if __name__ == "__main__":
    main()
