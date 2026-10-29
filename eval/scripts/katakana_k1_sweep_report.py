#!/usr/bin/env python3
"""sweep.jsonl を読んで、不採用の内訳と、不採用でない点の一覧を出す（捨てるスクリプト）。

使い方: katakana_k1_sweep_report.py <sweep.jsonl>"""
import json
import sys
from collections import Counter


def load(path):
    return [json.loads(l) for l in open(path, encoding="utf-8")]


def rejected(r):
    """不採用の理由の一覧 (空なら不採用でない)"""
    why = [name for name, v in r["sets"].items() if v["reg"] > 0]
    if r.get("loo_guard_reg", 0) > 0:
        why.append("loo(疑似の守る側)")
    if r["errs"]:
        why.append("error")
    return why


def main():
    rs = load(sys.argv[1])
    print("点の数", len(rs))
    ok = [r for r in rs if not rejected(r)]
    print("不採用でない点", len(ok))
    cnt = Counter()
    only = Counter()
    for r in rs:
        why = rejected(r)
        for w in why:
            cnt[w] += 1
        if len(why) == 1:
            only[why[0]] += 1
    print("理由ごとの点の数 (複数該当あり):")
    for k, v in cnt.most_common():
        print(f"  {k}: {v}（これだけで落ちた点 {only[k]}）")
    by = Counter((r["strong"], r["weak"]) for r in rs)
    byok = Counter((r["strong"], r["weak"]) for r in ok)
    for k in sorted(by):
        print(f"  strong={k[0]} weak={k[1]}: 点 {by[k]}、不採用でない {byok[k]}")
    print()
    print("不採用でない点 (kl_target_top1 / main_dev_top1 の降順)")
    print("base per_char strong weak | kl_top1/kl_n | main_top1/dev_n (基準) | ng shon jon unsplit | hl_imp | real(01,02)")
    for r in sorted(ok, key=lambda r: (-r.get("kl_target_top1", 0), -r.get("main_dev_top1", 0), -r["per_char"])):
        print(r["base"], r["per_char"], r["strong"], r["weak"], "|", f'{r["kl_target_top1"]}/{r["kl_target_n"]} (基準 {r["kl_target_top1_base"]})', "|",
              f'{r["main_dev_top1"]}/{r["main_dev_n"]} ({r["main_dev_top1_base"]})', "|",
              r["main_ng_top1"], r["main_shon_top1"], r["main_jon_top1"], r["main_unsplit_top1"], "|", r["hl_imp"], "|",
              r["real"].get("kl-split-01"), r["real"].get("kl-split-02"))


if __name__ == "__main__":
    main()
