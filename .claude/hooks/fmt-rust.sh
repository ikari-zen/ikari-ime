#!/bin/bash
# Claude Code の PostToolUse フック（matcher: Edit|Write）。issue #107。
# *.rs を編集したら、そのファイルのあるワークスペースで cargo fmt を流す（check.sh の cargo fmt --check で落とさないため）。
# 整形に失敗しても作業は止めない（常に終了コード 0。失敗は標準エラーに出すだけ）。
# 入力は標準入力の JSON（tool_input.file_path）。jq と cargo が要る。なければ何もしない。
set -uo pipefail

input="$(cat)"
command -v jq >/dev/null 2>&1 || exit 0
command -v cargo >/dev/null 2>&1 || exit 0

file="$(printf '%s' "$input" | jq -r '.tool_input.file_path // empty' 2>/dev/null)" || file=""
[[ "$file" == *.rs && -f "$file" ]] || exit 0

if ! (cd "$(dirname "$file")" && cargo fmt --all) >/dev/null 2>&1; then
  printf 'warning: cargo fmt に失敗しました（%s）。手元で cargo fmt --all を確かめてください\n' "$file" >&2
fi
exit 0
