#!/bin/bash
# Claude Code のフック（.claude/hooks/）と、check.sh の結果の記録（scripts/check-record.sh）のテスト。issue #107。
# 本物の gh、cargo は使わない（一時ディレクトリに偽物を置く）。ユーザーのデータや実リポジトリの記録には触れない。
set -euo pipefail

# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "$IKARI_ROOT"

command -v jq >/dev/null || die "jq がありません（brew install jq）"

readonly GUARD="$IKARI_ROOT/.claude/hooks/guard-pr-merge.sh"
readonly FMT="$IKARI_ROOT/.claude/hooks/fmt-rust.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
readonly TMP
readonly FAKE_BIN="$TMP/bin"
readonly REPO="$TMP/repo"
readonly SHA_A="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
mkdir -p "$FAKE_BIN" "$REPO"

failures=0
fail() { printf 'FAIL: %s\n' "$*" >&2; failures=$((failures + 1)); }

# 偽の gh: 引数を $FAKE_GH_ARGS に残し、FAKE_GH_SHA を返す（FAKE_GH_FAIL=1 なら失敗する）
cat > "$FAKE_BIN/gh" <<'G'
#!/bin/bash
printf '%s\n' "$*" > "$FAKE_GH_ARGS"
[[ "${FAKE_GH_FAIL:-0}" == "1" ]] && exit 1
printf '%s\n' "$FAKE_GH_SHA"
G
# 偽の cargo: 呼ばれた場所と引数を残す（FAKE_CARGO_FAIL=1 なら失敗する）
cat > "$FAKE_BIN/cargo" <<'G'
#!/bin/bash
printf '%s|%s\n' "$PWD" "$*" >> "$FAKE_CARGO_LOG"
[[ "${FAKE_CARGO_FAIL:-0}" == "1" ]] && exit 1
exit 0
G
chmod +x "$FAKE_BIN/gh" "$FAKE_BIN/cargo"
export FAKE_GH_ARGS="$TMP/gh.args" FAKE_CARGO_LOG="$TMP/cargo.log"

git -C "$REPO" init -q
git -C "$REPO" -c user.name=t -c user.email=t@example.com commit -q --allow-empty -m init
HEAD_SHA="$(git -C "$REPO" rev-parse HEAD)"
export IKARI_CHECK_RECORD_DIR="$TMP/records"

# ---- scripts/check-record.sh
# shellcheck source=check-record.sh
source "$IKARI_ROOT/scripts/check-record.sh"

check_record_write "$REPO" 0 full
[[ "$(check_record_read "$REPO" "$HEAD_SHA")" == *"CHECK_EXIT=0"* ]] || fail "記録: きれいな作業ツリーで 0 が記録されない"
check_record_write "$REPO" 3 no-swift
[[ "$(check_record_read "$REPO" "$HEAD_SHA")" == *"CHECK_EXIT=3"* ]] || fail "記録: 新しい結果で上書きされない"
touch "$REPO/dirty.txt"
check_record_write "$REPO" 0 full
check_record_read "$REPO" "$HEAD_SHA" >/dev/null && fail "記録: 未コミットの変更があるのに記録された（古い記録も消えるはず）"
rm "$REPO/dirty.txt"
check_record_read "$REPO" "$SHA_A" >/dev/null && fail "記録: 知らない SHA の記録が読めた"

# ---- guard-pr-merge.sh
# run_guard <コマンド>  → 終了コードを返す（標準エラーは $TMP/guard.err）
run_guard() {
  jq -n --arg c "$1" --arg d "$REPO" '{tool_name:"Bash",cwd:$d,tool_input:{command:$c}}' \
    | PATH="$FAKE_BIN:$PATH" "$GUARD" 2> "$TMP/guard.err"
}
expect() { # expect <期待する終了コード> <名前> <コマンド>
  local want="$1" name="$2" cmd="$3" got=0
  run_guard "$cmd" || got=$?
  [[ "$got" -eq "$want" ]] || fail "guard: $name（期待 $want、実際 $got）: $(cat "$TMP/guard.err")"
}

export FAKE_GH_SHA="$HEAD_SHA"
rm -rf "$IKARI_CHECK_RECORD_DIR"

expect 0 "マージでないコマンドは通す" "cargo test --workspace"
expect 0 "gh pr view は通す" "gh pr view 5"
expect 0 "gh pr mergeable という別の語は通す" "echo gh pr mergeable"
expect 2 "記録がなければ止める" "gh pr merge 5 --squash --delete-branch"
check_record_write "$REPO" 1 full
expect 2 "CHECK_EXIT=1 なら止める" "gh pr merge 5 --squash"
check_record_write "$REPO" 0 full
expect 0 "CHECK_EXIT=0 なら通す" "gh pr merge 5 --squash --delete-branch"
expect 0 "複合コマンドの中でも通す（0）" "cd /tmp && gh pr merge 5 --squash"
check_record_write "$REPO" 1 full
expect 2 "複合コマンドの中でも止める（1）" "cd /tmp && gh pr merge 5 --squash"
expect 2 "複合コマンド（; 区切り）も止める" "echo x; gh pr merge 5"
check_record_write "$REPO" 0 full

