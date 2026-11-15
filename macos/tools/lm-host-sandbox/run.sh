#!/bin/bash
# 計測用（製品では使わない）。ikari-lm-host を、measure.sb の Sandbox の中で起動する包み（ADR 0060 Q4 の (2)）。
# 引数は ikari-lm-host と同じ（--model <GGUF> --gpu on|off）。標準入出力はそのまま渡るので、
# 評価ツールの --lm-host にこのスクリプトを渡すと、隔離ありの起動（load_ms）と往復（lm inference）を測れる:
#
#   IKARI_LM_HOST_EXE=target/lm/release/ikari-lm-host \
#     target/lm/release/ikari-eval --ranker full --set eval/sets/homophones-ext.tsv \
#       --dict data/build/dict --lm <GGUF> --lm-host macos/tools/lm-host-sandbox/run.sh
#
# 環境変数: IKARI_LM_HOST_EXE（ikari-lm-host の場所。必須）
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exe="${IKARI_LM_HOST_EXE:?IKARI_LM_HOST_EXE に ikari-lm-host の場所を設定してください}"
exe="$(cd "$(dirname "$exe")" && pwd)/$(basename "$exe")"
model=""
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
  if [[ "${args[$i]}" == --model ]]; then model="${args[$((i + 1))]:-}"; fi
done
[[ -n "$model" ]] || { echo "run.sh: --model がありません" >&2; exit 2; }
# Sandbox はシンボリックリンクを解いたパスで照合する
model="$(cd "$(dirname "$model")" && pwd -P)/$(basename "$model")"
exec /usr/bin/sandbox-exec -f "$here/measure.sb" \
  -D "HOME=$HOME" -D "MODEL=$model" -D "EXE_DIR=$(dirname "$exe")" \
  "$exe" "$@"
