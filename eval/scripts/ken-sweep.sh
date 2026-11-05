#!/usr/bin/env bash
# 「件」の連接補正の掃引（ADR 0054 §5、§6.3）。辞書を更新したとき（Sudachi は 3〜4 か月に 1 回）のやり直しにも使う。
#
#   eval/scripts/ken-sweep.sh --work <scratch> --data <data> --variants "0:0 4000:0 8500:0 8500:9000" [--lm <gguf>] [--jobs 3]
#   eval/scripts/ken-sweep.sh --work <scratch> --data <data> --variants "..." --only build|run
#
# 変種 = `<matter_ken_after_no>:<matter_ken_after_rentai>`。`0:0` が基準（補正なし。grammar.bin は今と同じ）。
# 手順（辞書の更新のたび）:
#   1. ビルドの確認: 新しい辞書で `ikari-dict-builder` が通るか（connection.rs の確かめ。失敗なら理由が出る）。
#      （このスクリプトは T 以下の δ も作れるよう --allow-weak-connection を付ける。製品のビルドでは付けない）
#      できた辞書の `--policy-report` に、件の左 ID、右 ID の集合、閾値 T、競合の語が出る。
#   2. 変種に、基準 0:0、今の既定、T の前後（T-2000、T+250、T+500、T+1000、T+2000）を入れて、このスクリプトを流す。
#   3. `eval/scripts/ken_sweep_summary.py --work <scratch>` で表にする（基準に対する 1 位の悪化、〜の件の順位）。
#   4. 既定を変えるときは、表を PR に貼ってユーザーに確かめる（値は計測で決める）。結果は eval/reports/ に置く。
#
# 出力（<work> の中）: dict/<変種>/（辞書。`--policy-report` つき）、results/<変種>/<構成>/<セット名>.tsv（ケース別の順位）。
# 構成 = cost、topic、full（LM なし）、full-lm（--lm を渡したときだけ。締め切り 2000ms、プール既定）。
# 辞書は <work>/dict に作る。実機の辞書（data/build）とユーザーデータには触れない。
# 入力: <data>/raw（SudachiDict と chiVe。scripts/fetch-data.sh）、<data>/build/town_lex.csv（先に
# `cargo xtask dict --data <data>` で基準を 1 回作っておく。<data> は実機の data と別のディレクトリにして、raw だけ symlink する）。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="" DATA="" VARIANTS="" LM="" JOBS=3 ONLY="" EVAL_BIN="" LM_EVAL_BIN="" BUILDER=""
LM_DEADLINE_MS=2000
AJIMEE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --work) WORK="$2"; shift 2 ;;
    --data) DATA="$2"; shift 2 ;;
    --variants) VARIANTS="$2"; shift 2 ;;
    --lm) LM="$2"; shift 2 ;;
    --jobs) JOBS="$2"; shift 2 ;;
    --only) ONLY="$2"; shift 2 ;;
    --eval-bin) EVAL_BIN="$2"; shift 2 ;;
    --lm-eval-bin) LM_EVAL_BIN="$2"; shift 2 ;;
    --builder) BUILDER="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ -n "$WORK" && -n "$DATA" && -n "$VARIANTS" ]] || { echo "--work, --data, --variants are required" >&2; exit 2; }
case "$WORK" in "$ROOT"/data*|*/data/build*) echo "refusing to write under data/build" >&2; exit 2 ;; esac
EVAL_BIN="${EVAL_BIN:-$ROOT/target/release/ikari-eval}"
BUILDER="${BUILDER:-$ROOT/target/release/ikari-dict-builder}"
LM_EVAL_BIN="${LM_EVAL_BIN:-$EVAL_BIN}"
SUDACHI="$(ls -d "$DATA"/raw/sudachi-* | head -n 1)"
CHIVE="$(ls "$DATA"/raw/chive/*/*.txt | head -n 1)"
AJIMEE_JSON="$DATA/raw/ajimee/evaluation_items.json"
mkdir -p "$WORK/dict" "$WORK/results"

variant_name() { local no="${1%%:*}" rentai="${1##*:}"; echo "n${no}r${rentai}"; }

