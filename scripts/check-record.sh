#!/bin/bash
# scripts/check.sh の結果の記録（issue #107）。check.sh が書き、.claude/hooks/guard-pr-merge.sh が読む。
# 記録はコミット SHA ごとに 1 ファイル。置き場所は git の共有ディレクトリ（リポジトリの外。worktree 間で共有され、コミットされない）。
# source して使う。依存は git だけ。jj（Jujutsu）の作業コピー（.jj があり jj が使える場所）では、jj で調べる（下の「jj の作業コピー」）。
# 環境変数 IKARI_CHECK_RECORD_DIR で置き場所を変えられる（テスト用）。

readonly CHECK_RECORD_SUBDIR="ikari-check"
readonly CHECK_RECORD_ROOT_COMMIT="0000000000000000000000000000000000000000"

# ---- jj の作業コピー ----
# jj では作業中の変更 @ がコミットで、PR に載るのは @ の親 @- になる（git の HEAD が @- を指す共存の形でも同じ）。
# 記録の対象は @- の SHA（git のコミット ID）。@ に変更があれば、git で作業ツリーが汚れているときと同じに記録しない。
# 共存の形でない jj の workspace（.git がない）でも、jj で調べるので動く。
# jj の上の階層にある git の worktree（.claude/worktrees/ など。jj の外の普通の git）は、jj の作業コピーとみなさない:
# jj のルートと git のルートが一致するか、git がなければ jj とする。
check_record_is_jj() { # <リポジトリ>
  command -v jj >/dev/null 2>&1 || return 1
  local jj_root git_top
  jj_root="$(cd "$1" 2>/dev/null && jj root 2>/dev/null)" || return 1
  git_top="$(git -C "$1" rev-parse --show-toplevel 2>/dev/null)" || return 0
  [[ "$(cd "$jj_root" && pwd -P)" == "$(cd "$git_top" && pwd -P)" ]]
}
# "<@ が空か: true|false> <親の SHA>" を出力する（親が 1 つでなければ失敗。作業コピーの変更は jj が取り込む）
check_record_jj_info() { # <リポジトリ>
  local out empty parents
  out="$(cd "$1" 2>/dev/null && jj log --no-graph -r @ -T 'empty ++ " " ++ parents.map(|p| p.commit_id()).join(",") ++ "\n"' 2>/dev/null)" || return 1
  read -r empty parents <<<"$out"
  [[ -n "$parents" && "$parents" != *,* && "$parents" != "$CHECK_RECORD_ROOT_COMMIT" ]] || return 1
  printf '%s %s\n' "$empty" "$parents"
}

# 記録の置き場所を出力する（git のリポジトリでなければ失敗）
check_record_dir() {
  if [[ -n "${IKARI_CHECK_RECORD_DIR:-}" ]]; then
    printf '%s\n' "$IKARI_CHECK_RECORD_DIR"
    return 0
  fi
  local common
  if common="$(git -C "${1:-.}" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"; then
    printf '%s/%s\n' "$common" "$CHECK_RECORD_SUBDIR"
    return 0
  fi
  # git が使えない jj の workspace（共存の形でない）: jj の裏にある git のディレクトリ
  if check_record_is_jj "${1:-.}"; then
    common="$(cd "${1:-.}" && jj git root 2>/dev/null)" || return 1
    printf '%s/%s\n' "$common" "$CHECK_RECORD_SUBDIR"
    return 0
  fi
  return 1
}

# check_record_write <リポジトリ> <終了コード> <範囲: full|no-swift>
# 未コミットの変更があるときは記録しない（結果が SHA の内容と一致しないため）。同じ SHA の古い記録は消す
check_record_write() {
  local repo="$1" code="$2" scope="$3" dir sha info empty
  dir="$(check_record_dir "$repo")" || return 0
  if check_record_is_jj "$repo"; then
    info="$(check_record_jj_info "$repo")" || return 0
    empty="${info%% *}"
    sha="${info#* }"
    mkdir -p "$dir" 2>/dev/null || return 0
    rm -f "$dir/$sha"
    if [[ "$empty" != "true" ]]; then
      printf 'check の記録: jj の @ に変更があるので記録しません。jj commit などで @ を空にして（PR に載るのは @-）、もう一度 check.sh を流してください\n' >&2
      return 0
    fi
    printf 'CHECK_EXIT=%s\nSCOPE=%s\n' "$code" "$scope" > "$dir/$sha" 2>/dev/null || true
    return 0
  fi
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
