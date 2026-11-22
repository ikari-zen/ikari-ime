#!/bin/bash
# スパイク #424（マージしない）。#423 の計測用の包み（../lm-host-sandbox/run.sh と measure.sb）と同じ起動。
# 違いは SPIKE_GPU で --gpu を差し替えられることだけ（原因の切り分け用）
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/../lm-host-sandbox" && pwd)"
exe="${IKARI_LM_HOST_EXE:?IKARI_LM_HOST_EXE}"
exe="$(cd "$(dirname "$exe")" && pwd)/$(basename "$exe")"
model=""
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
  case "${args[$i]}" in
    --model) model="${args[$((i + 1))]}" ;;
    --gpu) [[ -n "${SPIKE_GPU:-}" ]] && args[$((i + 1))]="$SPIKE_GPU" ;;
  esac
done
[[ -n "$model" ]] || { echo "run-wrap.sh: --model がありません" >&2; exit 2; }
model="$(cd "$(dirname "$model")" && pwd -P)/$(basename "$model")"
exec /usr/bin/sandbox-exec -f "$here/measure.sb" \
  -D "HOME=$HOME" -D "MODEL=$model" -D "EXE_DIR=$(dirname "$exe")" \
  "$exe" "${args[@]}"
