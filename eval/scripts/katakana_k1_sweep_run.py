#!/usr/bin/env python3
"""ADR 0037 §7 の 3: K1 の掃引（捨てるスクリプト。本体には入れない）。

サブコマンド
  prep    基準（K1 無効、今の main の既定）を取り、狙う群の主 (main900) の id を決める
  sweep   格子の点ごとに、全セット (dev)・AJIMEE・疑似の守る側・狙う群を基準と比べる (LM なし)
  lm      指定した点だけ LM ありで測る

使い方: katakana_k1_sweep_run.py <sub> --repo <repo> --scratch <作業 dir> --dict <ふつうの辞書> --loo-dict <絞った辞書> [--ajimee <json>] ...
"""
import argparse
import csv
import json
import os
import shutil
import sys
import time
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import katakana_k1_sweep_lib as L  # noqa: E402

PER_CHARS = [0] + list(range(500, 2501, 250))
BASES = list(range(6000, 24001, 2000))
STRONGS = [5000, 0]
WEAKS = [0, 2000]
BASELINE = (0, 0, 5000, 0)  # K1 無効、strong 5000 (main の既定)、weak 0
AJIMEE_NAME = "ajimee"
LOO_NAME = "loo"
LM_TARGET_LIMIT = 1000


def grid():
    pts = [BASELINE]
    for s in STRONGS:
        for w in WEAKS:
            for p in PER_CHARS:
                for b in BASES:
                    pts.append((b, p, s, w))
    # 0/0 は strong/weak の違いだけの点も残す (K1 無効のまま strong/weak を動かす)
    for s in STRONGS:
        for w in WEAKS:
            if (0, 0, s, w) not in pts:
                pts.append((0, 0, s, w))
    return pts


def main_target_ids(loo_out_tsv):
    """S0 と同じ定義: homophone-left を除き、カタカナの候補があり、(カタカナの順位 != 1 または 期待の順位 != 1)"""
    ids = []
    with open(loo_out_tsv, encoding="utf-8", newline="") as f:
        for r in csv.DictReader(f, delimiter="\t", quoting=csv.QUOTE_NONE):
            if "homophone-left" in r["group"] or not r["dict_min"]:
                continue
            if not r["kata_rank"]:
                continue
            if r["kata_rank"] != "1" or r["exp_rank"] != "1":
                ids.append(r["id"])
    return ids


def eval_all(a, kv, outdir, lm=None):
    """全評価対象を流して {名前: cases} を返す。戻りの 2 つ目は異常の一覧"""
    os.makedirs(outdir, exist_ok=True)
    res, errs = {}, []
    jobs = [(os.path.basename(p)[:-4], p, "ikari", a.dict, "dev") for p in L.set_list(a.repo)]
    if a.ajimee:
        jobs.append((AJIMEE_NAME, a.ajimee, "ajimee", a.dict, "all"))
    jobs.append((LOO_NAME, a.loo_tsv, "ikari", a.loo_dict, "dev"))
    for name, path, fmt, dd, split in jobs:
        out = os.path.join(outdir, name + ".tsv")
        rc, so, se = L.run_eval(a.repo, dd, path, out, split=split, set_format=fmt, kv=kv, lm=lm)
        if rc != 0 or not os.path.exists(out):
            errs.append((name, rc, se[-300:]))
            continue
        res[name] = L.read_cases(out)
    return res, errs


def summarize(base, cur, main_ids, errs):
    rec = {"errs": [(n, rc) for n, rc, _ in errs], "sets": {}, "reg_examples": []}
    total_reg = 0
    for name, b in base.items():
        if name == LOO_NAME or name not in cur:
            continue
        reg, imp = L.diff(b, cur[name])
        rec["sets"][name] = {"reg": len(reg), "imp": len(imp)}
        total_reg += len(reg)
        for r in reg[:3]:
            rec["reg_examples"].append((name,) + r)
    rec["reg_sets_total"] = total_reg
    # katakana-like の狙う群 (dev)
    kl_b, kl_c = base.get(L.KL_SET), cur.get(L.KL_SET)
    if kl_b and kl_c:
        tgt = [k for k, v in kl_b.items() if v["kind"] in L.TARGET_KINDS]
        rec["kl_target_n"] = len(tgt)
        rec["kl_target_top1_base"] = sum(1 for k in tgt if kl_b[k]["rank_i"] == 1)
        rec["kl_target_top1"] = sum(1 for k in tgt if kl_c[k]["rank_i"] == 1)
        rec["real"] = {k[0]: kl_c[k]["rank_i"] for k in kl_c if k[0] in ("kl-split-01", "kl-split-02", "kl-split-03", "kl-split-04")}
    # 疑似
    lb, lc = base.get(LOO_NAME), cur.get(LOO_NAME)
    if lb and lc:
        reg, imp = L.diff(lb, lc)
        rec["loo_guard_reg"] = len(reg)
        rec["loo_reg_examples"] = reg[:5]
        rec["loo_imp_all"] = len(imp)
        ms = set(main_ids)
        main_dev = [k for k in lb if k[0] in ms]
        rec["main_dev_n"] = len(main_dev)
        rec["main_dev_top1_base"] = sum(1 for k in main_dev if lb[k]["rank_i"] == 1)
        rec["main_dev_top1"] = sum(1 for k in main_dev if lc[k]["rank_i"] == 1)
        # 内訳 (tags)
        for tag in ("unsplit", "ng", "shon", "jon"):
            ks = [k for k in main_dev if tag in lb[k]["tags"].split(",")]
            rec["main_" + tag + "_n"] = len(ks)
            rec["main_" + tag + "_top1"] = sum(1 for k in ks if lc[k]["rank_i"] == 1)
        hl = [k for k in lb if "homophone-left" in lb[k]["tags"].split(",") and lb[k]["rank_i"] != 1]
        rec["hl_imp"] = sum(1 for k in hl if lc[k]["rank_i"] == 1)
    return rec


