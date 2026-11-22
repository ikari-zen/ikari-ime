#!/bin/bash
# スパイク #424（マージしない）。形を交互に、回数ぶん流す（1 本ずつ）。
#   rounds.sh <回数> <形...>        例: rounds.sh 4 direct plain wrap product
# 各回の前に、ほかの重い作業（cargo、rustc、swift、ikari-eval、check.sh）が無いのを確かめる（30 秒ずつ、最大 20 分待つ）。
# 回の間にそれらが現れたら、その回を捨てて（ファイル名に .dirty を付けて残す）流し直す（1 回まで）。
# 各回のあとに、その間の Sandbox の拒否のログ（ikari-lm-host のもの）を数える。
set -uo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
rounds="${1:?回数}"
shift
kinds=("$@")
log="$root/target/m424/rounds.log"
mkdir -p "$root/target/m424"
# 名前で見る（-f だと、ほかのプロセスの引数の文字列に当たる）。check.sh は bash の引数で見る
heavy() { pgrep -x 'cargo|rustc|swift-build|swift-frontend|ikari-eval|xcodebuild|clang' >/dev/null || pgrep -f '^(/bin/)?(ba|z)?sh [^ ]*scripts/check\.sh' >/dev/null; }
busy() { heavy; }
wait_quiet() {
  local waited=0
  while busy; do
    ((waited >= 1200)) && { echo "$(date +%T) still busy after 20min, going on" >>"$log"; return; }
    sleep 30
    waited=$((waited + 30))
  done
}
one() {
  local kind="$1" n="$2" attempt="$3"
  wait_quiet
  local start
  start="$(date '+%Y-%m-%d %H:%M:%S')"
  "$here/measure.sh" "$kind" "$n" >/dev/null &
  local pid=$!
  local dirty=0
  while kill -0 "$pid" 2>/dev/null; do
    sleep 15
    if pgrep -x 'cargo|rustc|swift-build|swift-frontend|xcodebuild|clang' >/dev/null || pgrep -f '^(/bin/)?(ba|z)?sh [^ ]*scripts/check\.sh' >/dev/null || (($(pgrep -x ikari-eval | wc -l) > 1)); then dirty=1; fi
  done
  wait "$pid"
  local out="$root/target/m424/$kind-${SPIKE_GPU:-on}-$n.txt"
  local denies
  denies="$(/usr/bin/log show --start "$start" --style compact \
    --predicate 'sender == "Sandbox" AND eventMessage CONTAINS "ikari-lm-host"' 2>/dev/null | grep -c deny)"
  echo "$(date +%T) kind=$kind gpu=${SPIKE_GPU:-on} n=$n attempt=$attempt dirty=$dirty denies=$denies $(grep -E '^- LM' "$out")" >>"$log"
  if ((dirty)); then
    mv "$out" "$out.dirty"
    ((attempt < 2)) && one "$kind" "$n" $((attempt + 1))
  fi
}
for ((r = 1; r <= rounds; r++)); do
  # 形の順を回ごとにずらす（時間の流れの偏りを散らす）
  for ((k = 0; k < ${#kinds[@]}; k++)); do
    kind="${kinds[$(((k + r - 1) % ${#kinds[@]}))]}"
    one "$kind" "$r" 1
  done
done
echo "$(date +%T) done" >>"$log"
