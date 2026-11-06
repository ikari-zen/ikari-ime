#!/usr/bin/env bash
# ADR 0055 §4 の掃引（レーン E）の共通部分。source して使う（単体では動かない）。
# 製品の手順（cargo xtask dict の --data と --webwords-*）で辞書を scratch に作る。実機の data/build は書き換えない。
#
# 関数:
#   ww_prepare_raw <scratch の raw> <data/raw の元> <外来語の一覧 webwords.tsv>
#       元の raw の各項目へのシンボリックリンクと、webwords/webwords.tsv（実体は一覧へのリンク）を持つ raw を作る。
#       元の raw に webwords があっても無視する（一覧は引数のものだけ）。
#   ww_build_dict <scratch の raw> <点のディレクトリ> [cargo xtask dict への追加の引数…]
#       <点>/data/{raw -> scratch の raw, build/} を作り、cargo xtask dict --data <点>/data --rebuild を流す。
#       辞書は <点>/data/build/dict、webwords の lex とレポートは <点>/data/build/ に残る。ログは <点>/build.log。
set -euo pipefail

WW_REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

ww_prepare_raw() {
  local raw=$1 src=$2 list=$3 item
  rm -rf "$raw"
  mkdir -p "$raw/webwords"
  for item in "$src"/*; do
    [ "$(basename "$item")" = webwords ] && continue
    ln -s "$item" "$raw/$(basename "$item")"
  done
  ln -s "$list" "$raw/webwords/webwords.tsv"
}

ww_build_dict() {
  local raw=$1 point=$2
  shift 2
  rm -rf "$point/data"
  mkdir -p "$point/data/build"
  ln -s "$raw" "$point/data/raw"
  (cd "$WW_REPO" && cargo run --quiet --release --package xtask -- dict --data "$point/data" --rebuild "$@") > "$point/build.log" 2>&1
}
