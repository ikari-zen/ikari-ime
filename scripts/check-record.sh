#!/bin/bash
# scripts/check.sh の結果の記録（issue #107）。check.sh が書き、.claude/hooks/guard-pr-merge.sh が読む。
# 記録はコミット SHA ごとに 1 ファイル。置き場所は git の共有ディレクトリ（リポジトリの外。worktree 間で共有され、コミットされない）。
# source して使う。依存は git だけ。
# 環境変数 IKARI_CHECK_RECORD_DIR で置き場所を変えられる（テスト用）。

readonly CHECK_RECORD_SUBDIR="ikari-check"

# 記録の置き場所を出力する（git のリポジトリでなければ失敗）
check_record_dir() {
  if [[ -n "${IKARI_CHECK_RECORD_DIR:-}" ]]; then
    printf '%s\n' "$IKARI_CHECK_RECORD_DIR"
    return 0
  fi
  local common
  common="$(git -C "${1:-.}" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" || return 1
  printf '%s/%s\n' "$common" "$CHECK_RECORD_SUBDIR"
}

# check_record_write <リポジトリ> <終了コード> <範囲: full|no-swift>
# 未コミットの変更があるときは記録しない（結果が SHA の内容と一致しないため）。同じ SHA の古い記録は消す
check_record_write() {
  local repo="$1" code="$2" scope="$3" dir sha
  dir="$(check_record_dir "$repo")" || return 0
  sha="$(git -C "$repo" rev-parse HEAD 2>/dev/null)" || return 0
  mkdir -p "$dir" 2>/dev/null || return 0
  rm -f "$dir/$sha"
  if [[ -n "$(git -C "$repo" status --porcelain 2>/dev/null)" ]]; then
    return 0
  fi
  printf 'CHECK_EXIT=%s\nSCOPE=%s\n' "$code" "$scope" > "$dir/$sha" 2>/dev/null || true
}

# check_record_read <リポジトリ> <SHA>  記録の内容を出力する。なければ失敗
check_record_read() {
  local dir
  dir="$(check_record_dir "$1")" || return 1
  [[ -f "$dir/$2" ]] || return 1
  cat "$dir/$2"
}