# PR の先頭コミットが、記録のあるコミットと違うときは止める（check.sh の後にコミットを足した場合）
FAKE_GH_SHA="$SHA_A" expect 2 "先頭コミットが違えば止める" "gh pr merge 5 --squash"
# gh が失敗したら安全側に止める
FAKE_GH_FAIL=1 expect 2 "gh pr view の失敗は止める" "gh pr merge 5 --squash"

# PR の指定が gh pr view に渡る
expect 0 "番号の指定" "gh pr merge 124 --squash --delete-branch"
[[ "$(cat "$FAKE_GH_ARGS")" == "pr view 124 --json headRefOid -q .headRefOid" ]] || fail "guard: 番号が gh pr view に渡らない: $(cat "$FAKE_GH_ARGS")"
expect 0 "指定なし（現在のブランチ）" "gh pr merge --squash"
[[ "$(cat "$FAKE_GH_ARGS")" == "pr view --json headRefOid -q .headRefOid" ]] || fail "guard: 指定なしの引数が違う: $(cat "$FAKE_GH_ARGS")"
expect 0 "値つきのフラグの値を PR と取り違えない" "gh pr merge --squash --body 'fix' 7"
[[ "$(cat "$FAKE_GH_ARGS")" == "pr view 7 "* ]] || fail "guard: --body の値を PR と取り違えた: $(cat "$FAKE_GH_ARGS")"
expect 0 "-R の指定" "gh pr merge 9 -R owner/repo --squash"
[[ "$(cat "$FAKE_GH_ARGS")" == *"-R owner/repo"* ]] || fail "guard: -R が gh pr view に渡らない: $(cat "$FAKE_GH_ARGS")"

# 回避: 環境変数（コマンドの中に書いても効かない）
rm -rf "$IKARI_CHECK_RECORD_DIR"
IKARI_ALLOW_UNCHECKED_MERGE=1 expect 0 "環境変数で回避できる" "gh pr merge 5 --squash"
expect 2 "コマンド内の環境変数では回避できない" "IKARI_ALLOW_UNCHECKED_MERGE=1 gh pr merge 5 --squash"

# 入力の異常: JSON でない、command がない
got=0; printf 'not json' | PATH="$FAKE_BIN:$PATH" "$GUARD" 2>/dev/null || got=$?
[[ "$got" -eq 0 ]] || fail "guard: JSON でない入力は通す（期待 0、実際 $got）"
got=0; printf '{"tool_name":"Bash","tool_input":{}}' | PATH="$FAKE_BIN:$PATH" "$GUARD" 2>/dev/null || got=$?
[[ "$got" -eq 0 ]] || fail "guard: command のない入力は通す（期待 0、実際 $got）"

# ---- fmt-rust.sh
run_fmt() { # run_fmt <file_path>
  jq -n --arg f "$1" '{tool_name:"Edit",tool_input:{file_path:$f}}' | PATH="$FAKE_BIN:$PATH" "$FMT" 2>/dev/null
}
mkdir -p "$TMP/ws/src"
touch "$TMP/ws/src/lib.rs" "$TMP/ws/README.md"
: > "$FAKE_CARGO_LOG"
run_fmt "$TMP/ws/README.md" || fail "fmt: .rs でないファイルで失敗した"
[[ ! -s "$FAKE_CARGO_LOG" ]] || fail "fmt: .rs でないファイルで cargo が呼ばれた"
run_fmt "$TMP/ws/src/missing.rs" || fail "fmt: 存在しないファイルで失敗した"
[[ ! -s "$FAKE_CARGO_LOG" ]] || fail "fmt: 存在しないファイルで cargo が呼ばれた"
run_fmt "$TMP/ws/src/lib.rs" || fail "fmt: .rs で失敗した"
[[ "$(cat "$FAKE_CARGO_LOG")" == *"/ws/src|fmt --all" ]] || fail "fmt: cargo fmt --all がファイルの場所で呼ばれない: $(cat "$FAKE_CARGO_LOG")"
FAKE_CARGO_FAIL=1 run_fmt "$TMP/ws/src/lib.rs" || fail "fmt: cargo が失敗しても作業を止めてはいけない"
got=0; printf 'not json' | PATH="$FAKE_BIN:$PATH" "$FMT" 2>/dev/null || got=$?
[[ "$got" -eq 0 ]] || fail "fmt: JSON でない入力で失敗した"

# ---- settings.json と実行権限
jq -e '.hooks.PreToolUse[0].hooks[0].command and .hooks.PostToolUse[0].hooks[0].command' .claude/settings.json >/dev/null \
  || fail ".claude/settings.json が壊れている"
for hook in "$GUARD" "$FMT"; do
  [[ -x "$hook" ]] || fail "$hook に実行権限がありません"
done

[[ "$failures" -eq 0 ]] || die "フックのテストが $failures 件失敗しました"
log "フックのテスト: 成功"
