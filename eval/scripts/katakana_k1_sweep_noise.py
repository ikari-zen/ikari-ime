#!/usr/bin/env python3
"""ADR 0037 §6.3: ノイズの量（捨てるスクリプト）。

S0 の守る群の読み (katakana_s0_cases.py の guard.tsv) を CLI で単独に変換し、基準 (K1 無効) と点を比べる。
  top1_kata   1 位が origin=katakana (辞書にない語のカタカナの候補) に変わった読み
  top3_kata   最初の 3 件に origin=katakana の候補が新しく入った読み (基準の 3 件にはなかった)
結果は JSON (群ごとの件数と、変わった読みの全件)。

使い方: katakana_k1_sweep_noise.py --cli <ikari> --dict <dir> --user-dir <空 dir> --cases <guard.tsv>
          --out <json> [--kv <ranker-config>] [--base-json <基準の json>]
"""
import argparse
import json
import subprocess
from concurrent.futures import ThreadPoolExecutor

NUM = 3
KATAKANA_ORIGIN = "katakana"


def run_one(a, case):
    group, cid, ctx, reading, expected = case
    cmd = [a.cli, "--dict", a.dict, "--user-dir", a.user_dir, "--ranker", "full", "--format", "tsv", "-n", str(NUM)]
    if a.kv:
        cmd += ["--ranker-config", a.kv]
    if ctx:
        cmd += ["--context", ctx]
    cmd += ["--", reading]
    p = subprocess.run(cmd, capture_output=True, text=True)
    rows = [l.split("\t") for l in p.stdout.splitlines()]
    rows = [r for r in rows if len(r) >= 5]
    return {"group": group, "reading": reading, "context": ctx,
            "top": [(r[1], r[4]) for r in rows[:NUM]]}


def group_of(g):
    return "sets" if g.startswith("guard-set:") else g


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", required=True)
    ap.add_argument("--dict", required=True)
    ap.add_argument("--user-dir", required=True)
    ap.add_argument("--cases", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--kv")
    ap.add_argument("--base-json")
    ap.add_argument("--jobs", type=int, default=8)
    a = ap.parse_args()
    cases = []
    with open(a.cases, encoding="utf-8") as f:
        for line in f:
            p = line.rstrip("\n").split("\t")
            if len(p) == 5:
                cases.append(tuple(p))
    with ThreadPoolExecutor(a.jobs) as ex:
        res = list(ex.map(lambda c: run_one(a, c), cases))
    out = {"n": len(res), "rows": res}
    if a.base_json:
        base = {(r["context"], r["reading"], r["group"]): r for r in json.load(open(a.base_json, encoding="utf-8"))["rows"]}
        counts, examples = {}, {"top1_kata": [], "top3_kata": []}
        for r in res:
            b = base[(r["context"], r["reading"], r["group"])]
            g = group_of(r["group"])
            c = counts.setdefault(g, {"n": 0, "top1_kata": 0, "top3_kata": 0})
            c["n"] += 1
            t1 = bool(r["top"]) and r["top"][0][1] == KATAKANA_ORIGIN
            b1 = bool(b["top"]) and b["top"][0][1] == KATAKANA_ORIGIN
            new3 = any(o == KATAKANA_ORIGIN for _, o in r["top"]) and not any(o == KATAKANA_ORIGIN for _, o in b["top"])
            if t1 and not b1:
                c["top1_kata"] += 1
                examples["top1_kata"].append((r["group"], r["reading"], b["top"][0][0] if b["top"] else "", r["top"][0][0]))
            if new3:
                c["top3_kata"] += 1
                kat = [s for s, o in r["top"] if o == KATAKANA_ORIGIN][0]
                examples["top3_kata"].append((r["group"], r["reading"], b["top"][0][0] if b["top"] else "", kat))
        out["counts"] = counts
        out["examples"] = examples
        out["top1_kata_total"] = sum(c["top1_kata"] for c in counts.values())
        out["top3_kata_total"] = sum(c["top3_kata"] for c in counts.values())
    with open(a.out, "w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False)


if __name__ == "__main__":
    main()
