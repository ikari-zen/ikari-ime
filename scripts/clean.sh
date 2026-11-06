#!/bin/bash
# 開発用の掃除（使っていないブランチ、worktree、成果物を消す）。macOS 専用。
#
#   scripts/clean.sh                      何をなぜ消すかの一覧だけ出す（dry-run。何も消さない）
#   scripts/clean.sh --apply              一覧の「削除」だけを実際に消す
#   scripts/clean.sh --branches           ローカルのブランチだけ
#   scripts/clean.sh --remote             origin のブランチだけ
#   scripts/clean.sh --worktrees          worktree だけ
#   scripts/clean.sh --artifacts          成果物だけ
#   --deep                                成果物で target/ 全体を消す対象にする
#   --days N                              成果物の「古い」の日数（既定 14）
#   --force-unmerged --include 名前       「要確認」のローカルブランチを、名前を指定して消す（何回でも指定できる）
#
# 触らないもの: ユーザーデータ（~/Library/Application Support/ikari-ime/）、data/、macos/Frameworks、
#   ~/Library/Input Methods/、実機のプロセス。main、現在のブランチ、開いている PR のブランチ。
#   履歴の書き換えと force push はしない。リモートの削除は `git push origin --delete` だけ。
# PR の状態は gh で調べる（スカッシュマージは merge-base では分からない）。gh が失敗したものは「不明」として消さない。
set -euo pipefail

# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

readonly MAIN_BRANCH="main"
readonly REMOTE_NAME="origin"
readonly DEFAULT_STALE_DAYS=14
# 無視ファイルのうち、掃除の対象にする最上位のディレクトリ（これ以外は触らない）
readonly ARTIFACT_IGNORED_ROOTS=(eval scratch)
# 対象の中でも消さないパス（前方一致）
readonly ARTIFACT_PROTECTED=(data macos/Frameworks)

REPO_DIR="$IKARI_ROOT"
APPLY=0
DEEP=0
FORCE_UNMERGED=0
STALE_DAYS="$DEFAULT_STALE_DAYS"
INCLUDES=()
SEL_BRANCHES=0 SEL_REMOTE=0 SEL_WORKTREES=0 SEL_ARTIFACTS=0
DELETED=0 REVIEW=0 KEPT=0 FAILED=0

usage() {
  sed -n '2,/^set -euo/p' "$IKARI_SCRIPTS_DIR/clean.sh" | sed '$d' | sed 's/^# \{0,1\}//'
}

g() { git -C "$REPO_DIR" "$@"; }

# ---- gh（テストでは差し替える） ----
# ブランチ名の PR の状態（OPEN / MERGED / CLOSED）を 1 行ずつ出す。PR がなければ何も出さない。失敗したら非 0。
gh_pr_states() {
  (cd "$REPO_DIR" && gh pr list --head "$1" --state all --json state --jq '.[].state')
}

# ---- 出力 ----
emit() { # 種別 対象 理由
  local kind="$1" target="$2" reason="$3"
  case "$kind" in
    delete) if [[ "$APPLY" -eq 1 ]]; then printf '[削除]   %s — %s\n' "$target" "$reason"; else printf '[削除予定] %s — %s\n' "$target" "$reason"; fi ;;
    review) printf '[要確認] %s — %s\n' "$target" "$reason"; REVIEW=$((REVIEW + 1)) ;;
    keep) printf '[残す]   %s — %s\n' "$target" "$reason"; KEPT=$((KEPT + 1)) ;;
  esac
}
section() { printf '\n== %s ==\n' "$1"; }
human_size() { du -sh "$1" 2>/dev/null | cut -f1 | tr -d ' '; }

# ---- 判定 ----
# PR の状態から DECISION（delete / keep / none）と REASON を決める
DECISION="" REASON=""
decide_by_pr() {
  local name="$1" states
  if ! states="$(gh_pr_states "$name" 2>/dev/null)"; then
    DECISION=keep REASON="PR の状態が不明（gh が失敗した）"
    return 0
  fi
  if grep -qx OPEN <<<"$states"; then
    DECISION=keep REASON="開いている PR がある"
  elif grep -qx MERGED <<<"$states"; then
    DECISION=delete REASON="PR は MERGED"
  elif grep -qx CLOSED <<<"$states"; then
    DECISION=delete REASON="PR は CLOSED"
  else
    DECISION=none REASON="PR がない"
  fi
}

