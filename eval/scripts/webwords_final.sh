#!/usr/bin/env bash
# ADR 0055 §4.2 の守る群の残り（phr）と LM あり、新しいセット webwords を、絞った点で流す（レーン E）。
#   webwords_final.sh <scratch> --base       基準の辞書で流す
#   webwords_final.sh <scratch> <点>…         点 = "df,コスト"。辞書を作り直して流し、基準と比べる
# scratch に要るもの: webwords_sweep.sh --base の結果（pt/base/）、cases/phr.tsv と cases/phrk.tsv（普通名詞の連なり。ADR 0030 §4 の phr と同じ作り方。
#   ADR 0030 の phr は捨てるセットなので、同じ手順の eval/scripts/webwords_phr.py で作る）。
# 結果: <scratch>/pt/<名前>/final/eval/{phr,phrk,webwords}.tsv（LM なし）と <名前>/lm/eval/*.tsv（LM あり）。要約を <scratch>/final-summary.tsv に足す。
# 環境変数: LM（zenz の gguf。省略すると LM ありは流さない）、LM_SETS（LM ありで流すセット。既定は下の LM_SETS_DEFAULT。ajimee を含む）
set -euo pipefail
SCR=$1
shift
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$REPO"
# shellcheck source=eval/scripts/webwords_lib.sh
source eval/scripts/webwords_lib.sh
RAW=$SCR/raw
LM=${LM:-}
# 外来語が入ると動きうるセット（読みがカタカナ語になるもの）と AJIMEE。carry-* と homophones-ext は 1,269 件ずつで LM ありだと各 10 分以上かかるので外した（LM なしは全セット）
LM_SETS_DEFAULT="katakana-like new-words brand-katakana katakana-suffix english-first funhen kana-rescue webwords place-names abr-towns address-chain ajimee"
LM_SETS=${LM_SETS:-$LM_SETS_DEFAULT}

run_final() { # <辞書> <点のディレクトリ>
  local dict=$1 d=$2
  mkdir -p "$d/final/eval"
  for set in "$SCR/cases/phr.tsv" "$SCR/cases/phrk.tsv" "$REPO/eval/sets/webwords.tsv"; do
    name=$(basename "$set" .tsv)
    target/release/ikari-eval --dict "$dict" --ranker full --repeat 1 --set "$set" --out-cases "$d/final/eval/$name.tsv" > "$d/final/eval/$name.md" 2>&1
  done
  if [ -n "$LM" ]; then
    ONLY="$LM_SETS" eval/scripts/webwords_allsets.sh "$dict" "$d/lm" "$LM" > "$d/lm.log" 2>&1
  fi
}

if [ "${1:-}" = --base ]; then
  run_final "$SCR/pt/base/data/build/dict" "$SCR/pt/base"
  exit 0
fi

for point in "$@"; do
  IFS=, read -r DF COST <<< "$point"
  NAME="df${DF}-c${COST}"
  D=$SCR/pt/$NAME
  mkdir -p "$D"
  ww_build_dict "$RAW" "$D" --webwords-min-df "$DF" --webwords-cost "$COST"
  rm -rf "$D/final" "$D/lm"
  run_final "$D/data/build/dict" "$D"
  set +e
  python3 -I eval/scripts/webwords_compare.py --all "$SCR/pt/base/final" "$D/final" --show 100 > "$D/final-compare.md"
  python3 -I eval/scripts/webwords_compare.py --all "$SCR/pt/base/lm" "$D/lm" --show 100 > "$D/lm-compare.md" 2> /dev/null
  set -e
  printf '%s\t' "$NAME" | tee -a "$SCR/final-summary.tsv"
  awk -F'|' '/^\| (phr|phrk|webwords) /{for(i=3;i<=8;i++)gsub(/ /,"",$i);printf "%s: top1 %s->%s worse=%s better=%s chg=%s; ",$2,$4,$5,$6,$7,$8}' "$D/final-compare.md" | tee -a "$SCR/final-summary.tsv"
  awk -F'|' '/^\| 合計/{for(i=4;i<=8;i++)gsub(/ /,"",$i);printf "\tLM: top1 %s->%s worse=%s better=%s chg=%s\n",$4,$5,$6,$7,$8}' "$D/lm-compare.md" | tee -a "$SCR/final-summary.tsv"
  [ "${KEEP:-}" = 1 ] || rm -rf "$D/data/build/dict" "$D/data/build/dict.prev"
done
