#!/usr/bin/env bash
# ADR 0039 PoC: webwords の lex.csv を --notcore-lex に足した辞書を、scratch に作る（捨てるスクリプト）。
#   webwords_build_dict.sh <data のディレクトリ（raw と build の町名 lex が要る）> <出力ディレクトリ> [<webwords_lex.csv>]
# xtask/src/dict.rs の手順（town と chain の lex は data/build のものをそのまま使う）と同じ引数で ikari-dict-builder を呼ぶ。
# data/ には書かない。lex.csv を渡さなければ、基準の辞書（同じ手順で webwords なし）になる。
set -euo pipefail
DATA=$1
OUT=$2
WEB=${3:-}
BIN=${IKARI_DICT_BUILDER:-target/release/ikari-dict-builder}
SUD=$DATA/raw/sudachi-20260723
CHIVE=$DATA/raw/chive/chive-1.3-mc90/chive-1.3-mc90.txt
TOWN=$DATA/build/town_lex.csv
CHAIN=$DATA/build/chain_lex.csv
mkdir -p "$OUT"
EXTRA=()
VEXTRA=()
if [ -n "$WEB" ]; then
  EXTRA=(--notcore-lex "$WEB")
  VEXTRA=(--lex "$WEB")
fi
"$BIN" --policy dict-builder/policy/chive.tsv --chive-vocab "$CHIVE" \
  --lex "$SUD/small_lex.csv" --lex "$SUD/core_lex.csv" \
  --notcore-lex "$SUD/notcore_lex.csv" --notcore-lex "$TOWN" --chain-lex "$CHAIN" ${EXTRA[@]+"${EXTRA[@]}"} \
  --matrix "$SUD/matrix.def" --label "sudachi-full-poc" --out "$OUT" --policy-report "$OUT/policy-report.md" > "$OUT/build.log" 2>&1
"$BIN" vectors --chive "$CHIVE" --dict "$OUT" --out "$OUT/vectors.bin" \
  --lex "$SUD/small_lex.csv" --lex "$SUD/core_lex.csv" --lex "$SUD/notcore_lex.csv" --lex "$TOWN" --lex "$CHAIN" ${VEXTRA[@]+"${VEXTRA[@]}"} >> "$OUT/build.log" 2>&1
"$BIN" --verify "$OUT" >> "$OUT/build.log" 2>&1
echo built "$OUT"
