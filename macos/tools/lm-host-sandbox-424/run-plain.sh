#!/bin/bash
# スパイク #424（マージしない）。対照: Sandbox なしで、製品と同じ渡し方（fd 3、空の環境変数）。SPIKE_GPU で --gpu を差し替え
set -euo pipefail
exe="${IKARI_LM_HOST_EXE:?IKARI_LM_HOST_EXE}"
model=""
gpu=""
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
  case "${args[$i]}" in
    --model) model="${args[$((i + 1))]}" ;;
    --gpu) gpu="${args[$((i + 1))]}" ;;
  esac
done
[[ -n "$model" ]] || { echo "run-plain.sh: --model がありません" >&2; exit 2; }
gpu="${SPIKE_GPU:-${gpu:-on}}"
exec 3<"$model"
exec /usr/bin/env -i "$exe" --model /dev/fd/3 --gpu "$gpu"