# ローカルブランチの判定。DECISION は delete / keep / review
decide_local() {
  local b="$1" n
  decide_by_pr "$b"
  [[ "$DECISION" == none ]] || return 0
  if g show-ref --verify --quiet "refs/remotes/$REMOTE_NAME/$b"; then
    DECISION=review REASON="PR がなく、$REMOTE_NAME/$b がある（作業の途中かもしれない）"
  elif g merge-base --is-ancestor "$b" "$REMOTE_NAME/$MAIN_BRANCH" 2>/dev/null; then
    DECISION=delete REASON="$REMOTE_NAME/$MAIN_BRANCH に完全に入っている"
  else
    n="$(g rev-list --count "$b" --not --remotes="$REMOTE_NAME")"
    DECISION=review REASON="PR がなく、$REMOTE_NAME にない未 push のコミットが ${n} 件ある"
  fi
}

# worktree の情報（git worktree list --porcelain）を「パス|ブランチ|フラグ」の行にする。先頭がメインの worktree
worktree_rows() {
  g worktree list --porcelain | awk '
    function flush() { if (path != "") print path "|" branch "|" flags; path=""; branch=""; flags="" }
    /^worktree / { flush(); path=substr($0, 10); next }
    /^branch / { b=substr($0, 8); sub("^refs/heads/", "", b); branch=b; next }
    /^locked/ { flags=flags "L"; next }
    /^prunable/ { flags=flags "P"; next }
    /^detached/ { flags=flags "D"; next }
    END { flush() }'
}

branch_in_worktree() { # 名前 → いずれかの worktree で使用中か
  worktree_rows | cut -d'|' -f2 | grep -qxF "$1"
}

# ---- ローカルのブランチ ----
do_branches() {
  section "ローカルのブランチ"
  local current b
  current="$(g branch --show-current)"
  while IFS= read -r b; do
    [[ -n "$b" ]] || continue
    if [[ "$b" == "$MAIN_BRANCH" ]]; then emit keep "$b" "main"; continue; fi
    if [[ "$b" == "$current" ]]; then emit keep "$b" "現在のブランチ"; continue; fi
    if branch_in_worktree "$b"; then emit keep "$b" "worktree で使用中"; continue; fi
    decide_local "$b"
    case "$DECISION" in
      delete)
        emit delete "$b" "$REASON"
        if [[ "$APPLY" -eq 1 ]]; then
          if g branch -D "$b" >/dev/null; then DELETED=$((DELETED + 1)); else FAILED=$((FAILED + 1)); warn "$b を消せなかった"; fi
        fi ;;
      review)
        if [[ "$FORCE_UNMERGED" -eq 1 ]] && printf '%s\n' "${INCLUDES[@]+"${INCLUDES[@]}"}" | grep -qxF "$b"; then
          emit delete "$b" "${REASON}（--force-unmerged --include で指定）"
          if [[ "$APPLY" -eq 1 ]]; then
            if g branch -D "$b" >/dev/null; then DELETED=$((DELETED + 1)); else FAILED=$((FAILED + 1)); warn "$b を消せなかった"; fi
          fi
        else
          emit review "$b" "$REASON"
        fi ;;
      *) emit keep "$b" "$REASON" ;;
    esac
  done < <(g for-each-ref --format='%(refname:short)' refs/heads/)
}

# ---- リモートのブランチ ----
do_remote() {
  section "リモートのブランチ（${REMOTE_NAME}）"
  local ref b
  while IFS= read -r ref; do
    b="${ref#"$REMOTE_NAME"/}"
    [[ "$b" != "HEAD" && "$ref" != "$REMOTE_NAME" ]] || continue
    if [[ "$b" == "$MAIN_BRANCH" ]]; then emit keep "$ref" "main"; continue; fi
    decide_by_pr "$b"
    case "$DECISION" in
      delete)
        emit delete "$ref" "$REASON"
        if [[ "$APPLY" -eq 1 ]]; then
          if g push "$REMOTE_NAME" --delete "$b" >/dev/null 2>&1; then DELETED=$((DELETED + 1)); else FAILED=$((FAILED + 1)); warn "$ref を消せなかった"; fi
        fi ;;
      none) emit review "$ref" "PR がない" ;;
      *) emit keep "$ref" "$REASON" ;;
    esac
  done < <(g for-each-ref --format='%(refname:short)' "refs/remotes/$REMOTE_NAME/")
}

