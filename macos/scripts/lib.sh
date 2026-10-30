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
