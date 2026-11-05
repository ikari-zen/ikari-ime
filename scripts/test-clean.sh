#!/bin/bash
# scripts/clean.sh のテスト。一時リポジトリ（と bare の origin）で dry-run と --apply の判定を確かめる。
# gh は gh_pr_states を差し替えて使う（実際の GitHub、実リポジトリ、ユーザーデータには触れない）。
set -euo pipefail

# clean.sh が lib.sh を読む（二重に読むと readonly で失敗する）。main は直接実行のときだけ走る
# shellcheck source=clean.sh
source "$(dirname "${BASH_SOURCE[0]}")/clean.sh"

TMP="$(cd "$(mktemp -d)" && pwd -P)"
trap 'rm -rf "$TMP"' EXIT
readonly TMP
failures=0
fail() { printf 'FAIL: %s\n' "$*" >&2; failures=$((failures + 1)); }

# 偽の gh: ブランチ名ごとの応答。FAKE_PR_<名前の英数字以外を _ にしたもの> に空白区切りの状態、"FAIL" で失敗
gh_pr_states() {
  local v="FAKE_PR_${1//[^A-Za-z0-9]/_}"
  local val="${!v:-}"
  [[ "$val" != FAIL ]] || return 1
  [[ -z "$val" ]] || tr ' ' '\n' <<<"$val"
}
export IKARI_CLEAN_NO_FETCH=1

commit() { git -c user.name=t -c user.email=t@example.com commit -q --allow-empty -m "$1"; }

setup_repo() {
  git init -q --bare -b main "$TMP/origin.git"
  git init -q -b main "$TMP/repo"
  cd "$TMP/repo"
  git remote add origin "$TMP/origin.git"
  commit init
  git push -q origin main
  local b
  for b in merged closed open nopr-local pushed-nopr inmain unknown wt-merged wt-dirty wt-locked used-wt; do
    git branch "$b" main
  done
  for b in merged closed open nopr-local pushed-nopr unknown wt-merged wt-dirty wt-locked used-wt; do
    git checkout -q "$b"
    commit "$b"
  done
  git checkout -q main
  git push -q origin merged closed open pushed-nopr unknown
  git push -q origin inmain:remote-nopr
  git fetch -q origin
  git worktree add -q "$TMP/repo-wt" used-wt
  mkdir -p .claude/worktrees
  git worktree add -q .claude/worktrees/wt-merged wt-merged
  git worktree add -q .claude/worktrees/wt-dirty wt-dirty
  git worktree add -q .claude/worktrees/wt-locked wt-locked
  git worktree lock .claude/worktrees/wt-locked
  touch .claude/worktrees/wt-dirty/uncommitted.txt
  export FAKE_PR_merged=MERGED FAKE_PR_closed=CLOSED FAKE_PR_open=OPEN FAKE_PR_unknown=FAIL
  export FAKE_PR_wt_merged=MERGED FAKE_PR_wt_dirty=MERGED FAKE_PR_wt_locked=MERGED
  REPO_DIR="$TMP/repo"
}

reset_opts() {
  APPLY=0 DEEP=0 FORCE_UNMERGED=0 STALE_DAYS="$DEFAULT_STALE_DAYS" INCLUDES=()
  SEL_BRANCHES=0 SEL_REMOTE=0 SEL_WORKTREES=0 SEL_ARTIFACTS=0 DELETED=0 REVIEW=0 KEPT=0 FAILED=0
}
run() { reset_opts; main "$@" 2>&1; }

expect() { # 出力 パターン 説明
  grep -qE -- "$2" <<<"$1" || fail "$3（期待: $2）"
}
expect_not() {
  ! grep -qE -- "$2" <<<"$1" || fail "$3（あってはいけない: $2）"
}
has_branch() { git -C "$TMP/repo" show-ref --verify --quiet "refs/heads/$1"; }
has_remote() { git -C "$TMP/origin.git" show-ref --verify --quiet "refs/heads/$1"; }

setup_repo

# ---- dry-run
out="$(run)"
expect "$out" '^\[削除予定\] merged — PR は MERGED' "dry-run: MERGED のブランチは削除予定"
expect "$out" '^\[削除予定\] closed — PR は CLOSED' "dry-run: CLOSED のブランチは削除予定"
expect "$out" '^\[削除予定\] inmain — origin/main に完全に入っている' "dry-run: main に入ったブランチは削除予定"
expect "$out" '^\[残す\] +open — 開いている PR' "dry-run: OPEN は残す"
expect "$out" '^\[残す\] +main — main' "dry-run: main は残す"
expect "$out" '^\[残す\] +used-wt — worktree で使用中' "dry-run: worktree で使用中は残す"
expect "$out" '^\[要確認\] nopr-local — .*未 push のコミットが 1 件' "dry-run: 未 push は要確認"
expect "$out" '^\[要確認\] pushed-nopr — PR がなく' "dry-run: PR なしで push 済みは要確認"
expect "$out" '^\[残す\] +unknown — PR の状態が不明' "dry-run: gh 失敗は不明で残す"
expect "$out" '^\[削除予定\] origin/merged — PR は MERGED' "dry-run: リモートの MERGED は削除予定"
expect "$out" '^\[残す\] +origin/open — 開いている PR' "dry-run: リモートの OPEN は残す"
expect "$out" '^\[要確認\] origin/remote-nopr — PR がない' "dry-run: リモートの PR なしは要確認"
expect "$out" '^\[残す\] +origin/unknown — PR の状態が不明' "dry-run: リモートの gh 失敗は残す"
expect "$out" '^\[残す\] +origin/main — main' "dry-run: リモートの main は残す"
expect "$out" 'worktrees/wt-merged — wt-merged: PR は MERGED' "dry-run: PR 済みでクリーンな worktree は削除予定"
expect "$out" '^\[残す\] +.*wt-dirty — 変更が残っている' "dry-run: 変更が残る worktree は残す"
expect "$out" '^\[残す\] +.*wt-locked — ロックされている' "dry-run: ロックされた worktree は残す"
expect "$out" '^\[残す\] +.*repo-wt — 対象の場所ではない' "dry-run: 対象外の場所の worktree は残す"
has_branch merged || fail "dry-run: ブランチが消えた"
[[ -d "$TMP/repo/.claude/worktrees/wt-merged" ]] || fail "dry-run: worktree が消えた"
has_remote merged || fail "dry-run: リモートが消えた"

