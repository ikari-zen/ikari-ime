#!/bin/bash
# build.sh / install.sh / smoke.sh の共通部分（source して使う）。
# 識別子は docs/phase2-contract.md §1 に揃える。頭とチームID は、このファイルの 2 行にだけ書く（ADR 0036 §3）。
# 環境変数 IKARI_ID_PREFIX と TEAM_ID で上書きできる。Swift 側の macos/Sources/IkariXPC/Identity.swift は
# macos/scripts/gen-identity.sh がここから生成する（build.sh が呼ぶ。既定値との違いは scripts/check.sh が落とす）。

IKARI_SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IKARI_MACOS_DIR="$(cd "$IKARI_SCRIPTS_DIR/.." && pwd)"
IKARI_ROOT="$(cd "$IKARI_MACOS_DIR/.." && pwd)"

# 他の人が自分の値で使うときは、環境変数を与えるか、既定値の 2 行を書き換える
readonly IKARI_ID_PREFIX="${IKARI_ID_PREFIX:-io.github.ikari-zen}"
readonly TEAM_ID="${TEAM_ID:-L9Z8SK775X}"  # 証明書の subject.OU
[[ "$TEAM_ID" =~ ^[A-Z0-9]{10}$ ]] || { printf 'error: TEAM_ID は英大文字と数字の 10 文字です\n' >&2; exit 1; }
[[ "$IKARI_ID_PREFIX" =~ ^[A-Za-z0-9.-]+$ ]] || { printf 'error: IKARI_ID_PREFIX は英数字、-、. だけです: %s\n' "$IKARI_ID_PREFIX" >&2; exit 1; }

readonly BUNDLE_ID="$IKARI_ID_PREFIX.inputmethod.ikari"
readonly ENGINE_BUNDLE_ID="$BUNDLE_ID.engine"
readonly SMOKE_ID="$BUNDLE_ID.smoke"
readonly SETTINGS_ID="$BUNDLE_ID.settings"
readonly LM_HOST_ID="$BUNDLE_ID.lm-host"
readonly LM_CAGE_ID="$BUNDLE_ID.lm-cage"
readonly DATA_ID="$BUNDLE_ID.data"  # ikari-data（取得の部品。ADR 0079 §3.2）。署名の identifier と Info.plist の CFBundleIdentifier
readonly INPUT_SOURCE_JAPANESE="$BUNDLE_ID.Japanese"
readonly APP_NAME="ikari.app"
readonly SETTINGS_APP_NAME="IkariSettings.app"
readonly INSTALL_DIR="$HOME/Library/Input Methods"

log() { printf '==> %s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# 署名の方式とビルド種別。build.sh / install.sh / smoke.sh が同じ出力先を指すためのもの。
parse_common_flag() {
  case "$1" in
    --release) CONFIG=release ;;
    --debug) CONFIG=debug ;;
    *) return 1 ;;
  esac
}

validate_signing() {
  case "$SIGNING" in
    team | adhoc) ;;
    *) die "--signing は team か adhoc です: $SIGNING" ;;
  esac
}

# 出力先: target/ は .gitignore 済み。
bundle_out_dir() { printf '%s/target/macos/%s-%s' "$IKARI_ROOT" "$CONFIG" "$SIGNING"; }

# Cargo.toml の [workspace.package] の version
workspace_version() {
  sed -n '/^\[workspace\.package\]/,/^\[/{s/^version[[:space:]]*=[[:space:]]*"\(.*\)"[[:space:]]*$/\1/p;}' \
    "$IKARI_ROOT/Cargo.toml" | head -1
}

# plutil で Info.plist のキーを置き換える（値に引用符や括弧があってもそのまま入る）
plist_set_string() { plutil -replace "$2" -string "$3" "$1"; }

# テンプレート（Info.plist、InfoPlist.strings）の置き場所の印を埋める。印が残っていれば止まる（ADR 0036 §3.1）。
# 頭は [A-Za-z0-9.-] だけなので、sed の置き換えで壊れない
render_identity() {
  local file="$1"
  sed -i '' "s/@IKARI_ID_PREFIX@/$IKARI_ID_PREFIX/g" "$file"
  if grep -q '@IKARI_' "$file"; then die "$file に埋まっていない印が残っています"; fi
}

# ikari-data の entitlement が決定のとおりか（ADR 0079 §3.3、§3.7 の 4・5）。標準入力に plist（XML）を受け取る。
# 合っていれば何も出さず 0、外れていれば理由を 1 行出して 1 を返す（build.sh の verify_data と、
# macos/scripts/test-data-entitlements.sh が使う。検査そのものをテストで確かめられるよう、ここに置く）
readonly DATA_EXCEPTION_KEY='com.apple.security.temporary-exception.files.home-relative-path.read-write'
readonly DATA_EXCEPTION_FETCH='/Library/Application Support/ikari-ime/fetch/'
readonly DATA_EXCEPTION_INBOX='/Library/Application Support/ikari-ime/inbox/'
readonly DATA_ENTITLEMENT_KEYS=3
data_entitlements_error() {
  local ents xml keys key_exc="${DATA_EXCEPTION_KEY//./\\.}"
  ents="$(cat)"
  [[ -n "$ents" ]] || { echo "entitlement を読めません"; return 1; }
  xml="$(printf '%s' "$ents" | plutil -convert xml1 -o - - 2>/dev/null)" || { echo "entitlement を plist として読めません"; return 1; }
  keys="$(printf '%s\n' "$xml" | grep -c '<key>' || true)"
  [[ "$keys" -eq "$DATA_ENTITLEMENT_KEYS" ]] || { echo "entitlement が $DATA_ENTITLEMENT_KEYS つではありません（キーが $keys 個）"; return 1; }

  local key_sandbox key_client
  key_sandbox="$(printf '%s' "$xml" | plutil -extract 'com\.apple\.security\.app-sandbox' raw - 2>/dev/null || true)"
  key_client="$(printf '%s' "$xml" | plutil -extract 'com\.apple\.security\.network\.client' raw - 2>/dev/null || true)"
  [[ "$key_sandbox" == true ]] || { echo "com.apple.security.app-sandbox が true ではありません: '$key_sandbox'"; return 1; }
  [[ "$key_client" == true ]] || { echo "com.apple.security.network.client が true ではありません: '$key_client'"; return 1; }

  # 一時例外の配列は、fetch/ と inbox/ の 2 つの文字列に厳密に一致する（型も数も並びも）
  local type count first second
  type="$(printf '%s' "$xml" | plutil -type "$key_exc" - 2>/dev/null || true)"
  [[ "$type" == array ]] || { echo "一時例外が配列ではありません: '$type'"; return 1; }
  count="$(printf '%s' "$xml" | plutil -extract "$key_exc" raw - 2>/dev/null || true)"
  [[ "$count" == 2 ]] || { echo "一時例外が 2 つではありません: '$count'"; return 1; }
  first="$(printf '%s' "$xml" | plutil -extract "$key_exc.0" raw - 2>/dev/null || true)"
  second="$(printf '%s' "$xml" | plutil -extract "$key_exc.1" raw - 2>/dev/null || true)"
  [[ "$first" == "$DATA_EXCEPTION_FETCH" ]] || { echo "一時例外の 1 つめが fetch/ ではありません: '$first'"; return 1; }
  [[ "$second" == "$DATA_EXCEPTION_INBOX" ]] || { echo "一時例外の 2 つめが inbox/ ではありません: '$second'"; return 1; }
}
