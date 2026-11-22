#!/bin/bash
# スパイク #424（マージしない）。rounds.log が <行数> になるまで待つ（最大 <秒>。既定 570）。終わったら中身を出す
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
log="$root/target/m424/rounds.log"
want="${1:?行数}"
limit="${2:-570}"
t=0
while (( t < limit )); do
  [[ -f "$log" ]] && (( $(wc -l <"$log") >= want )) && break
  sleep 15
  t=$((t + 15))
done
cat "$log" 2>/dev/null
uptime
