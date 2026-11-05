#!/usr/bin/env python3
"""ken-sweep.sh の結果（<work>/results）を、基準（0:0）に対する表にする（ADR 0054 §5）。

    python3 -I eval/scripts/ken_sweep_summary.py --work <scratch> [--base n0r0] [--detail n8500r0 --config cost]

表:
  - 変種ごと・構成ごとの「1 位の悪化」（基準で 1 位だったケースが 1 位でなくなった数）と「改善」の数（全セットの合計）
  - issue210 の ken-matter（〜の件）の 1 位の数と順位、ken-guard、ken-pref、ken-rentai
  - 文節の区切りのレポートが基準と一致するか
--detail を付けると、その変種・構成の悪化したケースを一覧にする。
"""

import argparse
import re
from pathlib import Path

NO_RANK = "-"


def read_cases(path):
    """--out-cases の TSV -> {(id): (expected, top1, rank or None)}"""
    cases = {}
    lines = path.read_text(encoding="utf-8").splitlines()
    for line in lines[1:]:
        f = line.split("\t")
        if len(f) < 7:
            continue
        rank = int(f[6]) if f[6].strip().isdigit() else None
        cases[f[0]] = (f[4], f[5], rank, f[3])
    return cases


def read_tags(path):
    """評価セット -> {id: tags}"""
    lines = path.read_text(encoding="utf-8").splitlines()
    header = None
    tags = {}
    for line in lines:
        if line.startswith("#") or not line.strip():
            continue
        f = line.split("\t")
        if header is None:
            header = f
            continue
        if "tags" in header and len(f) > header.index("tags"):
            tags[f[0]] = f[header.index("tags")].split(",")
    return tags


def variants(results):
    return sorted(
        (d.name for d in results.iterdir() if d.is_dir()),
        key=lambda n: tuple(int(x) for x in re.findall(r"\d+", n)),
    )


def compare(base_dir, var_dir):
    """セットごとに (悪化[(set,id,...)], 改善の数)。"""
    worse, better = [], 0
    for base_tsv in sorted(base_dir.glob("*.tsv")):
        var_tsv = var_dir / base_tsv.name
        if not var_tsv.exists():
            continue
        base, var = read_cases(base_tsv), read_cases(var_tsv)
        for case_id, (expected, top1, rank, reading) in base.items():
            if case_id not in var:
                continue
            _, top1_v, rank_v, _ = var[case_id]
            if rank == 1 and rank_v != 1:
                worse.append((base_tsv.stem, case_id, reading, expected, top1_v, rank_v))
            elif rank != 1 and rank_v == 1:
                better += 1
    return worse, better


def issue210(var_dir, sets_dir):
    tags = read_tags(sets_dir / "issue210.tsv")
    cases = read_cases(var_dir / "issue210.tsv")
    out = {}
    for tag in ("ken-matter", "ken-guard", "ken-pref", "ken-rentai"):
        ids = [i for i, t in tags.items() if tag in t and i in cases]
        top1 = sum(1 for i in ids if cases[i][2] == 1)
        out[tag] = (top1, len(ids), [cases[i][2] for i in ids])
    return out, cases


def seg_report(path):
    if not path.exists():
        return None
    text = path.read_text(encoding="utf-8")
    # 応答時間の行は揺れるので、時間の節より前だけを比べる
    return text.split("## 応答時間")[0]


def fmt_rank(r):
    return NO_RANK if r is None else str(r)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", required=True, type=Path)
    ap.add_argument("--base", default="n0r0")
    ap.add_argument("--detail", default=None)
    ap.add_argument("--config", default="cost")
    ap.add_argument("--sets", type=Path, default=Path(__file__).resolve().parent.parent / "sets")
    args = ap.parse_args()
    results = args.work / "results"
    names = variants(results)
    configs = sorted({c.name for v in names for c in (results / v).iterdir() if (c / "DONE").exists()})

    print("## 1 位の悪化（基準 %s で 1 位 → 1 位でない）／改善（その逆）。全セットと AJIMEE の合計\n" % args.base)
    print("| 変種（の:連体詞） | " + " | ".join(f"{c} 悪化 / 改善" for c in configs) + " | segments |")
    print("|---|" + "---|" * (len(configs) + 1))
    for v in names:
        cells = []
        seg_same = []
        for c in configs:
            base_dir, var_dir = results / args.base / c, results / v / c
            if not (var_dir / "DONE").exists() or not (base_dir / "DONE").exists():
                cells.append("-")
                continue
            worse, better = compare(base_dir, var_dir)
            cells.append(f"{len(worse)} / {better}")
            if v != args.base:
                seg_same.append(seg_report(base_dir / "segments.md") == seg_report(var_dir / "segments.md"))
        seg = "一致" if seg_same and all(seg_same) else ("相違" if seg_same else "-")
        print(f"| {v} | " + " | ".join(cells) + f" | {seg} |")

    for c in configs:
        print(f"\n## issue210（構成 {c}）。1 位の数 / 件数、順位（`-` は圏外）\n")
        print("| 変種 | ken-matter | ken-guard | ken-pref | ken-rentai | ken-matter の順位 |")
        print("|---|---|---|---|---|---|")
        for v in names:
            d = results / v / c
            if not (d / "DONE").exists():
                continue
            out, _ = issue210(d, args.sets)
            ranks = ",".join(fmt_rank(r) for r in out["ken-matter"][2])
            print(
                f"| {v} | {out['ken-matter'][0]}/{out['ken-matter'][1]} | {out['ken-guard'][0]}/{out['ken-guard'][1]}"
                f" | {out['ken-pref'][0]}/{out['ken-pref'][1]} | {out['ken-rentai'][0]}/{out['ken-rentai'][1]} | {ranks} |"
            )

    if args.detail:
        base_dir = results / args.base / args.config
        var_dir = results / args.detail / args.config
        worse, _ = compare(base_dir, var_dir)
        print(f"\n## 悪化したケース（{args.detail}、構成 {args.config}）: {len(worse)} 件\n")
        print("| セット | id | 読み | 期待 | 変種の 1 位 | 順位 |")
        print("|---|---|---|---|---|---|")
        for s, i, reading, expected, top1, rank in worse:
            print(f"| {s} | {i} | {reading} | {expected} | {top1} | {fmt_rank(rank)} |")


if __name__ == "__main__":
    main()