def load_base(a):
    base = {}
    d = os.path.join(a.scratch, "base")
    for fn in os.listdir(d):
        if fn.endswith(".tsv"):
            base[fn[:-4]] = L.read_cases(os.path.join(d, fn))
    return base


def cmd_prep(a):
    base_dir = os.path.join(a.scratch, "base")
    kv = os.path.join(a.scratch, "base.kv")
    L.write_kv(kv, *BASELINE)
    t = time.time()
    # kv なしの結果 (今の main の既定) と同じか確かめるため、kv なしで取る
    res, errs = eval_all(a, None, base_dir)
    print("基準を取った", round(time.time() - t, 1), "秒、異常:", errs, file=sys.stderr)
    ids = main_target_ids(a.loo_out)
    with open(os.path.join(a.scratch, "main900.txt"), "w", encoding="utf-8") as f:
        f.write("\n".join(ids) + "\n")
    lb = res[LOO_NAME]
    print("main (全 split):", len(ids), " dev:", sum(1 for k in lb if k[0] in set(ids)), file=sys.stderr)
    # K1 無効の kv (0/0、strong 5000、weak 0) が kv なしと同じことの確認
    res2, _ = eval_all(a, kv, os.path.join(a.scratch, "base-kv-check"))
    same = all(
        all(res[n][k]["rank_i"] == res2[n][k]["rank_i"] and res[n][k]["top1"] == res2[n][k]["top1"] for k in res[n])
        for n in res if n in res2)
    print("kv なしと K1 無効の kv が同じ:", same, file=sys.stderr)


def cmd_sweep(a):
    base = load_base(a)
    main_ids = [l.strip() for l in open(os.path.join(a.scratch, "main900.txt"), encoding="utf-8") if l.strip()]
    pts = grid()
    res_path = os.path.join(a.scratch, "sweep.jsonl")
    done = set()
    if os.path.exists(res_path):
        for l in open(res_path, encoding="utf-8"):
            done.add(json.loads(l)["name"])
    todo = [p for p in pts if L.point_name(*p) not in done]
    print("点:", len(pts), "未了:", len(todo), file=sys.stderr, flush=True)

    def one(p):
        name = L.point_name(*p)
        kv = os.path.join(a.scratch, "kv", name + ".kv")
        os.makedirs(os.path.dirname(kv), exist_ok=True)
        L.write_kv(kv, *p)
        outdir = os.path.join(a.scratch, "out", name)
        t = time.time()
        cur, errs = eval_all(a, kv, outdir)
        rec = summarize(base, cur, main_ids, errs)
        rec.update(name=name, base=p[0], per_char=p[1], strong=p[2], weak=p[3], sec=round(time.time() - t, 1))
        shutil.rmtree(outdir, ignore_errors=True)
        return rec

    with ThreadPoolExecutor(a.jobs) as ex, open(res_path, "a", encoding="utf-8") as fo:
        for i, rec in enumerate(ex.map(one, todo)):
            fo.write(json.dumps(rec, ensure_ascii=False) + "\n")
            fo.flush()
            print(i + 1, len(todo), rec["name"], "sets_reg", rec["reg_sets_total"], "loo_reg", rec.get("loo_guard_reg"),
                  "main_top1", rec.get("main_dev_top1"), rec["sec"], file=sys.stderr, flush=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("sub", choices=["prep", "sweep"])
    ap.add_argument("--repo", required=True)
    ap.add_argument("--scratch", required=True)
    ap.add_argument("--dict", required=True)
    ap.add_argument("--loo-dict", required=True)
    ap.add_argument("--loo-tsv", required=True, dest="loo_tsv")
    ap.add_argument("--loo-out", required=True, dest="loo_out", help="S0 の collect が baseline で出した loo.out.tsv")
    ap.add_argument("--ajimee")
    ap.add_argument("--jobs", type=int, default=8)
    a = ap.parse_args()
    {"prep": cmd_prep, "sweep": cmd_sweep}[a.sub](a)


if __name__ == "__main__":
    main()
