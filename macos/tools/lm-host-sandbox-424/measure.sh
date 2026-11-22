#!/bin/bash
# スパイク #424（マージしない）。1 回の評価を、指定の形で流す。結果は target/m424/<形>-<番号>.txt
#   measure.sh <direct|plain|wrap|product> <番号> [セット]
# direct = #423 の「隔離なし」（実行ファイルを直に。パスでモデル）、plain = Sandbox なしで製品の渡し方（fd 3、空の環境変数）、
# wrap = #423 の計測用の包み（sandbox-exec と measure.sb）、product = 製品の檻と同じ app-sandbox だけ（make-bundle.sh）
# 環境変数: IKARI_DATA（辞書とモデルのある data/。既定はこのリポジトリの data/）、SPIKE_GPU（on|cpu|off。既定 on）
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
kind="${1:?plain|wrap|product}"
n="${2:?番号}"
set_file="${3:-$root/eval/sets/homophones-ext.tsv}"
data="${IKARI_DATA:-$root/data}"
bin="$root/target/lm/aarch64-apple-darwin/release"
here="$root/macos/tools"
export IKARI_LM_HOST_EXE="$bin/ikari-lm-host"
case "$kind" in
  direct) host="$bin/ikari-lm-host" ;;
  plain) host="$here/lm-host-sandbox-424/run-plain.sh" ;;
  wrap) host="$here/lm-host-sandbox-424/run-wrap.sh" ;;
  product) host="$here/lm-host-sandbox-424/run-product.sh" ;;
  *) echo "measure.sh: 知らない形: $kind" >&2; exit 2 ;;
esac
mkdir -p "$root/target/m424"
out="$root/target/m424/$kind-${SPIKE_GPU:-on}-$n.txt"
{
  echo "# kind=$kind gpu=${SPIKE_GPU:-on} n=$n start=$(date +%s) uptime=$(uptime)"
  "$bin/ikari-eval" --ranker full --set "$set_file" --dict "$data/build/dict" \
    --lm "$data/raw/zenz/zenz-v3.2-small-Q5_K_M.gguf" --lm-host "$host"
  echo "# end=$(date +%s) uptime=$(uptime)"
} >"$out" 2>&1
echo "$out"
