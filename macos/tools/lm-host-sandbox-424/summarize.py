#!/usr/bin/env python3
# スパイク #424（マージしない）。target/m424/*.txt の要約（回ごとの表と、形ごとの平均・範囲）
import re
import statistics
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[3] / "target" / "m424"
rows = []
for p in sorted(root.glob("*-*-*.txt")):
    if p.name.startswith("trial"):
        continue
    s = p.read_text()
    head = re.search(r"kind=(\w+) gpu=(\w+) n=(\w+) start=(\d+) uptime=.*load averages: ([\d.]+)", s)
    end = re.search(r"# end=(\d+) uptime=.*load averages: ([\d.]+)", s)
    lm = re.search(r"呼び出し (\d+) / 時間切れ (\d+) \(([\d.]+)%\) / 遅れて届いた結果の利用 (\d+)", s)
    warm = re.search(r"^\| warm \| (\d+) \| ([\d.]+) \| ([\d.]+) \| ([\d.]+) \|", s, re.M)
    cold = re.search(r"^\| cold[^|]*\| (\d+) \| ([\d.]+) \| ([\d.]+) \| ([\d.]+) \|", s, re.M)
    top = re.search(r"^\| 全体 \| (\d+) \| (\d+) \|", s, re.M)
    if not (head and end and lm and warm and top):
        print(f"skip {p.name}", file=sys.stderr)
        continue
    rows.append(
        dict(
            kind=head[1], gpu=head[2], n=head[3], secs=int(end[1]) - int(head[4]),
            load0=float(head[5]), load1=float(end[2]),
            calls=int(lm[1]), to=int(lm[2]), to_pct=float(lm[3]),
            p50=float(warm[2]), p99=float(warm[3]), wmax=float(warm[4]),
            cp99=float(cold[3]) if cold else 0, top1=int(top[2]),
        )
    )
print("| 形 | GPU | 回 | 秒 | load 始/終 | warm p50 | warm p99 | warm 最悪 | cold p99 | 時間切れ | 1位正解 |")
print("|---|---|---|---|---|---|---|---|---|---|---|")
for r in sorted(rows, key=lambda r: (r["gpu"], r["kind"], r["n"])):
    print(f"| {r['kind']} | {r['gpu']} | {r['n']} | {r['secs']} | {r['load0']:.1f}/{r['load1']:.1f} | {r['p50']:.2f} | "
          f"{r['p99']:.2f} | {r['wmax']:.1f} | {r['cp99']:.1f} | {r['to_pct']:.1f}% ({r['to']}) | {r['top1']} |")
print()
print("| 形 | GPU | 回数 | warm p50 平均 [範囲] | warm p99 平均 [範囲] (標準偏差) | 時間切れ% 平均 [範囲] | 1位正解 [範囲] |")
print("|---|---|---|---|---|---|---|")
groups = {}
for r in rows:
    groups.setdefault((r["gpu"], r["kind"]), []).append(r)
for (gpu, kind), rs in sorted(groups.items()):
    def agg(k, f="{:.2f}"):
        xs = [r[k] for r in rs]
        return f"{f.format(statistics.mean(xs))} [{f.format(min(xs))}–{f.format(max(xs))}]"
    sd = statistics.pstdev([r["p99"] for r in rs])
    print(f"| {kind} | {gpu} | {len(rs)} | {agg('p50')} | {agg('p99')} ({sd:.2f}) | {agg('to_pct', '{:.1f}')} | "
          f"{min(r['top1'] for r in rs)}–{max(r['top1'] for r in rs)} |")
