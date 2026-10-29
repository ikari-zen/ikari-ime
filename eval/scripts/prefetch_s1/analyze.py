#!/usr/bin/env python3
"""ADR 0038 §6.2 S1: run.sh の出力（target/prefetch-s1）を集計して、Markdown の表を標準出力に出す（捨てるスクリプト）。

使い方: analyze.py [<out> (既定 target/prefetch-s1)]
「期待 → 外れ」= 比べる前（左）の腕で 1 位が期待、比べる後（右）の腕で 1 位が期待でない。
「外れ → 期待」= その逆。ケースの 1 位は --out-cases の rank 列が 1 かどうかで見る。
腕: X = 元のセット・LM なし、Y = stale・LM あり、Z = 元のセット・LM あり。
重み付き: 取り除いた単位の長さ L ごとの率を、重み w_L（診断ログの commit_chars の分布）で平均する。
  ケースのない L は除いて重みを正規化する。
"""
import collections
import csv
import glob
import gzip
import os
import sys

OUT = sys.argv[1] if len(sys.argv) > 1 else "target/prefetch-s1"
MAX_LEN = 12
LOG_DIR = os.path.expanduser("~/Library/Application Support/ikari-ime/logs/")

# 測定 (a) の重み（issue #1。2026-10-28 時点の診断ログ。convert の dleft == commit_chars）
W_A = {3: 21, 4: 12, 2: 12, 5: 5, 7: 4, 1: 3, 6: 1, 11: 1}


def log_weights():
    """診断ログの ctx_change から commit_chars の分布を数える。(convert のみ, 全 req) の 2 つ。dleft == commit_chars > 0 の行だけ。"""
    conv, allr = collections.Counter(), collections.Counter()
    for f in sorted(glob.glob(LOG_DIR + "engine*.log*")):
        op = gzip.open if f.endswith(".gz") else open
        with op(f, "rt", errors="replace") as fh:
            for line in fh:
                if " ctx_change " not in line:
                    continue
                kv = dict(x.split("=", 1) for x in line.split() if "=" in x)
                cc = kv.get("commit_chars", "0")
                if cc != "0" and kv.get("dleft", "").lstrip("+") == cc:
                    allr[int(cc)] += 1
                    if kv.get("req") == "convert":
                        conv[int(cc)] += 1
    return conv, allr


def load_cases(path):
    d = {}
    with open(path, encoding="utf-8") as fh:
        for row in csv.DictReader(fh, delimiter="\t", quoting=csv.QUOTE_NONE):
            d[row["id"]] = int(row["rank"]) == 1
    return d


