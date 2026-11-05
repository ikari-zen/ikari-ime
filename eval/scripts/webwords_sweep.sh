#!/usr/bin/env bash
# ADR 0039 PoC の掃引（捨てるスクリプト）。1 点 = lex を作る → 辞書を作る → snapshot（LM なし。--ajimee）→ 基準と比べる → ばーじょにんぐ の順位。
#   webwords_sweep.sh <scratch> <点>…      点 = "df,f5,cost[,tiers]"  例: 10,off,15000   10,on,15000   10,off,15000,"10:15000;50:12000"
# scratch には webwords-out/{f1,f2,f4,f5,f6b}.tsv、webwords-dict/base、ws-snap/base-nolm（基準の snapshot）がある前提。
# 結果は <scratch>/sweep/<点の名前>/ に置き、1 行の要約を <scratch>/sweep/summary.tsv に足す。
# 環境変数: MIN_LEN（既定 3）、LISTS（既定 5 ファイルすべて）、LM（gguf を渡すと LM ありの snapshot も取る）
set -euo pipefail
SCR=$1
shift
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$REPO"
DATA=${DATA:-$REPO/data}
SUD=$DATA/raw/sudachi-20260723
OUT=$SCR/webwords-out
LISTS=${LISTS:-"$OUT/f1.tsv $OUT/f2.tsv $OUT/f4.tsv $OUT/f5.tsv $OUT/f6b.tsv"}
MIN_LEN=${MIN_LEN:-3}
mkdir -p "$SCR/sweep" "$SCR/ws-snap"
for point in "$@"; do
  IFS=, read -r DF F5 COST TIERS <<< "$point"
  TIERS=${TIERS//;/,}
  NAME="df${DF}-f5${F5}-c${COST}${TIERS:+-t${TIERS//[:,]/_}}${IDS:+-id${IDS//,/_}}-len${MIN_LEN}"
  D=$SCR/sweep/$NAME
  mkdir -p "$D"
  F5ARG=()
  [ "$F5" = on ] && F5ARG=(--f5)
  # shellcheck disable=SC2086
  python3 -I eval/scripts/webwords_make_lex.py --sudachi "$SUD" --lists $LISTS --out "$D/webwords_lex.csv" \
    --min-df "$DF" --min-len "$MIN_LEN" --cost "$COST" ${TIERS:+--cost-tiers "$TIERS"} ${IDS:+--ids "$IDS"} ${F5ARG[@]+"${F5ARG[@]}"} --stats "$D/lex.stats" > /dev/null
  eval/scripts/webwords_build_dict.sh "$DATA" "$D/dict" "$D/webwords_lex.csv" > /dev/null
  rm -rf "$SCR/ws-snap/$NAME"
  IKARI_DICT_DIR="$D/dict" eval/scripts/snapshot.sh "$SCR/ws-snap/$NAME" --ajimee > "$D/snapshot.log" 2>&1 || true
  set +e
  python3 -I eval/scripts/webwords_compare.py "$SCR/ws-snap/base-nolm" "$SCR/ws-snap/$NAME" > "$D/compare.md"
  set -e
  REG=$(awk -F'|' '/^\| 合計/{gsub(/ /,"",$4);gsub(/ /,"",$5);gsub(/ /,"",$6);gsub(/ /,"",$7);print "top1 "$4"->"$5" worse="$6" better="$7}' "$D/compare.md")
  RANKS=$(python3 -I eval/scripts/webwords_rank.py --cli target/release/ikari --user-dir "$SCR/webwords-user-empty" --dicts "x=$D/dict" | awk -F'\t' 'NR>1 && $3=="ばーじょにんぐ"{printf "%s=%s ", $2, $4}')
  ROWS=$(wc -l < "$D/webwords_lex.csv" | tr -d ' ')
  SIZE=$(stat -f %z "$D/dict/system.dict")
  printf '%s\t%s\t%s\t%s\t%s\n' "$NAME" "$ROWS" "$SIZE" "$REG" "$RANKS" | tee -a "$SCR/sweep/summary.tsv"
done
