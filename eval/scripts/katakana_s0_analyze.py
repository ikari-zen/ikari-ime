#!/usr/bin/env python3
"""ADR 0037 §6.1 S0: collect の出力から、分布・ROC・K6（並びの表）の分離を出す（捨てるスクリプト）。

出力は markdown の断片（標準出力）。表を作る語から、除外表（leave-one-out の除外語）と、評価の読み（eval/sets の
全ケースの読み、狙う群、守る群の乱数抽出の読み）を除く（ADR 0037 §5、§6.1）。
  --table-out <TSV>   K6 の表（並び -> 値）を書き出す（判定 1 回の時間を測る Rust の捨てコード用）
"""
import argparse
import csv
import glob
import math
import os
import sys
import time
from bisect import bisect_left, bisect_right
from collections import Counter, defaultdict

SMALL_JOIN = set("ゃゅょぁぃぅぇぉゎ")
ALPHA = 0.5


def is_hira(s):
    return bool(s) and all("ぁ" <= c <= "ゖ" or c == "ー" for c in s)


def is_kata_only(s):
    return bool(s) and all("ァ" <= c <= "ヺ" or c == "ー" for c in s)


def to_hira(s):
    return "".join(chr(ord(c) - 0x60) if "ァ" <= c <= "ヶ" else c for c in s)


def units_char(r):
    return list(r)


def units_mora(r):
    u = []
    for c in r:
        if c in SMALL_JOIN and u:
            u[-1] += c
        else:
            u.append(c)
    return u


def gram_keys(units):
    p = ["^"] + units + ["$"]
    out = []
    for n in (2, 3):
        for i in range(len(p) - n + 1):
            out.append("\u0001".join(p[i:i + n]))
    return out


# ---------------------------------------------------------------- 読み込み
def load_out(path):
    with open(path, encoding="utf-8") as f:
        head = f.readline().rstrip("\n").split("\t")
        return [dict(zip(head, l.rstrip("\n").split("\t"))) for l in f]


def fnum(x):
    return float(x) if x not in ("", None) else None


def lex_pairs(sudachi):
    foreign, native = set(), set()
    for name in ("small_lex.csv", "core_lex.csv", "notcore_lex.csv"):
        with open(os.path.join(sudachi, name), encoding="utf-8", newline="") as f:
            for line in f:
                row = next(csv.reader([line]))
                if len(row) < 17:
                    continue
                s, r = row[0], to_hira(row[11])
                if not is_hira(r) or len(r) < 2:
                    continue
                if is_kata_only(s):
                    foreign.add((s, r))
                elif not all(ord(c) < 128 for c in s):
                    native.add((s, r))
    return foreign, native


def build_table(foreign, native, drop_readings, drop_pairs, unit_fn):
    """読みごとに 1 回数える（同じ読みの別表記で水増ししない）"""
    fr = {r for (s, r) in foreign if r not in drop_readings and (s, r) not in drop_pairs}
    nr = {r for (s, r) in native if r not in drop_readings and (s, r) not in drop_pairs}
    cf, cn = Counter(), Counter()
    for r in fr:
        cf.update(set(gram_keys(unit_fn(r))))
    for r in nr:
        cn.update(set(gram_keys(unit_fn(r))))
    keys = set(cf) | set(cn)
    tf, tn = sum(cf.values()), sum(cn.values())
    v = len(keys) + 1
    table = {}
    for k in keys:
        table[k] = math.log((cf[k] + ALPHA) / (tf + ALPHA * v)) - math.log((cn[k] + ALPHA) / (tn + ALPHA * v))
    unseen = math.log(ALPHA / (tf + ALPHA * v)) - math.log(ALPHA / (tn + ALPHA * v))
    return table, unseen, len(fr), len(nr)


def k6(table, unseen, r, unit_fn):
    ks = gram_keys(unit_fn(r))
    return sum(table.get(k, unseen) for k in ks) / len(ks)


