#!/usr/bin/env python3
"""S1 の腕ごとの --out-cases を、切り方ごとに 1 本へまとめ、`ikari-eval compare` に渡す（捨てるスクリプト）。
使い方: compare.py [<out>]   出力: <out>/cmp/{x,y,z}-{A,B}.tsv と、compare の標準出力 <out>/cmp/{Y-X,Z-X,Z-Y}-{A,B}.txt
"""
import os
import subprocess
import sys

OUT = sys.argv[1] if len(sys.argv) > 1 else "target/prefetch-s1"
os.makedirs(f"{OUT}/cmp", exist_ok=True)
for k in ("A", "B"):
    ids = {}
    with open(f"{OUT}/units-{k}.tsv", encoding="utf-8") as fh:
        next(fh)
        for line in fh:
            s, i, *_ = line.split("\t")
            if s != "set":
                ids.setdefault(s, set()).add(i)
    header = None
    for arm, d in (("x", "x"), ("y", f"y{k}"), ("z", "z")):
        rows = []
        for s, want in sorted(ids.items()):
            p = f"{OUT}/{d}/{s}.tsv"
            if not os.path.exists(p):
                continue
            lines = open(p, encoding="utf-8").read().split("\n")
            header = lines[0]
            rows += [l for l in lines[1:] if l and l.split("\t")[0] in want]
        with open(f"{OUT}/cmp/{arm}-{k}.tsv", "w", encoding="utf-8") as fh:
            fh.write(header + "\n" + "\n".join(rows) + "\n")
    for name, a, b in (("Y-X", "x", "y"), ("Z-X", "x", "z"), ("Z-Y", "y", "z")):
        r = subprocess.run(["target/release/ikari-eval", "compare", f"{OUT}/cmp/{a}-{k}.tsv", f"{OUT}/cmp/{b}-{k}.tsv"], capture_output=True, text=True)
        with open(f"{OUT}/cmp/{name}-{k}.txt", "w", encoding="utf-8") as fh:
            fh.write(r.stdout + r.stderr)
