#!/bin/bash
# 辞書とモデルを ~/Library/Application Support/ikari-ime/ に入れる（ADR 0004 §8.3）。
#
#   scripts/install-data.sh [--src <data のディレクトリ>] [--dest <ikari-ime のディレクトリ>] [--no-model] [--dry-run]
#
#   <src>/build/dict/{grammar.bin,system.dict,vectors.bin}            -> <dest>/dict/
#   <src>/build/dict/emoji.tsv（あれば。なくてもよい）               -> <dest>/dict/
#   <src>/build/dict/postal.tsv（あれば。なくてもよい）              -> <dest>/dict/
#   <src>/raw/zenz/zenz-v3.2-small-Q5_K_M.gguf                         -> <dest>/models/
#
# コピーは同じディレクトリの一時ファイルに書いてから rename する。エンジンが mmap している最中のファイルを
# 直接書き換えない（古い inode はエンジンが閉じるまで生きる）。内容が同じファイルは飛ばす。
# --src の既定: このリポジトリの data/。なければ（git worktree のとき）本体のチェックアウトの data/。
# --dest の既定: ${IKARI_USER_DIR}（Rust 側と同じ名前）、なければ ~/Library/Application Support/ikari-ime。
#   旧名 $IKARI_DATA_DIR も当面は読む（警告を出す）。
#
# 中身は `cargo xtask data install`（xtask/src/data/install.rs。ADR 0043 段 X3）に移した。ここは呼ぶだけの包み。
set -euo pipefail

# 呼び出し元のディレクトリのまま実行する（相対パスの引数が変わらない）
exec cargo run --quiet --manifest-path "$(dirname "${BASH_SOURCE[0]}")/../Cargo.toml" --package xtask -- data install "$@"
