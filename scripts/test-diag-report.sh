#!/bin/bash
# scripts/diag-report.py の出力が、固定のログ（scripts/testdata/diag/）に対する期待（expected.md）と一致するか確かめる。
# 期待を更新するとき: scripts/test-diag-report.sh --update
# テストでは、ユーザーのログ（~/Library/Application Support/ikari-ime/logs/）は読まない。
set -euo pipefail

# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "$IKARI_ROOT"

readonly DATA="scripts/testdata/diag"
readonly EXPECTED="$DATA/expected.md"
readonly REPORT="scripts/diag-report.py"

command -v python3 >/dev/null || die "python3 がありません"

if [[ "${1:-}" == "--update" ]]; then
  python3 -I "$REPORT" "$DATA" > "$EXPECTED"
  log "更新しました: $EXPECTED"
  exit 0
fi

actual="$(python3 -I "$REPORT" "$DATA")"
[[ "$actual" == "$(cat "$EXPECTED")" ]] || {
  diff <(printf '%s\n' "$actual") "$EXPECTED" >&2 || true
  die "diag-report.py の出力が $EXPECTED と違います"
}

# ファイルを直接渡しても（順序が逆でも）同じ出力になる
files="$(python3 -I "$REPORT" "$DATA/engine.log" "$DATA/engine-20261025-000000.log.gz")"
[[ "$files" == "$actual" ]] || die "ファイル指定とディレクトリ指定で出力が違います"

# TSV は行の先頭に節名がつく
tsv="$(python3 -I "$REPORT" --tsv "$DATA")"
printf '%s\n' "$tsv" | grep -q '^ログの健全性	読めた行	33$' || die "TSV に期待の行がありません"

# 日付の絞り込み（2026-10-27 だけ → 古い世代の行は入らない）
since="$(python3 -I "$REPORT" --since 2026-10-27 "$DATA")"
printf '%s\n' "$since" | grep -q '^| 読めた行 | 26 |$' || die "--since の結果が違います"

# smoke のセッションは既定で除く（--include-smoke で戻す）
printf '%s\n' "$actual" | grep -q '^| 除いた smoke の行 | 3 |$' || die "smoke の行を除けていません"
smoke="$(python3 -I "$REPORT" --include-smoke "$DATA")"
printf '%s\n' "$smoke" | grep -q '^| smoke の行（除かない） | 3 |$' || die "--include-smoke の結果が違います"
printf '%s\n' "$smoke" | grep -q '^| pick=5 | 1 | 33.3% |$' || die "--include-smoke で smoke の合成が入っていません"
printf '%s\n' "$actual" | grep -q '^| pick=5 | 0 | 0.0% |$' || die "smoke の合成が既定で入っています"

log "diag-report のテスト: OK"
