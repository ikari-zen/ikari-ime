"""ADR 0037 §7 の 3: K1 の掃引で使う共通部品（捨てるコード。本体には入れない）。

ikari-eval / ikari を呼んで、--out-cases の結果を基準と比べる。"""
import csv
import glob
import json
import os
import subprocess

FNV_OFFSET, FNV_PRIME, MASK = 0xCBF29CE484222325, 0x100000001B3, (1 << 64) - 1
SPLIT_MODULUS = 5
NO_RANK = 10**6
SEED = "ikari-katakana-s0-v1"
KL_SET = "katakana-like"
TARGET_KINDS = ("カタカナ語(弱い)", "カタカナ語(分割負け)")


def fnv(s: str) -> int:
    h = FNV_OFFSET
    for b in s.encode():
        h = ((h ^ b) * FNV_PRIME) & MASK
    return h


def split_of(case_id: str) -> str:
    return "test" if fnv(case_id) % SPLIT_MODULUS == 0 else "dev"


def point_name(base, per_char, strong, weak):
    return f"b{base}_p{per_char}_s{strong}_w{weak}"


def write_kv(path, base, per_char, strong, weak):
    with open(path, "w", encoding="utf-8") as f:
        f.write(f"katakana_like.base={base}\n")
        f.write(f"katakana_like.per_char={per_char}\n")
        f.write(f"katakana_like.strong_bonus={strong}\n")
        f.write(f"katakana_like.weak_bonus={weak}\n")


def rank_of(v):
    try:
        return int(v)
    except ValueError:
        return NO_RANK


def read_cases(path):
    """--out-cases の TSV -> {キー: dict}。キーは (id, context, reading)"""
    out = {}
    with open(path, encoding="utf-8", newline="") as f:
        for row in csv.DictReader(f, delimiter="\t", quoting=csv.QUOTE_NONE):
            row["rank_i"] = rank_of(row["rank"])
            out[(row["id"], row["context"], row["reading"])] = row
    return out


def run_eval(repo, dict_dir, set_path, out_path, split="dev", set_format="ikari", kv=None, lm=None, extra=None):
    cmd = [os.path.join(repo, "target/release/ikari-eval"), "--dict", dict_dir, "--ranker", "full",
           "--set", set_path, "--split", split, "--repeat", "1", "--out-cases", out_path]
    if set_format != "ikari":
        cmd += ["--set-format", set_format]
    if kv:
        cmd += ["--ranker-config", kv]
    if lm:
        cmd += ["--lm", lm, "--lm-budget-ms", "2000", "--lm-deadline-ms", "2000"]
    if extra:
        cmd += extra
    p = subprocess.run(cmd, capture_output=True, text=True, cwd=repo)
    return p.returncode, p.stdout, p.stderr


def diff(base, cur):
    """基準で 1 位だったのに 1 位でなくなった (回帰)、基準で 1 位でなかったのに 1 位になった (改善)"""
    reg, imp = [], []
    for k, b in base.items():
        c = cur.get(k)
        if c is None:
            continue
        if b["rank_i"] == 1 and c["rank_i"] != 1:
            reg.append((k[0], b["reading"], b["expected"], b["top1"], c["top1"], c["rank_i"]))
        elif b["rank_i"] != 1 and c["rank_i"] == 1:
            imp.append((k[0], b["reading"], b["expected"], b["top1"], c["top1"], b["rank_i"]))
    return reg, imp


def set_list(repo):
    return sorted(glob.glob(os.path.join(repo, "eval/sets/*.tsv")))


def dump(path, obj):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False)
