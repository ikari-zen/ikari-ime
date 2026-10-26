#!/usr/bin/env bash
# 期待が N 位以内かを調べる（ikari-eval の評価セットには N 位以内の書式がないため。brand-katakana.tsv の回帰用。#223）。
#   eval/scripts/rank-within.sh <セット.tsv> [N=2] [ikari-eval の追加引数...]
# tags に known がある行は、N 位以内でなくても失敗に数えない（今は届かない既知の行）。known なのに N 位以内に入った行は「印を外す」と出す。
# tags に within-N がある行は、その行だけ N を上書きする（postal.tsv のように行ごとに位が違うセット用。#203）。
# 終了コード: 0 = 失敗なし、1 = known でない行が N 位以内に入っていない。
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
set_file="${1:?使い方: rank-within.sh <セット.tsv> [N] [ikari-eval の追加引数...]}"
limit="${2:-2}"
shift $(( $# < 2 ? $# : 2 ))
out="$(mktemp)"
trap 'rm -f "$out"' EXIT
cd "$root"
cargo run --release -q -p ikari-eval -- --ranker full --repeat 1 "$@" --set "$set_file" --out-cases "$out" > /dev/null
awk -F'\t' -v n="$limit" '
  NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; next }
  {
    lim = n
    if (match($col["tags"], /(^|,)within-[0-9]+(,|$)/)) { t = substr($col["tags"], RSTART, RLENGTH); gsub(/[^0-9]/, "", t); lim = t + 0 }
    rank = $col["rank"]; ok = (rank != "" && rank + 0 <= lim); known = ($col["tags"] ~ /(^|,)known(,|$)/)
    total++
    if (ok) { pass++; if (known) print "印を外す: " $1 " " $col["reading"] " " $col["expected"] " (rank " rank ")" }
    else if (known) { skipped++ }
    else { fail++; print "失敗: " $1 " " $col["reading"] " " $col["expected"] " top1=" $col["top1"] " (rank " rank ")" }
  }
  END { printf "%d 件中 %d 件が %d 位以内（既知の未達 %d、失敗 %d）\n", total, pass, n, skipped, fail; exit (fail > 0) }
' "$out"
