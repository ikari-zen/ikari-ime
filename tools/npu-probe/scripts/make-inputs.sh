#!/usr/bin/env bash
# 測定ツールの入力（tools/npu-probe/inputs/homophones-40.jsonl）を Mac で作る（ADR 0075 §4.2）。
# 評価（ikari-eval --ranker full --lm）を 1 件ずつ流し、LM に届いた頼み（プロンプトと候補）を
# npu-probe-record-host が記録する。学習は一時ディレクトリ（評価の既定。利用者の学習を混ぜない）。
#
#   tools/npu-probe/scripts/make-inputs.sh <zenz の GGUF> [辞書のディレクトリ]
#
# 辞書の既定は $IKARI_DICT_DIR、なければ ./data/build/dict。llama.cpp（Metal）をビルドするので cmake が要る。
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"
model="${1:?usage: make-inputs.sh <zenz.gguf> [dict-dir]}"
dict="${2:-${IKARI_DICT_DIR:-$root/data/build/dict}}"
set_file="eval/sets/homophones-ext.tsv"
out="tools/npu-probe/inputs/homophones-40.jsonl"
target="${CARGO_TARGET_DIR:-$root/target}/release"

cargo build --release -p ikari-npu-probe --features metal --bin npu-probe-record-host --bin ikari-npu-probe
cargo build --release -p ikari-eval --features lm-metal

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
records="$work/records.jsonl"
: > "$records"
# 見出し（# の行と id の行）
header="$(grep -E $'^(#|id\t)' "$set_file")"
while IFS= read -r id; do
  one="$work/one.tsv"
  { printf '%s\n' "$header"; grep -E $"^${id}\t" "$set_file"; } > "$one"
  IKARI_NPU_PROBE_RECORD="$records" IKARI_NPU_PROBE_ID="$id" \
    "$target/ikari-eval" --set "$one" --ranker full --dict "$dict" \
    --lm "$model" --lm-host "$target/npu-probe-record-host" --lm-budget-ms 5000 > /dev/null
  echo "記録した: $id" >&2
done < <("$target/ikari-npu-probe" make-inputs --print-ids)

"$target/ikari-npu-probe" make-inputs --records "$records" --set "$set_file" --out "$out"
echo "書いた: $out（$(wc -l < "$out") 件）" >&2
