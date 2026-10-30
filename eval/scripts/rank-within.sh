#!/usr/bin/env bash
# 期待が N 位以内かを調べる（ikari-eval の評価セットには N 位以内の書式がないため。brand-katakana.tsv の回帰用。#223）。
#   eval/scripts/rank-within.sh <セット.tsv> [N=2] [ikari-eval の追加引数...]
# tags に known がある行は、N 位以内でなくても失敗に数えない（今は届かない既知の行）。known なのに N 位以内に入った行は「印を外す」と出す。
# tags に within-N がある行は、その行だけ N を上書きする（postal.tsv のように行ごとに位が違うセット用。#203）。
# 終了コード: 0 = 失敗なし、1 = known でない行が N 位以内に入っていない。
#
# 中身は `cargo xtask eval rank-within`（xtask/src/eval/rank_within.rs。ADR 0043 段 X4）に移した。ここは呼ぶだけの包み。
# セットの相対パスは、これまでと同じくリポジトリのルートからになる。
set -euo pipefail

exec cargo run --quiet --manifest-path "$(dirname "${BASH_SOURCE[0]}")/../../Cargo.toml" --package xtask -- eval rank-within "$@"