# ---- --apply（worktree、ローカル、リモート）
run --apply >"$TMP/apply.out" || fail "--apply が失敗した: $(cat "$TMP/apply.out")"
for b in merged closed inmain wt-merged; do
  ! has_branch "$b" || fail "apply: $b が残っている"
done
for b in open nopr-local pushed-nopr unknown wt-dirty wt-locked used-wt main; do
  has_branch "$b" || fail "apply: $b が消えた"
done
[[ ! -d "$TMP/repo/.claude/worktrees/wt-merged" ]] || fail "apply: wt-merged の worktree が残っている"
for d in .claude/worktrees/wt-dirty .claude/worktrees/wt-locked; do
  [[ -d "$TMP/repo/$d" ]] || fail "apply: $d が消えた"
done
[[ -d "$TMP/repo-wt" ]] || fail "apply: 対象外の worktree が消えた"
for b in merged closed; do
  ! has_remote "$b" || fail "apply: origin/$b が残っている"
done
for b in open unknown remote-nopr pushed-nopr main; do
  has_remote "$b" || fail "apply: origin/$b が消えた"
done

# ---- --force-unmerged は --include した名前だけ
out="$(run --branches --force-unmerged)"
expect "$out" '^\[要確認\] nopr-local' "include なしの --force-unmerged は何も消さない"
out="$(run --branches --force-unmerged --include nopr-local --apply)"
! has_branch nopr-local || fail "include: nopr-local が残っている"
has_branch pushed-nopr || fail "include: 指定していない pushed-nopr が消えた"
# 現在のブランチは --include でも消さない
git -C "$TMP/repo" checkout -q pushed-nopr
out="$(run --branches --force-unmerged --include pushed-nopr --apply)"
expect "$out" '^\[残す\] +pushed-nopr — 現在のブランチ' "現在のブランチは --include でも残す"
has_branch pushed-nopr || fail "include: 現在のブランチが消えた"

# ---- 成果物
mkdir -p "$TMP/repo/target/macos/old" "$TMP/repo/target/macos/new" "$TMP/repo/target/debug" \
  "$TMP/repo/eval/out" "$TMP/repo/data/raw" "$TMP/repo/macos/Frameworks"
touch -t 202001010000 "$TMP/repo/target/macos/old"
printf 'target\ndata/\neval/out/\nmacos/Frameworks/\n' >"$TMP/repo/.gitignore"
touch "$TMP/repo/eval/out/x.txt" "$TMP/repo/data/raw/keep.txt" "$TMP/repo/macos/Frameworks/keep.txt"
out="$(run --artifacts)"
expect "$out" '^\[削除予定\] target/macos/old' "成果物: 古い target/macos は削除予定"
expect_not "$out" 'target/macos/new' "成果物: 新しい target/macos は対象外"
expect "$out" '^\[削除予定\] eval/out' "成果物: eval の無視ファイルは削除予定"
expect "$out" '全体は消さない' "成果物: target 全体は既定で消さない"
expect_not "$out" 'data/raw|Frameworks' "成果物: data と Frameworks は触れない"
run --artifacts --apply >/dev/null
[[ ! -e "$TMP/repo/target/macos/old" ]] || fail "成果物 apply: 古いビルドが残る"
[[ -e "$TMP/repo/target/macos/new" ]] || fail "成果物 apply: 新しいビルドが消えた"
[[ ! -e "$TMP/repo/eval/out/x.txt" ]] || fail "成果物 apply: eval/out が残る"
[[ -e "$TMP/repo/data/raw/keep.txt" && -e "$TMP/repo/macos/Frameworks/keep.txt" ]] || fail "成果物 apply: 保護された場所が消えた"
run --artifacts --deep --apply >/dev/null
[[ ! -e "$TMP/repo/target" ]] || fail "--deep: target が残る"
[[ -e "$TMP/repo/data/raw/keep.txt" ]] || fail "--deep: data が消えた"

# ---- 引数の誤り
for bad in "--nope" "--include" "--days x" "--include a"; do
  # shellcheck disable=SC2086
  if (reset_opts; main $bad) >/dev/null 2>&1; then fail "引数の誤りで成功した: $bad"; fi
done
out="$( (reset_opts; main --help) 2>&1 )"
expect "$out" 'dry-run' "--help に使い方が出る"

[[ "$failures" -eq 0 ]] || die "clean.sh のテストが $failures 件失敗しました"
log "clean.sh のテスト: 成功"
