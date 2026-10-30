#!/bin/bash
# コミットの前に流す門（CI を置かない代わり。docs/refactor-plan-2026-10.md §5.5、§6.2）。
#
#   scripts/check.sh                 文書とスクリプトの検査、cargo fmt --check、clippy -D warnings、cargo test --workspace、swift test
#   scripts/check.sh --lm            + ikari-lm / ikari-engine-ffi / ikari-stack を feature つきで clippy と test（cmake が要る）
#   scripts/check.sh --real          + 実データの #[ignore] テスト（辞書とモデルを使う。--lm を含む）
#   scripts/check.sh --perf          + 性能の #[ignore] テスト（release）
#   scripts/check.sh --eval <基準>   + eval/scripts/snapshot.sh --compare <基準>（振る舞いが変わっていないことの確認）
#   scripts/check.sh --docs-only     文書とスクリプトの検査だけ（数秒）
#   scripts/check.sh --no-swift      swift test を飛ばす（macOS 以外）
#
# --real の前提（setup.sh と同じ既定の場所。環境変数で変えられる）:
#   IKARI_DICT_DIR   辞書（既定 data/build/dict）
#   IKARI_ZENZ_MODEL zenz のモデル（既定 data/raw/zenz/zenz-v3.2-small-Q5_K_M.gguf）
#   IKARI_EDGE_MODEL kanakanji-edge のモデル（既定 data/raw/kanakanji-edge/kanakanji-edge-33m-Q8_0.gguf）
# 前提がなければ失敗する（黙って成功しない）。
#
# 中身は `cargo xtask check`（xtask/src/check.rs。ADR 0043 段 X1）に移した。ここは呼ぶだけの包み。
# 結果の記録（ikari-check/<SHA>）の書式は scripts/check-record.sh と同じ。
set -euo pipefail

# 呼び出し元のディレクトリのまま実行する（相対パスの引数が変わらない）
exec cargo run --quiet --manifest-path "$(dirname "${BASH_SOURCE[0]}")/../Cargo.toml" --package xtask -- check "$@"