# ---- worktree ----
do_worktrees() {
  section "worktree"
  local first=1 row path branch flags top parent root_wt
  top="$(g rev-parse --show-toplevel)"
  parent="$(dirname "$(worktree_rows | head -n1 | cut -d'|' -f1)")"
  root_wt="$(worktree_rows | head -n1 | cut -d'|' -f1)"
  while IFS= read -r row; do
    path="${row%%|*}"; row="${row#*|}"; branch="${row%%|*}"; flags="${row#*|}"
    if [[ "$first" -eq 1 ]]; then first=0; continue; fi
    # 対象は .claude/worktrees/ と、リポジトリの隣の ikari-ime-*
    case "$path" in
      "$root_wt"/.claude/worktrees/*) ;;
      "$parent"/ikari-ime-*) ;;
      *) emit keep "$path" "対象の場所ではない"; continue ;;
    esac
    if [[ "$path" == "$top" ]]; then emit keep "$path" "いま作業している worktree"; continue; fi
    if [[ "$flags" == *L* ]]; then emit keep "$path" "ロックされている"; continue; fi
    if [[ "$flags" == *P* ]]; then emit keep "$path" "場所がない（git worktree prune で消える）"; continue; fi
    if [[ -z "$branch" ]]; then emit keep "$path" "ブランチがない（detached）"; continue; fi
    if [[ -n "$(git -C "$path" status --porcelain 2>/dev/null)" ]]; then emit keep "$path" "変更が残っている（${branch}）"; continue; fi
    decide_local "$branch"
    if [[ "$DECISION" == delete ]]; then
      emit delete "$path" "$branch: ${REASON}、作業ツリーはきれい（$(human_size "$path")）"
      if [[ "$APPLY" -eq 1 ]]; then
        if g worktree remove "$path" >/dev/null 2>&1; then DELETED=$((DELETED + 1)); else FAILED=$((FAILED + 1)); warn "$path を消せなかった"; fi
      fi
    else
      emit keep "$path" "$branch: $REASON"
    fi
  done < <(worktree_rows)
  if [[ "$APPLY" -eq 1 ]]; then g worktree prune; else g worktree prune --dry-run | sed 's/^/(prune 予定) /'; fi
}

# ---- 成果物 ----
is_protected() { # リポジトリからの相対パス
  local p="$1" d
  for d in "${ARTIFACT_PROTECTED[@]}"; do
    [[ "$p" == "$d" || "$p" == "$d"/* ]] && return 0
  done
  return 1
}

remove_path() { # 対象 理由。REPO_DIR の下だけ消す
  local path="$1" reason="$2"
  case "$path" in
    "$REPO_DIR"/target|"$REPO_DIR"/target/*|"$REPO_DIR"/eval/*|"$REPO_DIR"/scratch/*) ;;
    *) die "対象外のパスは消せません: $path" ;;
  esac
  emit delete "${path#"$REPO_DIR"/}" "${reason}（$(human_size "$path")）"
  if [[ "$APPLY" -eq 1 ]]; then
    if rm -rf "$path"; then DELETED=$((DELETED + 1)); else FAILED=$((FAILED + 1)); warn "$path を消せなかった"; fi
  fi
}

do_artifacts() {
  section "成果物"
  local t="$REPO_DIR/target" p root rel
  if [[ "$DEEP" -eq 1 && -d "$t" ]]; then
    remove_path "$t" "--deep: target/ 全体"
  elif [[ -d "$t" ]]; then
    if [[ -d "$t/macos" ]]; then
      while IFS= read -r p; do
        [[ -n "$p" ]] || continue
        remove_path "$p" "target/macos の ${STALE_DAYS} 日より古いビルド"
      done < <(find "$t/macos" -mindepth 1 -maxdepth 1 -mtime "+$STALE_DAYS" | sort)
    fi
    if command -v cargo-sweep >/dev/null 2>&1; then
      if [[ "$APPLY" -eq 1 ]]; then
        emit delete "target/（cargo sweep）" "${STALE_DAYS} 日より古いビルドの成果物"
        if (cd "$REPO_DIR" && cargo sweep --time "$STALE_DAYS" >/dev/null 2>&1); then DELETED=$((DELETED + 1)); else FAILED=$((FAILED + 1)); warn "cargo sweep が失敗した"; fi
      else
        emit delete "target/（cargo sweep）" "${STALE_DAYS} 日より古いビルドの成果物（$(cd "$REPO_DIR" && cargo sweep --time "$STALE_DAYS" --dry-run 2>&1 | tail -n1)）"
      fi
    elif [[ -d "$t/debug/incremental" ]]; then
      emit keep "target/debug/incremental" "cargo-sweep がないので消さない（$(human_size "$t/debug/incremental")。cargo install cargo-sweep で掃除できる）"
    fi
    emit keep "target/" "全体は消さない（$(human_size "$t")。--deep で消す）"
  fi
  # 無視ファイル（eval/ と scratch/ の下だけ）
  while IFS= read -r -d '' rel; do
    [[ -n "$rel" ]] || continue
    for root in "${ARTIFACT_IGNORED_ROOTS[@]}"; do
      [[ "$rel" == "$root"/* ]] || continue
      if is_protected "$rel"; then emit keep "$rel" "保護された場所"; else remove_path "$REPO_DIR/$rel" "git が無視している一時ファイル"; fi
    done
  done < <(g ls-files -z --others --ignored --exclude-standard -- "${ARTIFACT_IGNORED_ROOTS[@]}")
  if [[ "$APPLY" -eq 1 ]]; then
    for root in "${ARTIFACT_IGNORED_ROOTS[@]}"; do
      # 空になったディレクトリだけ片づける（追跡されているファイルがあるディレクトリは空にならない）
      [[ ! -d "$REPO_DIR/$root" ]] || find "$REPO_DIR/$root" -mindepth 1 -type d -empty -delete
    done
  fi
}

# ---- 引数 ----
parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --apply) APPLY=1 ;;
      --branches) SEL_BRANCHES=1 ;;
      --remote) SEL_REMOTE=1 ;;
      --worktrees) SEL_WORKTREES=1 ;;
      --artifacts) SEL_ARTIFACTS=1 ;;
      --deep) DEEP=1 ;;
      --force-unmerged) FORCE_UNMERGED=1 ;;
      --include) [[ $# -ge 2 ]] || { usage >&2; die "--include には名前が要ります"; }; INCLUDES+=("$2"); shift ;;
      --days) [[ $# -ge 2 && "$2" =~ ^[0-9]+$ ]] || { usage >&2; die "--days には整数が要ります"; }; STALE_DAYS="$2"; shift ;;
      -h|--help) usage; exit 0 ;;
      *) usage >&2; die "知らない引数: $1" ;;
    esac
    shift
  done
  if [[ "${#INCLUDES[@]}" -gt 0 && "$FORCE_UNMERGED" -ne 1 ]]; then
    usage >&2; die "--include は --force-unmerged と一緒に使います"
  fi
  if [[ $((SEL_BRANCHES + SEL_REMOTE + SEL_WORKTREES + SEL_ARTIFACTS)) -eq 0 ]]; then
    SEL_BRANCHES=1 SEL_REMOTE=1 SEL_WORKTREES=1 SEL_ARTIFACTS=1
  fi
}

main() {
  parse_args "$@"
  if [[ "${IKARI_CLEAN_NO_FETCH:-0}" != 1 ]]; then
    g fetch --quiet --prune "$REMOTE_NAME" || warn "fetch に失敗した（古い情報で判定する）"
  fi
  g rev-parse --verify --quiet "$REMOTE_NAME/$MAIN_BRANCH" >/dev/null || die "$REMOTE_NAME/$MAIN_BRANCH がありません"
  if [[ "$APPLY" -eq 1 ]]; then log "実際に消します（--apply）"; else log "一覧だけ出します（dry-run。消すには --apply）"; fi
  # worktree を先に消すと、そのブランチが次の段で消せる
  [[ "$SEL_WORKTREES" -eq 0 ]] || do_worktrees
  [[ "$SEL_BRANCHES" -eq 0 ]] || do_branches
  [[ "$SEL_REMOTE" -eq 0 ]] || do_remote
  [[ "$SEL_ARTIFACTS" -eq 0 ]] || do_artifacts
  printf '\n'
  if [[ "$APPLY" -eq 1 ]]; then
    log "削除 $DELETED 件、失敗 $FAILED 件、要確認 $REVIEW 件"
  else
    log "要確認 $REVIEW 件。[削除予定] を消すには --apply"
  fi
  [[ "$FAILED" -eq 0 ]]
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