# ---------------------------------------------------------------- 統計
def quantiles(xs, qs=(0.05, 0.25, 0.5, 0.75, 0.95)):
    if not xs:
        return [None] * len(qs)
    s = sorted(xs)
    return [s[min(len(s) - 1, int(q * len(s)))] for q in qs]


def auc(pos, neg):
    """pos の値が大きいほど『狙う』。Mann-Whitney（同値は 0.5）"""
    if not pos or not neg:
        return float("nan")
    sn = sorted(neg)
    tot = 0.0
    for p in pos:
        tot += bisect_left(sn, p) + 0.5 * (bisect_right(sn, p) - bisect_left(sn, p))
    return tot / (len(pos) * len(neg))


def tpr_at_fpr(pos, neg, fpr):
    """neg の上位 fpr だけが越える閾値で、pos のうち閾値を超える割合と、その閾値"""
    sn = sorted(neg)
    k = int(len(sn) * (1 - fpr))
    thr = sn[min(k, len(sn) - 1)]
    return sum(1 for p in pos if p > thr) / len(pos), thr


def fmt(x, d=1):
    return "-" if x is None else (f"{x:.{d}f}" if isinstance(x, float) else str(x))


def qrow(name, xs, d=1):
    q = quantiles(xs)
    return f"| {name} | {len(xs)} | " + " | ".join(fmt(v, d) for v in q) + " |"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True)
    ap.add_argument("--sudachi", required=True)
    ap.add_argument("--run", required=True, help="collect の出力があるディレクトリ")
    ap.add_argument("--loo", required=True, help="prepare の出力ディレクトリ（exclusions.tsv）")
    ap.add_argument("--table-out")
    a = ap.parse_args()

    guard = load_out(os.path.join(a.run, "guard.out.tsv"))
    target = load_out(os.path.join(a.run, "target.out.tsv"))
    loo = load_out(os.path.join(a.run, "loo.out.tsv"))

    for r in guard + target + loo:
        r["n"], r["mora"] = int(r["n"]), int(r["mora"])
        r["dict_min"] = fnum(r["dict_min"])
        r["kana_cost"], r["kata_cost"] = fnum(r["kana_cost"]), fnum(r["kata_cost"])
        r["x1"] = r["dict_min"] / r["n"] if r["dict_min"] is not None else None
        r["x1m"] = r["dict_min"] / max(1, r["mora"]) if r["dict_min"] is not None else None
        r["exp_rank_i"] = int(r["exp_rank"]) if r["exp_rank"] else None
        r["kata_rank_i"] = int(r["kata_rank"]) if r["kata_rank"] else None

    # ---- 群の定義
    def is_gu(r):
        return r["reading"].endswith("ぐ")

    def is_red(r):
        s = r["reading"]
        n = len(s)
        return (n >= 4 and n % 2 == 0 and s[: n // 2] == s[n // 2:]) or (n >= 3 and len(set(s)) == 1)

    def is_choon(r):
        return "ー" in r["reading"]

    def is_onomato_set(r):
        return r["id"].startswith("kl-guard")

    g_all = [r for r in guard if r["x1"] is not None]
    g_core = [r for r in g_all if r["group"] == "guard-core"]
    g_sets = [r for r in g_all if r["group"].startswith("guard-set:")]
    g_len6 = [r for r in g_all if r["n"] >= 6]
    # 狙う群の主: 同音の別表記が残る語（homophone-left。ADR 0020 の英字 → カタカナの規則が働く）は別に数える（§6.1.1）
    loo_ok_all = [r for r in loo if r["x1"] is not None]
    loo_ok = [r for r in loo_ok_all if "homophone-left" not in r["group"]]
    loo_kata = [r for r in loo_ok if r["kata_rank_i"] is not None]  # カタカナの候補がある（同じ表記の分割の経路がない）
    loo_fail = [r for r in loo_kata if r["kata_rank_i"] != 1 or r["exp_rank_i"] != 1]  # 今は 1 位でない
    loo_dictpath = [r for r in loo_ok if r["kata_rank_i"] is None]
    loo_hl_fail = [r for r in loo_ok_all if "homophone-left" in r["group"] and r["kata_rank_i"] is not None and (r["kata_rank_i"] != 1 or r["exp_rank_i"] != 1)]
    # 今 1 位だが、カタカナの候補が lowered されていない（base_cost = ひらがなと同じ）のに 1 位 = 英字まじりの経路などの規則で上がっている
    loo_rescued = [r for r in loo_ok if r["kata_rank_i"] == 1 and r["exp_rank_i"] == 1 and r["kata_cost"] == r["kana_cost"]]

    out = []
    P = out.append

    # ---- 1. LOO の現状
    P("### LOO の現状（main の辞書の挙動、LM なし、`ikari --ranker full`、絞った辞書）")
    P("| 区分 | 件数 | 期待が 1 位 | カタカナ候補が期待（分割に同じ表記なし）| そのうち 1 位 | 期待が辞書の分割の経路で 1 位 | 期待が 50 位以内にない |")
    P("|---|---|---|---|---|---|---|")

    def loo_row(name, rs):
        n = len(rs)
        top1 = sum(1 for r in rs if r["exp_rank_i"] == 1)
        kat = [r for r in rs if r["kata_rank_i"] is not None and r["exp_origin"] == "katakana"]
        kat1 = sum(1 for r in kat if r["exp_rank_i"] == 1)
        dp = sum(1 for r in rs if r["exp_origin"] == "dictionary" and r["exp_rank_i"] == 1)
        miss = sum(1 for r in rs if r["exp_rank_i"] is None)
        return f"| {name} | {n} | {top1} ({top1 / n:.1%}) | {len(kat)} | {kat1} ({kat1 / max(1, len(kat)):.1%}) | {dp} | {miss} |"

    for name, f in [("全体", lambda r: True), ("unsplit", lambda r: "+unsplit" in r["group"]), ("split", lambda r: "+split" in r["group"]),
                    ("ng", lambda r: "+ng+" in r["group"]), ("shon", lambda r: "+shon+" in r["group"]), ("jon", lambda r: "+jon+" in r["group"]),
                    ("homophone-left", lambda r: "homophone-left" in r["group"])]:
        P(loo_row(name, [r for r in loo if f(r)]))
    P("")
    P(f"homophone-left を除いた主の群: 件数 {len(loo_ok)}、カタカナ候補が期待 {len(loo_kata)}、今 1 位でない {len(loo_fail)}、今 1 位だがカタカナの base_cost が下がっていない（英字まじりの経路などで上がっている） {len(loo_rescued)}。homophone-left の今 1 位でない {len(loo_hl_fail)}。")
    P("")

    # ---- 2. 最小コスト ÷ 文字数 の分布
    P("### 「カタカナ候補を除いた最小コスト ÷ 読みの文字数」の分位点（辞書の候補の最小 base_cost ÷ 文字数）")
    P("| 群 | 件数 | p5 | p25 | p50 | p75 | p95 |")
    P("|---|---|---|---|---|---|---|")
    P(qrow("守る: 全体", [r["x1"] for r in g_all]))
    P(qrow("守る: eval/sets", [r["x1"] for r in g_sets]))
    P(qrow("守る: Core の乱数抽出", [r["x1"] for r in g_core]))
    P(qrow("守る: 6 字以上のみ", [r["x1"] for r in g_len6]))
    P(qrow("守る: ぐ で終わる", [r["x1"] for r in g_all if is_gu(r)]))
    P(qrow("守る: 反復（擬音語型）", [r["x1"] for r in g_all if is_red(r)]))
    P(qrow("守る: kl-guard（擬音語・和語のセット）", [r["x1"] for r in g_sets if is_onomato_set(r)]))
    P(qrow("守る: 長音を含む", [r["x1"] for r in g_all if is_choon(r)]))
    P(qrow("狙う: LOO 全体（カタカナ候補あり）", [r["x1"] for r in loo_kata]))
    P(qrow("狙う: LOO 全体（今 1 位でない）", [r["x1"] for r in loo_fail]))
    P(qrow("狙う: LOO unsplit（カタカナ候補あり）", [r["x1"] for r in loo_kata if "+unsplit" in r["group"]]))
    P(qrow("狙う: LOO unsplit（今 1 位でない）", [r["x1"] for r in loo_fail if "+unsplit" in r["group"]]))
    P(qrow("狙う: LOO ng（今 1 位でない）", [r["x1"] for r in loo_fail if "+ng+" in r["group"]]))
    P(qrow("狙う: LOO shon（今 1 位でない）", [r["x1"] for r in loo_fail if "+shon+" in r["group"]]))
    P(qrow("狙う: LOO jon（今 1 位でない）", [r["x1"] for r in loo_fail if "+jon+" in r["group"]]))
    P(qrow("狙う: LOO homophone-left（今 1 位でない。別に数える）", [r["x1"] for r in loo_hl_fail]))
    P(qrow("狙う: LOO 期待が分割の経路（参考）", [r["x1"] for r in loo_dictpath]))
    P(qrow("狙う: katakana-like 弱い 13", [r["x1"] for r in target if r["group"] == "target-weak" and r["x1"] is not None]))
    P("")
    P("モーラで割った値（同じ群）")
    P("| 群 | 件数 | p5 | p25 | p50 | p75 | p95 |")
    P("|---|---|---|---|---|---|---|")
    P(qrow("守る: 全体", [r["x1m"] for r in g_all]))
    P(qrow("狙う: LOO 全体（今 1 位でない）", [r["x1m"] for r in loo_fail]))
    P("")
    P("AUC（狙う群の値が守る群より大きい確率。1 に近いほど分かれる）と、守る群の誤り率 1%・5% での拾う割合")
    P("| 狙う群 | 守る群 | 特徴 | AUC | FPR 1% の拾う割合 | FPR 5% の拾う割合 |")
    P("|---|---|---|---|---|---|")
    pos_sets = {"LOO 今 1 位でない": loo_fail, "LOO unsplit 今 1 位でない": [r for r in loo_fail if "+unsplit" in r["group"]], "LOO カタカナ候補あり": loo_kata}
    neg_sets = {"守る全体": g_all, "守る 6 字以上": g_len6, "Core 抽出": g_core}
    for pn, ps in pos_sets.items():
        for nn, ns in neg_sets.items():
            for fn, key in (("cost/文字", "x1"), ("cost/モーラ", "x1m")):
                pv, nv = [r[key] for r in ps], [r[key] for r in ns]
                t1, _ = tpr_at_fpr(pv, nv, 0.01)
                t5, _ = tpr_at_fpr(pv, nv, 0.05)
                P(f"| {pn} | {nn} | {fn} | {auc(pv, nv):.3f} | {t1:.1%} | {t5:.1%} |")
    P("")

    # ---- 3. 文字数ごとの辞書の最小コスト（守る群）
    P("### 文字数ごとの辞書の最小 base_cost（守る群 / 今 1 位でない LOO）")
    P("| 文字数 | 守る 件数 | 守る p5 | 守る p50 | 守る p95 | LOO 件数 | LOO p5 | LOO p50 | LOO p95 |")
    P("|---|---|---|---|---|---|---|---|---|")
    for n in list(range(2, 16)) + [16, 20, 25]:
        gs = [r["dict_min"] for r in g_all if r["n"] == n]
        ls = [r["dict_min"] for r in loo_fail if r["n"] == n]
        if not gs and not ls:
            continue
        gq, lq = quantiles(gs, (0.05, 0.5, 0.95)), quantiles(ls, (0.05, 0.5, 0.95))
        P(f"| {n} | {len(gs)} | " + " | ".join(fmt(v, 0) for v in gq) + f" | {len(ls)} | " + " | ".join(fmt(v, 0) for v in lq) + " |")
    P("")

    # ---- 4. K1 の格子の見込み: base + per_char × n < dict_min で『働く』割合
    P("### K1 の格子の見込み（`base + per_char × 文字数 < 辞書の最小 base_cost` を『カタカナが 1 位を取る』の代理にする。ranker の加点は含まない）")
    P("守る群で働く割合（FPR）が 0.5%・1%・2%・5% 以下になる範囲で、狙う群（LOO 今 1 位でない）を最も多く拾う (base, per_char)。")
    pos = loo_fail
    neg = g_all
    best = {}
    for pc in range(0, 8001, 100):
        yp = sorted(r["dict_min"] - pc * r["n"] for r in pos)
        yn = sorted(r["dict_min"] - pc * r["n"] for r in neg)
        # base の候補は守る群の y の値（その直下）
        for base in sorted(set(int(v) // 100 * 100 for v in yn[:: max(1, len(yn) // 400)])):
            fpr = (len(yn) - bisect_right(yn, base)) / len(yn)
            tpr = (len(yp) - bisect_right(yp, base)) / len(yp)
            for lim in (0.0, 0.005, 0.01, 0.02, 0.05):
                if fpr <= lim and (lim not in best or tpr > best[lim][0]):
                    best[lim] = (tpr, base, pc, fpr)
    P("| 守るの誤り率の上限 | 拾う割合 | base | per_char | 実際の誤り率 |")
    P("|---|---|---|---|---|")
    for lim in sorted(best):
        t, b, pc, f = best[lim]
        P(f"| {lim:.1%} | {t:.1%} | {b} | {pc} | {f:.2%} |")
    P("")
    P("per_char ごとの、守る群の誤り率 ≤ 1% で拾える最大の割合（base は最適）")
    P("| per_char | 拾う割合 | base | 誤り率 |")
    P("|---|---|---|---|")
    for pc in range(0, 8001, 500):
        yp = sorted(r["dict_min"] - pc * r["n"] for r in pos)
        yn = sorted(r["dict_min"] - pc * r["n"] for r in neg)
        bb = None
        for base in sorted(set(int(v) // 50 * 50 for v in yn[:: max(1, len(yn) // 800)])):
            fpr = (len(yn) - bisect_right(yn, base)) / len(yn)
            tpr = (len(yp) - bisect_right(yp, base)) / len(yp)
            if fpr <= 0.01 and (bb is None or tpr > bb[0]):
                bb = (tpr, base, fpr)
        if bb:
            P(f"| {pc} | {bb[0]:.1%} | {bb[1]} | {bb[2]:.2%} |")
    P("")

    # ---- 5. 本物の未知語
    P("### 本物の未知語（ふつうの辞書、main）。疑似（LOO 今 1 位でない）の分布の中での位置")
    P("| id | 読み | 文脈 | 期待の順位 | カタカナの順位 | 辞書の最小 | 辞書の最小 ÷ 文字 | 疑似でのその値の百分位 | 1 位の文節数 | 疑似の文節数の中央値 |")
    P("|---|---|---|---|---|---|---|---|---|---|")
    lx = sorted(r["x1"] for r in loo_fail)
    lseg = sorted(int(r["top1_nseg"]) for r in loo_fail if r["top1_nseg"])
    for r in target:
        if r["group"] != "target-real":
            continue
        pct = bisect_left(lx, r["x1"]) / len(lx)
        P(f"| {r['id']} | {r['reading']} | {r['context'] or '-'} | {r['exp_rank']} | {r['kata_rank']} | {r['dict_min']:.0f} | {r['x1']:.0f} | {pct:.0%} | {r['top1_nseg']} | {lseg[len(lseg) // 2]} |")
    P("")
    P("katakana-like 弱い 13 本（ふつうの辞書、main）")
    P("| id | 読み | 期待の順位 | 期待の origin | カタカナの順位 | 辞書の最小 ÷ 文字 |")
    P("|---|---|---|---|---|---|")
    for r in target:
        if r["group"] == "target-weak":
            P(f"| {r['id']} | {r['reading']} | {r['exp_rank'] or '-'} | {r['exp_origin'] or '-'} | {r['kata_rank'] or '-'} | {fmt(r['x1'], 0)} |")
    P("")

    # ---- 6. K6
    foreign, native = lex_pairs(a.sudachi)
    excl = set()
    excl_readings = set()
    with open(os.path.join(a.loo, "exclusions.tsv"), encoding="utf-8") as f:
        f.readline()
        for l in f:
            s, r = l.split("\t")[:2]
            excl.add((s, to_hira(r)))
            excl_readings.add(to_hira(r))
    # 評価の読み: eval/sets の全ケース、狙う群、守るの乱数抽出
    eval_readings = set()
    for path in glob.glob(os.path.join(a.repo, "eval/sets/*.tsv")):
        with open(path, encoding="utf-8") as f:
            lines = [l.rstrip("\n") for l in f if not l.startswith("#") and l.strip()]
        head = lines[0].split("\t")
        for l in lines[1:]:
            p = dict(zip(head, l.split("\t")))
            if p.get("reading"):
                eval_readings.add(p["reading"])
    for r in target:
        eval_readings.add(r["reading"])
    sample_readings = {r["reading"] for r in guard if not r["group"].startswith("guard-set:")}

    P("### K6（並びの表）")
    P(f"表を作る語: 外来語の側（表記がカタカナだけ）{len(foreign)} 組、和語の側（表記が漢字・かな）{len(native)} 組（Sudachi small・core・notcore。重複を数える前の (表記, 読み) の組）。除外表 {len(excl)} 組。eval/sets と狙う群の読み {len(eval_readings)} 件。守るの乱数抽出の読み {len(sample_readings)} 件。")
    P("")
    variants = [
        ("V0 除かない（漏れの大きさの確認）", set(), set()),
        ("V1 除外語（読みごと）+ 評価の読みを除く", excl_readings | eval_readings, set()),
        ("V1' 除外語は (表記, 読み) の組だけ + 評価の読み", eval_readings, excl),
        ("V2 V1 + 守るの乱数抽出の読みも除く（主）", excl_readings | eval_readings | sample_readings, set()),
    ]
    results = {}
    P("| 表の作り方 | 単位 | 外来語の側の読み | 和語の側の読み | 並びの種類 | AUC 守る=Core 抽出 | AUC 守る=全体 | 守る=全体 FPR1% 拾う割合 | FPR5% 拾う割合 |")
    P("|---|---|---|---|---|---|---|---|---|")
    main_tab = None
    for vname, drop_r, drop_p in variants:
        for uname, ufn in (("文字", units_char), ("モーラ", units_mora)):
            table, unseen, nf, nn = build_table(foreign, native, drop_r, drop_p, ufn)
            sc = lambda rs: [k6(table, unseen, r["reading"], ufn) for r in rs]
            pv = sc(loo_fail)
            ng_all, ng_core = sc(g_all), sc(g_core)
            t1, thr1 = tpr_at_fpr(pv, ng_all, 0.01)
            t5, _ = tpr_at_fpr(pv, ng_all, 0.05)
            P(f"| {vname} | {uname} | {nf} | {nn} | {len(table)} | {auc(pv, ng_core):.3f} | {auc(pv, ng_all):.3f} | {t1:.1%} | {t5:.1%} |")
            results[(vname, uname)] = (table, unseen)
            if vname.startswith("V2") and uname == "モーラ":
                main_tab = (table, unseen, ufn)
    P("")
    # 主の表（V2、モーラ）と（V2、文字）の詳細
    for uname, ufn in (("モーラ", units_mora), ("文字", units_char)):
        table, unseen = results[(variants[3][0], uname)]
        sc = lambda rs: [k6(table, unseen, r["reading"], ufn) for r in rs]
        P(f"#### K6 主（V2、{uname}）の分布（値は 1 並びあたりの対数尤度比。大きいほど外来語らしい）")
        P("| 群 | 件数 | p5 | p25 | p50 | p75 | p95 |")
        P("|---|---|---|---|---|---|---|")
        groups = [
            ("守る: 全体", g_all), ("守る: eval/sets", g_sets), ("守る: Core 抽出", g_core), ("守る: 6 字以上", g_len6),
            ("守る: ぐ で終わる", [r for r in g_all if is_gu(r)]), ("守る: 反復", [r for r in g_all if is_red(r)]),
            ("守る: kl-guard", [r for r in g_sets if is_onomato_set(r)]), ("守る: 長音を含む", [r for r in g_all if is_choon(r)]),
            ("狙う: LOO 今 1 位でない", loo_fail), ("狙う: LOO unsplit 今 1 位でない", [r for r in loo_fail if "+unsplit" in r["group"]]),
            ("狙う: LOO ng", [r for r in loo_fail if "+ng+" in r["group"]]), ("狙う: LOO shon", [r for r in loo_fail if "+shon+" in r["group"]]),
            ("狙う: LOO jon", [r for r in loo_fail if "+jon+" in r["group"]]),
            ("狙う: katakana-like 弱い", [r for r in target if r["group"] == "target-weak"]),
        ]
        for name, rs in groups:
            P(qrow(name, sc(rs), 2))
        pv = sc(loo_fail)
        P("")
        P("| 守る群の誤り率の上限 | 閾値 | 狙う(LOO 今 1 位でない) 拾う割合 | ぐ で終わる 誤り率 | 反復 誤り率 | kl-guard 誤り率 | 長音を含む 誤り率 | 6 字以上の守る 誤り率 |")
        P("|---|---|---|---|---|---|---|---|")
        gall = sc(g_all)
        for fpr in (0.005, 0.01, 0.02, 0.05):
            t, thr = tpr_at_fpr(pv, gall, fpr)
            def fp(rs):
                v = sc(rs)
                return f"{sum(1 for x in v if x > thr) / max(1, len(v)):.1%} ({len(v)})"
            P(f"| {fpr:.1%} | {thr:.2f} | {t:.1%} | {fp([r for r in g_all if is_gu(r)])} | {fp([r for r in g_all if is_red(r)])} | {fp([r for r in g_sets if is_onomato_set(r)])} | {fp([r for r in g_all if is_choon(r)])} | {fp(g_len6)} |")
        # 同じ誤り率での比較（cost/文字）
        P("")
        P("同じ守る群の誤り率での、拾う割合の比較（狙う = LOO 今 1 位でない）")
        P("| 誤り率 | cost/文字 | K6 | K6 のほうが高いか |")
        P("|---|---|---|---|")
        for fpr in (0.005, 0.01, 0.02, 0.05, 0.10):
            tc, _ = tpr_at_fpr([r["x1"] for r in loo_fail], [r["x1"] for r in g_all], fpr)
            tk, _ = tpr_at_fpr(pv, gall, fpr)
            P(f"| {fpr:.1%} | {tc:.1%} | {tk:.1%} | {'はい' if tk > tc else 'いいえ'} |")
        P("")
        P("守る群を 6 字以上に絞ったときの比較（LOO は全部 6 字以上なので、長さをそろえた比較）")
        P("| 誤り率 | cost/文字 | cost/モーラ | K6 |")
        P("|---|---|---|---|")
        g6 = sc(g_len6)
        for fpr in (0.005, 0.01, 0.02, 0.05, 0.10):
            tc, _ = tpr_at_fpr([r["x1"] for r in loo_fail], [r["x1"] for r in g_len6], fpr)
            tm, _ = tpr_at_fpr([r["x1m"] for r in loo_fail], [r["x1m"] for r in g_len6], fpr)
            tk, _ = tpr_at_fpr(pv, g6, fpr)
            P(f"| {fpr:.1%} | {tc:.1%} | {tm:.1%} | {tk:.1%} |")
        P(f"AUC（6 字以上の守る群に対して）: cost/文字 {auc([r['x1'] for r in loo_fail], [r['x1'] for r in g_len6]):.3f}、K6 {auc(pv, g6):.3f}")
        P("")
        # 本物
        P(f"本物の未知語の K6（V2、{uname}）と、疑似の分布での百分位")
        sv = sorted(pv)
        for r in target:
            if r["group"] == "target-real":
                v = k6(table, unseen, r["reading"], ufn)
                P(f"- {r['reading']}（{r['context'] or '文脈なし'}）: K6 = {v:.2f}（疑似の {bisect_left(sv, v) / len(sv):.0%} 点、守る全体の {bisect_left(sorted(gall), v) / len(gall):.0%} 点）")
        P("")

    # ---- K1 + K6 の見込み: カタカナのコスト = base + per_char × n − (K6 の効き)。辞書の最小がそれを超えたら『働く』
    table, unseen, ufn = main_tab
    for r in g_all + loo_fail:
        r["k6"] = k6(table, unseen, r["reading"], ufn)
    P("### K1 + K6 の見込み（V2、モーラ。w = 0 が K1 のみ）")
    P("加算型: カタカナのコスト = base + per_char × n − w × K6。乗算型（K1+K6b）: base + (per_char − w × K6) × n。いずれも辞書の最小 base_cost を下回ったら『働く』とみなす。")
    P("| 型 | 守るの誤り率の上限 | 拾う割合 | base | per_char | w | 実際の誤り率 |")
    P("|---|---|---|---|---|---|---|")
    for kind in ("加算", "乗算"):
        best2 = {}
        for pc in range(0, 4001, 250):
            for w in range(0, 4001, 250) if kind == "乗算" else range(0, 10001, 500):
                f = (lambda r: r["dict_min"] - pc * r["n"] + w * r["k6"]) if kind == "加算" else (lambda r: r["dict_min"] - pc * r["n"] + w * r["k6"] * r["n"])
                yp = sorted(f(r) for r in loo_fail)
                yn = sorted(f(r) for r in g_all)
                for base in sorted(set(int(v) // 200 * 200 for v in yn[:: max(1, len(yn) // 300)])):
                    fpr = (len(yn) - bisect_right(yn, base)) / len(yn)
                    tpr = (len(yp) - bisect_right(yp, base)) / len(yp)
                    for lim in (0.005, 0.01, 0.02, 0.05):
                        if fpr <= lim and (lim not in best2 or tpr > best2[lim][0]):
                            best2[lim] = (tpr, base, pc, w, fpr)
        for lim in sorted(best2):
            t, b, pc, w, fp_ = best2[lim]
            P(f"| {kind} | {lim:.1%} | {t:.1%} | {b} | {pc} | {w} | {fp_:.2%} |")
    P("")

    # ---- 表のサイズと判定の時間
    P("### 表のサイズと判定の時間（V2、モーラ）")
    keys_bytes = sum(len(k.encode()) for k in table)
    P(f"- 並びの種類: {len(table)}。文字列で持つと UTF-8 で約 {keys_bytes / 1024:.0f} KiB + 値 4 バイト × {len(table)} = 約 {(keys_bytes + 4 * len(table)) / 1024:.0f} KiB。64 bit のハッシュ + 値 2 バイトなら約 {10 * len(table) / 1024:.0f} KiB（衝突の確認は別）。")
    cnt_by_n = Counter(k.count("\u0001") + 1 for k in table)
    P(f"- 内訳: 2 つ組 {cnt_by_n[2]}、3 つ組 {cnt_by_n[3]}")
    t0 = time.perf_counter()
    reps = 3
    for _ in range(reps):
        for r in g_all:
            k6(table, unseen, r["reading"], ufn)
    dt = (time.perf_counter() - t0) / (reps * len(g_all))
    P(f"- 判定 1 回（Python の参考。Rust の値は別に測る）: {dt * 1e6:.1f} µs（読みの平均 {sum(r['n'] for r in g_all) / len(g_all):.1f} 文字）")

    if a.table_out:
        with open(a.table_out, "w", encoding="utf-8") as f:
            f.write(f"#unseen\t{unseen}\n")
            for k, v in sorted(table.items()):
                f.write(f"{k}\t{v}\n")
        with open(a.table_out + ".readings", "w", encoding="utf-8") as f:
            for r in g_all:
                f.write(r["reading"] + "\n")

    print("\n".join(out))


if __name__ == "__main__":
    sys.exit(main())
