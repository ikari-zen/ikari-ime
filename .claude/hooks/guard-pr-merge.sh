#!/bin/bash
# Claude Code の PreToolUse フック（matcher: Bash）。issue #107。
# `gh pr merge` の前に、マージする PR の先頭コミットで scripts/check.sh が CHECK_EXIT=0 で終わった記録があるか確かめ、なければ止める（終了コード 2）。
# 記録は scripts/check.sh が書く（scripts/check-record.sh）。記録は「そのコミット」のもの。push 後に変えたコミットは、もう一度 check.sh を流す。
#
# 失敗したときの回避（作業を止め続けないため）: Claude Code を起動する前のシェルで IKARI_ALLOW_UNCHECKED_MERGE=1 を設定する。
#   コマンドの中に書いても効かない（フックはコマンドの環境変数を見ない）。--no-verify の類ではなく、ユーザーが明示して起動する回避。
# 入力は標準入力の JSON（tool_input.command、cwd）。jq が要る。jq がなく、入力に gh pr merge らしき文字があるときは、安全側に止める。
set -uo pipefail

readonly ALLOW_ENV="IKARI_ALLOW_UNCHECKED_MERGE"
readonly MERGE_PATTERN='(^|[^[:alnum:]_.-])gh[[:space:]]+pr[[:space:]]+merge([[:space:]]|$)'
readonly BLOCK_EXIT=2

block() {
  printf 'gh pr merge を止めました: %s\n' "$1" >&2
  printf 'scripts/check.sh を PR の先頭コミットで、作業ツリーをきれいにして流し、CHECK_EXIT=0 を確かめてから、もう一度マージしてください。\n' >&2
  exit "$BLOCK_EXIT"
}

input="$(cat)"

if ! command -v jq >/dev/null 2>&1; then
  if printf '%s' "$input" | grep -Eq 'gh[[:space:]]+pr[[:space:]]+merge'; then
    [[ "${!ALLOW_ENV:-}" == "1" ]] && exit 0
    block "jq がないので、コマンドを確かめられません（brew install jq）"
  fi
  exit 0
fi

command_text="$(printf '%s' "$input" | jq -r '.tool_input.command // empty' 2>/dev/null)" || command_text=""
cwd="$(printf '%s' "$input" | jq -r '.cwd // empty' 2>/dev/null)" || cwd=""
[[ -n "$cwd" ]] || cwd="."

# gh pr merge を含まないコマンドは関係ない
[[ "$command_text" =~ $MERGE_PATTERN ]] || exit 0

if [[ "${!ALLOW_ENV:-}" == "1" ]]; then
  printf '%s=1 なので、check.sh の記録を確かめずに通します\n' "$ALLOW_ENV" >&2
  exit 0
fi

# gh pr merge の後ろの語から、PR の指定（番号、URL、直後のブランチ名）と -R/--repo を取り出す
rest="${command_text#*gh pr merge}"
rest="${rest%%[;&|]*}"
rest="${rest%%$'\n'*}"
selector=""
repo_args=()
first=1
skip_next=0
take_repo=0
# shellcheck disable=SC2206
words=($rest)
for word in "${words[@]+"${words[@]}"}"; do
  word="${word%\"}"; word="${word#\"}"; word="${word%\'}"; word="${word#\'}"
  if [[ "$take_repo" -eq 1 ]]; then repo_args=(-R "$word"); take_repo=0; first=0; continue; fi
  if [[ "$skip_next" -eq 1 ]]; then skip_next=0; first=0; continue; fi
  case "$word" in
    -R | --repo) take_repo=1 ;;
    --repo=*) repo_args=(-R "${word#--repo=}") ;;
    -b | --body | -t | --subject | -F | --body-file | -A | --author-email | --match-head-commit) skip_next=1 ;;
    -*) ;;
    *)
      if [[ -z "$selector" ]]; then
        if [[ "$word" =~ ^[0-9]+$ || "$word" =~ ^https?:// ]]; then
          selector="$word"
        elif [[ "$first" -eq 1 && "$word" =~ ^[A-Za-z0-9._/:@-]+$ ]]; then
          selector="$word"
        fi
      fi
      ;;
  esac
  first=0
done

command -v gh >/dev/null 2>&1 || block "gh がないので、PR の先頭コミットを調べられません"
view_args=(pr view)
[[ -n "$selector" ]] && view_args+=("$selector")
view_args+=(--json headRefOid -q .headRefOid)
head_sha="$(cd "$cwd" 2>/dev/null && gh "${view_args[@]}" "${repo_args[@]+"${repo_args[@]}"}" 2>/dev/null)" || head_sha=""
[[ "$head_sha" =~ ^[0-9a-f]{40}$ ]] || block "PR の先頭コミットを gh pr view で調べられませんでした（指定: ${selector:-現在のブランチ}）"

# shellcheck source=../../scripts/check-record.sh
source "$(dirname "${BASH_SOURCE[0]}")/../../scripts/check-record.sh"
record="$(check_record_read "$cwd" "$head_sha")" || block "コミット ${head_sha:0:12} の check.sh の記録がありません"
exit_line="$(printf '%s\n' "$record" | grep '^CHECK_EXIT=' | head -1)"
[[ "$exit_line" == "CHECK_EXIT=0" ]] || block "コミット ${head_sha:0:12} の check.sh は ${exit_line:-結果不明} でした"
exit 0