def main():
    conv, allr = log_weights()
    weights = {"a(測定 a の分布)": W_A, "log(convert)": conv, "log(全 req)": allr}
    for scheme in ("A", "B"):
        units = []
        with open(f"{OUT}/units-{scheme}.tsv", encoding="utf-8") as fh:
            next(fh)
            for line in fh:
                s, i, n, u = line.rstrip("\n").split("\t")
                units.append((s, i, int(n)))
        top = {"X": {}, "Y": {}, "Z": {}}
        for s in sorted({u[0] for u in units}):
            top["X"][s] = load_cases(f"{OUT}/x/{s}.tsv")
            top["Z"][s] = load_cases(f"{OUT}/z/{s}.tsv")
            p = f"{OUT}/y{scheme}/{s}.tsv"
            top["Y"][s] = load_cases(p) if os.path.exists(p) else {}
        rows = [(s, i, n, top["X"][s][i], top["Y"][s][i], top["Z"][s][i]) for s, i, n in units if i in top["Y"][s]]
        print(f"\n## 切り方 {scheme}: {len(rows)} 件（units {len(units)}）\n")
        for name, a, b in (("Y 対 X（案 1 対 今）", 3, 4), ("Z 対 X（案 4 の上限対 今）", 3, 5), ("Z 対 Y（案 4 の上限対 案 1）", 4, 5)):
            print(f"### {scheme}: {name}\n")
            # 全体と長さごと
            by_len = collections.defaultdict(lambda: [0, 0, 0, 0])  # n, e->m, m->e, 前の腕の正解数
            by_set = collections.defaultdict(lambda: [0, 0, 0, 0])
            for r in rows:
                for dct, key in ((by_len, min(r[2], MAX_LEN)), (by_set, r[0])):
                    c = dct[key]
                    c[0] += 1
                    c[3] += r[a]
                    if r[a] and not r[b]:
                        c[1] += 1
                    if not r[a] and r[b]:
                        c[2] += 1
            tot = [sum(c[k] for c in by_len.values()) for k in range(4)]
            print(f"全体: n={tot[0]}、前の腕の正解 {tot[3]}、期待→外れ {tot[1]}、外れ→期待 {tot[2]}、差し引き（外れ→期待 − 期待→外れ）{tot[2]-tot[1]:+d}\n")
            print("セットごと\n\n| セット | 件数 | 期待→外れ | 外れ→期待 | 差し引き |\n|---|---|---|---|---|")
            for s in sorted(by_set):
                c = by_set[s]
                print(f"| {s} | {c[0]} | {c[1]} | {c[2]} | {c[2]-c[1]:+d} |")
            print("\n単位の長さごと\n\n| 長さ | 件数 | 期待→外れ | 外れ→期待 | 差し引き | 累積の差し引き |\n|---|---|---|---|---|---|")
            cum = 0
            for L in range(1, MAX_LEN + 1):
                c = by_len.get(L)
                if not c:
                    print(f"| {L} | 0 | - | - | - | - |")
                    continue
                cum += c[2] - c[1]
                print(f"| {L}{'+' if L == MAX_LEN else ''} | {c[0]} | {c[1]} | {c[2]} | {c[2]-c[1]:+d} | {cum:+d} |")
            print("\n重み付き（長さごとの率を重みで平均。単位は %）\n\n| 重み | 期待→外れ | 外れ→期待 | 差し引き（外れ→期待 − 期待→外れ）|\n|---|---|---|---|")
            for wn, w in weights.items():
                num_e = num_m = den = 0.0
                for L, c in by_len.items():
                    wl = w.get(L, 0)
                    if wl and c[0]:
                        num_e += wl * c[1] / c[0]
                        num_m += wl * c[2] / c[0]
                        den += wl
                if den:
                    print(f"| {wn} | {100*num_e/den:.1f} | {100*num_m/den:.1f} | {100*(num_m-num_e)/den:+.1f} |")
            print()
        # D_max の候補（Y 対 X）
        by_len = collections.defaultdict(lambda: [0, 0, 0])
        for r in rows:
            c = by_len[min(r[2], MAX_LEN)]
            c[0] += 1
            c[1] += r[3] and not r[4]
            c[2] += (not r[3]) and r[4]
        nets = {L: c[2] - c[1] for L, c in by_len.items()}
        ok_each = [L for L in sorted(nets) if nets[L] >= 0]
        # 1 字から順に、累積の差し引きが 0 以上である最大の L
        cum, dmax_cum = 0, 0
        for L in range(1, MAX_LEN + 1):
            if L in nets:
                cum += nets[L]
            if cum >= 0:
                dmax_cum = L
        # 1 字からずっと 1 つ 1 つの長さで差し引き 0 以上が続く最大の L
        dmax_each = 0
        for L in range(1, MAX_LEN + 1):
            if L in nets and nets[L] < 0:
                break
            dmax_each = L
        print(f"**{scheme} の D_max の候補（Y 対 X）**: 長さごとの差し引きが 0 以上の長さ = {ok_each}。1 字から途切れず 0 以上が続く最大の長さ = {dmax_each}。累積の差し引きが 0 以上である最大の長さ = {dmax_cum}\n")
    print("\n重みの分布\n")
    for wn, w in weights.items():
        print(f"- {wn}: n={sum(w.values())} {sorted(w.items())}")


main()
