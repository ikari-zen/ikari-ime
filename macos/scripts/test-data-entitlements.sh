#!/bin/bash
# ikari-data の entitlement の検査（lib.sh の data_entitlements_error。build.sh の verify_data が使う）のテスト（#489、ADR 0079 §3.7）。
# テンプレートは通り、決定から外れた形（例外を広げる、末尾の / を落とす、並びを替える、キーを足す・減らす）は
# すべて落ちることを確かめる。署名もビルドもしない（plist を直接与える）。plutil が要るので macOS 以外では飛ばす。
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

if ! command -v plutil >/dev/null 2>&1; then
  echo "skip: plutil がありません（macOS だけで流す）"
  exit 0
fi

TEMPLATE="$IKARI_SCRIPTS_DIR/templates/ikari-data.entitlements"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
readonly TEMPLATE TMP
failures=0
fail() { printf 'FAIL: %s\n' "$*" >&2; failures=$((failures + 1)); }
key_exc="${DATA_EXCEPTION_KEY//./\\.}"

# 合格するはず
expect_ok() {
  local name="$1" file="$2" problem
  if problem="$(data_entitlements_error <"$file")"; then
    echo "ok: $name"
  else
    fail "$name が落ちた: $problem"
  fi
}

# 落ちるはず。理由の文に、期待する言葉が入っていること
expect_ng() {
  local name="$1" want="$2" file="$3" problem
  if problem="$(data_entitlements_error <"$file")"; then
    fail "$name が通ってしまった"
  elif [[ "$problem" != *"$want"* ]]; then
    fail "$name の理由が想定と違う（'$want' を含まない）: $problem"
  else
    echo "ok: $name -> $problem"
  fi
}

# テンプレートの写しに plutil の操作を 1 つかけたものを作る（配列の要素の -replace は挿入になるので、配列ごと -json で替える）
mutant() {
  local name="$1"
  shift
  cp "$TEMPLATE" "$TMP/$name.plist"
  plutil "$@" "$TMP/$name.plist"
  printf '%s' "$TMP/$name.plist"
}

expect_ok "テンプレート" "$TEMPLATE"

# 例外を広げる・ずらす
expect_ng "データの場所の全体（エンジンと同じ例外）" "1 つめが fetch/" \
  "$(mutant wide -replace "$key_exc" -json '["/Library/Application Support/ikari-ime/","/Library/Application Support/ikari-ime/inbox/"]')"
expect_ng "全体だけの 1 つ" "2 つではありません" \
  "$(mutant wide-only -replace "$key_exc" -json '["/Library/Application Support/ikari-ime/"]')"
expect_ng "3 つめを足す" "2 つではありません" \
  "$(mutant third -insert "$key_exc.2" -string '/Library/Application Support/ikari-ime/models/')"
expect_ng "末尾の / が無い" "1 つめが fetch/" \
  "$(mutant no-slash -replace "$key_exc" -json "[\"${DATA_EXCEPTION_FETCH%/}\",\"$DATA_EXCEPTION_INBOX\"]")"
expect_ng "並びが逆" "1 つめが fetch/" \
  "$(mutant reversed -replace "$key_exc" -json "[\"$DATA_EXCEPTION_INBOX\",\"$DATA_EXCEPTION_FETCH\"]")"
expect_ng "2 つめが別の場所" "2 つめが inbox/" \
  "$(mutant other -replace "$key_exc" -json "[\"$DATA_EXCEPTION_FETCH\",\"/Library/Application Support/ikari-ime/user/\"]")"
expect_ng "例外が配列でない" "配列ではありません" \
  "$(mutant not-array -replace "$key_exc" -string "$DATA_EXCEPTION_FETCH")"

# キーを足す・減らす・替える
expect_ng "network.server を足す" "3 つではありません" \
  "$(mutant server -insert 'com\.apple\.security\.network\.server' -bool true)"
expect_ng "読み取りだけの例外を足す" "3 つではありません" \
  "$(mutant read-only -insert 'com\.apple\.security\.temporary-exception\.files\.home-relative-path\.read-only' -json '["/Library/Application Support/ikari-ime/"]')"
expect_ng "network.client が無い" "3 つではありません" \
  "$(mutant no-client -remove 'com\.apple\.security\.network\.client')"
swap="$(mutant swap-server -remove 'com\.apple\.security\.network\.client')"
plutil -insert 'com\.apple\.security\.network\.server' -bool true "$swap"
expect_ng "network.client を server に替える" "network.client が true ではありません" "$swap"
expect_ng "app-sandbox が false" "app-sandbox が true ではありません" \
  "$(mutant sandbox-false -replace 'com\.apple\.security\.app-sandbox' -bool false)"

# 空・壊れた入力
: >"$TMP/empty.plist"
expect_ng "空の入力" "読めません" "$TMP/empty.plist"
printf 'not a plist' >"$TMP/broken.plist"
expect_ng "plist でない入力" "plist として読めません" "$TMP/broken.plist"

if [[ "$failures" -gt 0 ]]; then
  printf '%d 件失敗\n' "$failures" >&2
  exit 1
fi
echo "test-data-entitlements: すべて通った"
