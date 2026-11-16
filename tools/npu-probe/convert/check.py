"""IR を OpenVINO で流し、fixtures の llama.cpp CPU の NLL と比べる（ADR 0075 §4.6 の確かめ。スパイク）。

使い方:
  python -I check.py <fixtures.jsonl> <IR の .xml>... [--device CPU] [--precision f32|f16|default]

件ごとに、IR の形 [B, L] に候補を B 個ずつ詰めて採点し（README の「契約」の詰め方）、次を出す:
  1 位の一致（NLL 最小の候補が基準と同じ件の割合）、Kendall の τ-b の中央値、|NLL の差| の最大と中央値、
  件ごとの所要（候補の全部。ms）の p50 と p99。
"""

import argparse
import json
import statistics
import time

import numpy as np
import openvino as ov
from scipy.stats import kendalltau

INPUT_NAME = "input_ids"
OUTPUT_NAME = "logits"
PAD_TOKEN = 1
END_TOKEN = 3  # `</s>`（lm/src/llama.rs の find_end_token）


def log_softmax(row: np.ndarray) -> np.ndarray:
    m = row.max()
    return row - m - np.log(np.exp(row - m).sum())


def row_nll(logits, p, cand):
    """README の「契約」の 4。logits は 1 行分 [L, vocab]"""
    targets = cand + [END_TOKEN]
    return -sum(float(log_softmax(logits[p - 1 + j])[t]) for j, t in enumerate(targets))


def case_nll(request, batch, length, prompt, cands):
    """件の全部の候補の NLL。B 個ずつ 1 回に詰め、余りの行は [PAD] だけ（結果は読まない）。長すぎる候補は inf"""
    p = len(prompt)
    out = [float("inf")] * len(cands)
    ok = [i for i, c in enumerate(cands) if c and p + len(c) <= length]
    for start in range(0, len(ok), batch):
        chunk = ok[start : start + batch]
        ids = np.full((batch, length), PAD_TOKEN, dtype=np.int64)
        for row, i in enumerate(chunk):
            ids[row, : p + len(cands[i])] = prompt + cands[i]
        logits = request.infer({INPUT_NAME: ids})[OUTPUT_NAME]
        for row, i in enumerate(chunk):
            out[i] = row_nll(logits[row].astype(np.float32), p, cands[i])
    return out


def candidate_nll(request, length, prompt, cand):
    """B = 1 の 1 候補（ov_vs_torch などの手元の確かめ用）"""
    return case_nll(request, 1, length, prompt, [cand])[0]


def percentile(xs, q):
    s = sorted(xs)
    k = max(1, int(np.ceil(q / 100 * len(s))))
    return s[k - 1]


def run(path, fixtures, device, precision):
    core = ov.Core()
    model = core.read_model(path)
    (inp,), (out,) = model.inputs, model.outputs
    if INPUT_NAME not in inp.get_names() or OUTPUT_NAME not in out.get_names():
        raise SystemExit(f"IO names: {inp.get_names()} {out.get_names()}")
    shape = inp.get_partial_shape()
    batch, length = shape[0].get_length(), shape[1].get_length()
    cfg = {} if precision == "default" else {"INFERENCE_PRECISION_HINT": precision}
    request = core.compile_model(model, device, cfg).create_infer_request()

    top1, taus, diffs, times = 0, [], [], []
    for f in fixtures:
        t0 = time.perf_counter()
        got = case_nll(request, batch, length, f["prompt_tokens"], f["candidate_tokens"])
        times.append((time.perf_counter() - t0) * 1000)
        ref = [float("inf") if v is None else v for v in f["nll_llama_cpu"]]
        pairs = [(r, g) for r, g in zip(ref, got) if np.isfinite(r) and np.isfinite(g)]
        rs, gs = [r for r, _ in pairs], [g for _, g in pairs]
        top1 += int(np.argmin(rs) == np.argmin(gs))
        taus.append(kendalltau(rs, gs).statistic)
        diffs += [abs(r - g) for r, g in pairs]
    n = len(fixtures)
    print(
        f"{path.rsplit('/', 1)[-1]} {device}/{precision}: top1={top1}/{n} ({100 * top1 / n:.1f}%) "
        f"tau_median={statistics.median(taus):.4f} tau_min={min(taus):.4f} "
        f"|dNLL| max={max(diffs):.4f} median={statistics.median(diffs):.4f} "
        f"case_ms p50={percentile(times, 50):.1f} p99={percentile(times, 99):.1f} (candidates={len(diffs)})"
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("fixtures")
    ap.add_argument("models", nargs="+")
    ap.add_argument("--device", default="CPU")
    ap.add_argument("--precision", default="f32", choices=["f32", "f16", "default"])
    a = ap.parse_args()
    fixtures = [json.loads(line) for line in open(a.fixtures, encoding="utf-8") if line.strip()]
    if not fixtures:
        raise SystemExit("no fixtures")
    print(f"openvino {ov.get_version()} fixtures={len(fixtures)}")
    for m in a.models:
        run(m, fixtures, a.device, a.precision)


if __name__ == "__main__":
    main()
