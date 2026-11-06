#!/usr/bin/env bash
# ADR 0055 §4.2 の段 1 の掃引（レーン E）。1 点 = 辞書を作る → 全セット + AJIMEE を流す → 基準と比べる → ばーじょにんぐ などの順位。
#   webwords_sweep.sh <scratch> <点>…        点 = "df,コスト"（コストは --webwords-cost の基準。df<20 はこれ +2000、20 以上は -1000、100 以上は -4000）
#                                          または "default"（値を渡さず dict-builder の既定の値で作る。名前は default。cargo xtask webwords measure が使う）
#
# scratch に要るもの:
#   webwords-list.tsv        schema=2 の一覧（dict-builder webwords collect → classify の出力）
#   pt/base/               基準の辞書（webwords_sweep.sh --base で作る）と、その eval/
#   user-empty/            空のディレクトリ（ikari の --user-dir。学習を混ぜない）
# 結果: <scratch>/pt/<名前>/{build.log,compare.md,eval/,data/build/webwords_lex.csv,data/build/webwords-report.md}
#       1 行の要約を <scratch>/summary.tsv に足す。
#   webwords_sweep.sh <scratch> --base      基準（--no-webwords）の辞書と eval を作る
# 環境変数: DATA_RAW（基準の raw の元。既定は <リポジトリ>/data/raw）、LM（zenz の gguf。指定すると LM あり（時間切れ上限 2000ms）も流す。基準は pt/base/eval-lm。ONLY="a b" でセットを絞る）、
#           KEEP=1（辞書を消さない。既定は評価のあと system.dict などを消す。1 点 1.2GB）
set -euo pipefail
SCR=$1
shift
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$REPO"
# shellcheck source=eval/scripts/webwords_lib.sh
source eval/scripts/webwords_lib.sh
RAW=$SCR/raw
LM=${LM:-}
mkdir -p "$SCR/user-empty" "$SCR/pt"

run_eval() { # <辞書> <出力ディレクトリ>（結果は <出力>/eval/）。LM が指定のときは続けて <出力>-lm/eval/ にも
  eval/scripts/webwords_allsets.sh "$1" "$2" > "$2.log" 2>&1
  if [ -n "$LM" ]; then
    eval/scripts/webwords_allsets.sh "$1" "$2-lm" "$LM" > "$2-lm.log" 2>&1
  fi
}

if [ "${1:-}" = --base ]; then
  ww_prepare_raw "$RAW" "${DATA_RAW:-$REPO/data/raw}" "$SCR/webwords-list.tsv"
  mkdir -p "$SCR/pt/base"
  ww_build_dict "$RAW" "$SCR/pt/base" --no-webwords
  run_eval "$SCR/pt/base/data/build/dict" "$SCR/pt/base"
  exit 0
fi

compare_line() { # <基準の eval> <候補の eval> → "top1 b->c worse=N better=M"
  python3 -I eval/scripts/webwords_compare.py "$1" "$2" 2> /dev/null | awk -F'|' '/^\| 合計/{for(i=4;i<=7;i++)gsub(/ /,"",$i);print "top1 "$4"->"$5" worse="$6" better="$7}'
}

for point in "$@"; do
  if [ "$point" = default ]; then
    NAME=default
    POINT_ARGS=()
  else
    IFS=, read -r DF COST <<< "$point"
    NAME="df${DF}-c${COST}"
    POINT_ARGS=(--webwords-min-df "$DF" --webwords-cost "$COST")
  fi
  D=$SCR/pt/$NAME
  mkdir -p "$D"
  ww_build_dict "$RAW" "$D" ${POINT_ARGS[@]+"${POINT_ARGS[@]}"}
  DICT=$D/data/build/dict
  # eval の出力は <D>/eval/…。compare は <出力>/eval/*.tsv を読むので、基準も同じ形にしてある
  rm -rf "$D/eval" "$D/eval-lm"
  run_eval "$DICT" "$D"
  set +e
  python3 -I eval/scripts/webwords_compare.py "$SCR/pt/base" "$D" --show 100 > "$D/compare.md"
  [ -n "$LM" ] && python3 -I eval/scripts/webwords_compare.py "$SCR/pt/base-lm" "$D-lm" --show 100 > "$D/compare-lm.md"
  set -e
  REG=$(compare_line "$SCR/pt/base" "$D")
  REGLM=""
  [ -n "$LM" ] && REGLM=$(compare_line "$SCR/pt/base-lm" "$D-lm")
  RANKS=$(python3 -I eval/scripts/webwords_rank.py --cli target/release/ikari --user-dir "$SCR/user-empty" --dicts "x=$DICT" | awk -F'\t' 'NR>1 && ($3=="ばーじょにんぐ" || $3=="しゃーでぃんぐ"){printf "%s:%s=%s ", $3, $2, $4}')
  ROWS=$(wc -l < "$D/data/build/webwords_lex.csv" | tr -d ' ')
  SIZE=$(stat -f %z "$DICT/system.dict")
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$NAME" "$ROWS" "$SIZE" "$REG" "$REGLM" "$RANKS" | tee -a "$SCR/summary.tsv"
  [ "${KEEP:-}" = 1 ] || rm -rf "$D/data/build/dict" "$D/data/build/dict.prev"
done