build_variant() {
  local v="$1" name no rentai out policy
  name="$(variant_name "$v")"; no="${v%%:*}"; rentai="${v##*:}"
  out="$WORK/dict/$name"
  if [[ -f "$out/vectors.bin" && -f "$out/policy-report.md" ]]; then return 0; fi
  rm -rf "$out"; mkdir -p "$out"
  policy="$out/policy.tsv"
  { grep -v '^matter_ken_' "$ROOT/dict-builder/policy/chive.tsv"
    printf 'matter_ken_after_no\t%s\nmatter_ken_after_rentai\t%s\n' "$no" "$rentai"; } > "$policy"
  local chain=()
  [[ -s "$DATA/build/chain_lex.csv" ]] && chain=(--chain-lex "$DATA/build/chain_lex.csv")
  "$BUILDER" --policy "$policy" --chive-vocab "$CHIVE" \
    --lex "$SUDACHI/small_lex.csv" --lex "$SUDACHI/core_lex.csv" \
    --notcore-lex "$SUDACHI/notcore_lex.csv" --notcore-lex "$DATA/build/town_lex.csv" "${chain[@]}" \
    --matrix "$SUDACHI/matrix.def" --label "sudachi-full-ken-$name" \
    --out "$out" --policy-report "$out/policy-report.md" --allow-weak-connection > "$out/build.log" 2>&1 \
    || { echo "build failed: $name (see $out/build.log)" >&2; tail -n 3 "$out/build.log" >&2; return 1; }
  local lexes=(--lex "$SUDACHI/small_lex.csv" --lex "$SUDACHI/core_lex.csv" --lex "$DATA/build/town_lex.csv")
  [[ -s "$DATA/build/chain_lex.csv" ]] && lexes+=(--lex "$DATA/build/chain_lex.csv")
  "$BUILDER" vectors --chive "$CHIVE" --dict "$out" --out "$out/vectors.bin" "${lexes[@]}" >> "$out/build.log" 2>&1
  "$BUILDER" --verify "$out" >> "$out/build.log" 2>&1
  for f in emoji.tsv postal.tsv; do
    [[ -f "$DATA/build/dict/$f" ]] && cp "$DATA/build/dict/$f" "$out/$f"
  done
  echo "built $name"
}

eval_variant() {
  local v="$1" name dict config set base
  name="$(variant_name "$v")"; dict="$WORK/dict/$name"
  local sets=("$ROOT"/eval/sets/*.tsv)
  local configs=(cost topic full)
  [[ -n "$LM" ]] && configs+=(full-lm)
  for config in "${configs[@]}"; do
    local dir="$WORK/results/$name/$config"
    [[ -f "$dir/DONE" ]] && continue
    rm -rf "$dir"; mkdir -p "$dir"
    local ranker="${config%-lm}" extra=() bin="$EVAL_BIN"
    if [[ "$config" == full-lm ]]; then
      extra=(--lm "$LM" --lm-budget-ms "$LM_DEADLINE_MS" --lm-deadline-ms "$LM_DEADLINE_MS" --repeat 1)
      bin="$LM_EVAL_BIN"
    else
      extra=(--repeat 1)
    fi
    for set in "${sets[@]}"; do
      base="$(basename "$set" .tsv)"
      [[ "$base" == segments ]] && continue
      (cd "$ROOT" && "$bin" --dict "$dict" --ranker "$ranker" "${extra[@]}" --set "$set" \
         --out-cases "$dir/$base.tsv" > "$dir/$base.md" 2> "$dir/$base.err") || true
    done
    if [[ -f "$AJIMEE_JSON" ]]; then
      (cd "$ROOT" && "$bin" --dict "$dict" --ranker "$ranker" "${extra[@]}" --set "$AJIMEE_JSON" --set-format ajimee \
         --out-cases "$dir/ajimee.tsv" > "$dir/ajimee.md" 2> "$dir/ajimee.err") || true
    fi
    # 文節の区切り（segments.tsv。レポートを比べる）
    (cd "$ROOT" && "$bin" segments --dict "$dict" --ranker "$ranker" --set "$ROOT/eval/sets/segments.tsv" --split all \
       > "$dir/segments.md" 2> "$dir/segments.err") || true
    touch "$dir/DONE"
    echo "evaluated $name $config"
  done
}

if [[ "$ONLY" != run ]]; then
  export -f build_variant variant_name
  export WORK DATA ROOT BUILDER SUDACHI CHIVE
  printf '%s\n' $VARIANTS | xargs -P "$JOBS" -I{} bash -c 'build_variant {}'
fi
if [[ "$ONLY" != build ]]; then
  for v in $VARIANTS; do eval_variant "$v"; done
fi
