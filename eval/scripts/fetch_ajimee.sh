#!/usr/bin/env bash
# AJIMEE-Bench（azooKey/AJIMEE-Bench）の評価データを取得する（ADR 0002 §15.2, §21.1 #4）。
#
# データ（JWTD_v2/v1/evaluation_items.json）は CC BY-SA 3.0。リポジトリには入れない。
# 出力先は data/raw/ajimee/（gitignore 済み）。環境変数 IKARI_AJIMEE_DIR で変えられる。
#
# 使い方:
#   eval/scripts/fetch_ajimee.sh
#   cargo run --release -p ikari-eval -- --set data/raw/ajimee/evaluation_items.json --set-format ajimee
#
# 中身は `cargo xtask data fetch --only ajimee`（xtask/src/data/fetch.rs。ADR 0043 段 X3）に移した。ここは呼ぶだけの包み。
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
args=(--only ajimee)
if [[ -n "${IKARI_AJIMEE_DIR:-}" ]]; then args+=(--ajimee-dir "$IKARI_AJIMEE_DIR"); fi
exec cargo run --quiet --manifest-path "$root/Cargo.toml" --package xtask -- data fetch "${args[@]}"
