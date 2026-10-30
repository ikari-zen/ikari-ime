#!/usr/bin/env bash
# リファクタの前後で振る舞いが変わっていないことを確かめるための snapshot（docs/refactor-plan-2026-10.md §6.1、§7.1）。
#
# 取る:
#   snapshot.sh <出力ディレクトリ> [--lm <gguf>] [--ajimee] [--smoke] [--no-ref]
# 取って基準と比べる（--lm / --ajimee / --smoke は基準の meta.env と同じものが既定）:
#   snapshot.sh --compare <基準のディレクトリ> [<出力ディレクトリ>]
#   <出力ディレクトリ> が取得済み（COMPLETE がある）なら、取り直さずに比べるだけ。
#
# 内容（出力ディレクトリの中）:
#   meta.env             コミット、日付、構成、使ったバイナリ
#   dict.sha256          辞書（data/build/dict）の各ファイルの sha256
#   dict-build.txt       辞書を作り直すコマンドと、入力の sha256（作り直した辞書の sha256 と比べる）
#   eval/<名前>.{tsv,md,cmd}   ケース別の結果（--out-cases）、レポート、実行したコマンド
#   scenario/<名前>.md   学習のシナリオ（a-02 / a-03 / a-04 / b23-*）のレポート
#   ref/<名前>.{tsv,md}  LM あり構成を既定の締め切りで流した参考値（比較しない）
#   cli/golden.txt       eval/scripts/cli-golden.tsv の入力に対する ikari の出力
#   smoke.txt            --smoke のときだけ。macos/scripts/smoke.sh --signing team --installed の出力
#   summary.tsv          構成ごとの 1 位正解数、warm p99、LM の時間切れ、取り直した回数
#
# smoke は PASS/FAIL/SKIP の行の集合（括弧の中の pid や時間は除く）で比べる。
# 一致の判定（§7.1）: --out-cases の列 1〜7（id kind context reading expected top1 rank）がバイト単位で一致、
# シナリオのレポートと CLI の出力が一致、辞書の sha256 が一致。warm p99 は基準の 1.1 倍を超えたら警告だけ出す
# （ミリ秒未満の差は揺れるので、絶対差 0.05ms 以下は無視する）。
# LM ありの構成は --lm-budget-ms 2000 --lm-deadline-ms 2000 で流し、時間切れが 0 になるまで最大 3 回流し直す。
# 0 にならなかった構成は INVALID にして、比較の失敗として数える。
#
# 環境変数: IKARI_SNAPSHOT_FLAGS（ikari-eval と ikari に足す引数。基準を語彙集なしで取るときは --no-packs。ADR 0012）、
#           IKARI_DICT_DIR（既定 data/build/dict）、IKARI_EVAL_BIN / IKARI_BIN（バイナリを指定。指定すると build しない）、
#           IKARI_SNAPSHOT_TARGET_DIR（LM つきのバイナリの置き場所。既定 target/snapshot-bin）
# 終了コード: 0 = 取得できた（比較なら差分 0 かつ有効）、1 = 差分または INVALID がある、2 = 実行できなかった。
#
# 中身は `cargo xtask eval snapshot`（xtask/src/eval/snapshot.rs。ADR 0043 段 X4）に移した。ここは呼ぶだけの包み。
set -euo pipefail

# 呼び出し元のディレクトリのまま実行する（相対パスの引数が変わらない。snapshot は xtask の中でリポジトリのルートに移る）
exec cargo run --quiet --manifest-path "$(dirname "${BASH_SOURCE[0]}")/../../Cargo.toml" --package xtask -- eval snapshot "$@"
