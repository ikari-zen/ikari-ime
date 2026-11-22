#!/bin/bash
# スパイク #424（マージしない）。評価ツールの --lm-host に渡す包み。製品の檻役と同じく、モデルを fd 3 に付け、
# 環境変数を空にして（ADR 0077 §3.2）、app-sandbox だけの束の ikari-lm-host を起動する。
# SPIKE_GPU があれば --gpu の値を差し替える（原因の切り分け用）。
set -euo pipefail
exe="${TMPDIR:-/tmp}/ikari-lm-host-424/LMHost424.app/Contents/MacOS/ikari-lm-host"
model=""
gpu=""
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
  case "${args[$i]}" in
    --model) model="${args[$((i + 1))]}" ;;
    --gpu) gpu="${args[$((i + 1))]}" ;;
  esac
done
[[ -n "$model" ]] || { echo "run-product.sh: --model がありません" >&2; exit 2; }
gpu="${SPIKE_GPU:-${gpu:-on}}"
exec 3<"$model"
exec /usr/bin/env -i "$exe" --model /dev/fd/3 --gpu "$gpu"
