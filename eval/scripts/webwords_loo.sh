#!/usr/bin/env bash
# ADR 0039 §6.2 の狙う群(1)と雑音: leave-one-out の辞書（Sudachi の〜ング・〜ション・〜ジョンのカタカナ語 8698 件を伏せたもの。ADR 0037 S0 と同じ）に、
# 伏せた世界の Sudachi に F1 を当てた webwords を足して、伏せた語が 1 位になる数を測る（捨てるスクリプト）。
#   webwords_loo.sh <scratch> base                      … 基準（webwords なし）の LOO 辞書を作り、loo.tsv と guard を流す
#   webwords_loo.sh <scratch> <点>…                     … 点 = "df,f5,cost"
# scratch には webwords-loo-data（S0 の prepare の出力から作った仮の data）、webwords-loo/loo.tsv、webwords-cases/guard-eval.tsv がある前提。
# 結果: <scratch>/loo/<名前>/eval/{loo,guard}.tsv と、要約を <scratch>/loo/summary.tsv に足す。LM は使わない（LM は別に snapshot で）。
set -euo pipefail
SCR=$1
shift
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$REPO"
LDATA=$SCR/webwords-loo-data
OUT=$SCR/webwords-out
LISTS=${LISTS:-"$OUT/f1.tsv $OUT/f2.tsv $OUT/f4.tsv $OUT/f5.tsv $OUT/f6b.tsv"}
MIN_LEN=${MIN_LEN:-3}
mkdir -p "$SCR/loo"

run_sets() { # dict outdir
  mkdir -p "$2/eval"
  target/release/ikari-eval --dict "$1" --ranker full --repeat 1 --set "$SCR/webwords-loo/loo.tsv" --out-cases "$2/eval/loo.tsv" > "$2/eval/loo.md" 2>&1
  target/release/ikari-eval --dict "$1" --ranker full --repeat 1 --set "$SCR/webwords-cases/guard-eval.tsv" --out-cases "$2/eval/guard.tsv" > "$2/eval/guard.md" 2>&1
}

if [ "$1" = base ]; then
  eval/scripts/webwords_build_dict.sh "$LDATA" "$SCR/loo/base/dict" > /dev/null
  run_sets "$SCR/loo/base/dict" "$SCR/loo/base"
  exit 0
fi
for point in "$@"; do
  IFS=, read -r DF F5 COST TIERS <<< "$point"
  TIERS=${TIERS//;/,}
  NAME="df${DF}-f5${F5}-c${COST}${TIERS:+-t${TIERS//[:,]/_}}${IDS:+-id${IDS//,/_}}-len${MIN_LEN}"
  D=$SCR/loo/$NAME
  mkdir -p "$D"
  F5ARG=()
  [ "$F5" = on ] && F5ARG=(--f5)
  # shellcheck disable=SC2086
  python3 -I eval/scripts/webwords_make_lex.py --sudachi "$SCR/webwords-loo/csv" --lists $LISTS --out "$D/webwords_lex.csv" \
    --min-df "$DF" --min-len "$MIN_LEN" --cost "$COST" ${TIERS:+--cost-tiers "$TIERS"} ${IDS:+--ids "$IDS"} ${F5ARG[@]+"${F5ARG[@]}"} --stats "$D/lex.stats" > /dev/null
  eval/scripts/webwords_build_dict.sh "$LDATA" "$D/dict" "$D/webwords_lex.csv" > /dev/null
  run_sets "$D/dict" "$D"
  set +e
  python3 -I eval/scripts/webwords_compare.py --all "$SCR/loo/base" "$D" > "$D/compare.md"
  set -e
  LOO=$(awk -F'|' '$2 ~ /loo/{gsub(/ /,"",$4);gsub(/ /,"",$5);gsub(/ /,"",$6);gsub(/ /,"",$7);gsub(/ /,"",$8);print "loo top1 "$4"->"$5" worse="$6" better="$7}' "$D/compare.md")
  GUARD=$(awk -F'|' '$2 ~ /guard/{gsub(/ /,"",$4);gsub(/ /,"",$5);gsub(/ /,"",$6);gsub(/ /,"",$7);gsub(/ /,"",$8);print "guard top1 "$4"->"$5" worse="$6" better="$7" top1chg="$8}' "$D/compare.md")
  ROWS=$(wc -l < "$D/webwords_lex.csv" | tr -d ' ')
  printf '%s\t%s\t%s\t%s\n' "$NAME" "$ROWS" "$LOO" "$GUARD" | tee -a "$SCR/loo/summary.tsv"
done
