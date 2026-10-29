#!/usr/bin/env bash
# ADR 0038 §6.2 S1: 3 つの腕（X 今 / Y 案 1 / Z 案 4 の上限）を流す（捨てるスクリプト）。
# 前提: make_stale.py で target/prefetch-s1 を作ってある。辞書とモデルは実機のものを読むだけ。
# 使い方: run.sh   （環境変数 IKARI_DICT_DIR、IKARI_ZENZ_MODEL で上書き可）
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../../.."

readonly OUT="target/prefetch-s1"
readonly SUPPORT="$HOME/Library/Application Support/ikari-ime"
readonly DICT="${IKARI_DICT_DIR:-$SUPPORT/dict}"
readonly GGUF="${IKARI_ZENZ_MODEL:-$SUPPORT/models/zenz-v3.2-small-Q5_K_M.gguf}"
readonly LM_BIN="target/lm/release/ikari-eval"
readonly NOLM_BIN="target/release/ikari-eval"
readonly LM_DEADLINE_MS=2000

mkdir -p "$OUT/x" "$OUT/z" "$OUT/yA" "$OUT/yB"
uptime >"$OUT/uptime.txt"

# 元のセット（A または B のどちらかで使うもの）
sets=$(cat "$OUT"/units-A.tsv "$OUT"/units-B.tsv | cut -f1 | grep -v '^set$' | sort -u)

for s in $sets; do
  [ -f "$OUT/x/$s.tsv" ] || "$NOLM_BIN" --dict "$DICT" --ranker full --repeat 1 --set "eval/sets/$s.tsv" --out-cases "$OUT/x/$s.tsv" >"$OUT/x/$s.md"
  [ -f "$OUT/z/$s.tsv" ] || "$LM_BIN" --dict "$DICT" --ranker full --repeat 1 --lm "$GGUF" --lm-deadline-ms $LM_DEADLINE_MS --set "eval/sets/$s.tsv" --out-cases "$OUT/z/$s.tsv" >"$OUT/z/$s.md"
  for k in A B; do
    [ -f "$OUT/stale-$k/$s.tsv" ] || continue
    [ -f "$OUT/y$k/$s.tsv" ] || "$LM_BIN" --dict "$DICT" --ranker full --repeat 1 --lm "$GGUF" --lm-deadline-ms $LM_DEADLINE_MS --set "$OUT/stale-$k/$s.tsv" --out-cases "$OUT/y$k/$s.tsv" >"$OUT/y$k/$s.md"
  done
  echo "done $s"
done
