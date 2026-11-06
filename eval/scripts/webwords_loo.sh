#!/usr/bin/env bash
# ADR 0055 §4.2 の狙う群 (1) と守る群（S0 の 6,999 読み）: leave-one-out（ADR 0037 S0 の 8,698 件を Sudachi から伏せた世界）で、
# 伏せた世界の Sudachi に F1 を当てた一覧（classify の --lex を伏せた Sudachi にして作る）を足し、伏せた語が 1 位に戻る数と、守る群の悪化を数える。
#   webwords_loo.sh <scratch> --base       基準（--no-webwords）の LOO 辞書を作り、loo と guard を流す
#   webwords_loo.sh <scratch> <点>…         点 = "df,コスト"（webwords_sweep.sh と同じ）
# scratch に要るもの:
#   loo-list.tsv                     伏せた Sudachi（loo-raw/sudachi-20260723）と ABR で classify した schema=2 の一覧
#   loo-src/sudachi-20260723/        伏せた Sudachi（small/core/notcore の lex.csv と matrix.def）。S0 の prepare の出力から作る
#   cases/loo.tsv、cases/guard-eval.tsv    伏せた 8,698 件と、守る群 6,999 読み（eval/scripts/katakana_s0_prepare.py と同じ作り方）
# 結果: <scratch>/loo/<名前>/{eval/loo.tsv,eval/guard.tsv,compare.md}。要約を <scratch>/loo/summary.tsv に足す。LM は使わない。
set -euo pipefail
SCR=$1
shift
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$REPO"
# shellcheck source=eval/scripts/webwords_lib.sh
source eval/scripts/webwords_lib.sh
RAW=$SCR/loo-raw
mkdir -p "$SCR/loo"

prepare_raw() {
  rm -rf "$RAW"
  ww_prepare_raw "$RAW" "$REPO/data/raw" "$SCR/loo-list.tsv"
  rm "$RAW/sudachi-20260723"
  mkdir "$RAW/sudachi-20260723"
  for f in small_lex.csv core_lex.csv notcore_lex.csv; do
    ln -s "$SCR/loo-src/sudachi-20260723/$f" "$RAW/sudachi-20260723/$f"
  done
  ln -s "$REPO/data/raw/sudachi-20260723/matrix.def" "$RAW/sudachi-20260723/matrix.def"
}

run_sets() { # <辞書> <出力ディレクトリ>
  mkdir -p "$2/eval"
  target/release/ikari-eval --dict "$1" --ranker full --repeat 1 --set "$SCR/cases/loo.tsv" --out-cases "$2/eval/loo.tsv" > "$2/eval/loo.md" 2>&1
  target/release/ikari-eval --dict "$1" --ranker full --repeat 1 --set "$SCR/cases/guard-eval.tsv" --out-cases "$2/eval/guard.tsv" > "$2/eval/guard.md" 2>&1
}

if [ "${1:-}" = --base ]; then
  prepare_raw
  mkdir -p "$SCR/loo/base"
  ww_build_dict "$RAW" "$SCR/loo/base" --no-webwords
  run_sets "$SCR/loo/base/data/build/dict" "$SCR/loo/base"
  [ "${KEEP:-}" = 1 ] || rm -rf "$SCR/loo/base/data/build/dict" "$SCR/loo/base/data/build/dict.prev"
  exit 0
fi

for point in "$@"; do
  IFS=, read -r DF COST <<< "$point"
  NAME="df${DF}-c${COST}"
  D=$SCR/loo/$NAME
  mkdir -p "$D"
  ww_build_dict "$RAW" "$D" --webwords-min-df "$DF" --webwords-cost "$COST"
  run_sets "$D/data/build/dict" "$D"
  set +e
  python3 -I eval/scripts/webwords_compare.py --all "$SCR/loo/base" "$D" > "$D/compare.md"
  set -e
  LOO=$(awk -F'|' '$2 ~ /loo/{for(i=4;i<=8;i++)gsub(/ /,"",$i);print "loo top1 "$4"->"$5" worse="$6" better="$7}' "$D/compare.md")
  GUARD=$(awk -F'|' '$2 ~ /guard/{for(i=4;i<=8;i++)gsub(/ /,"",$i);print "guard top1 "$4"->"$5" worse="$6" better="$7" top1chg="$8}' "$D/compare.md")
  ROWS=$(wc -l < "$D/data/build/webwords_lex.csv" | tr -d ' ')
  printf '%s\t%s\t%s\t%s\n' "$NAME" "$ROWS" "$LOO" "$GUARD" | tee -a "$SCR/loo/summary.tsv"
  [ "${KEEP:-}" = 1 ] || rm -rf "$D/data/build/dict" "$D/data/build/dict.prev"
done
