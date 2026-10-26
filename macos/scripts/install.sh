#!/bin/bash
# ビルドした ikari.app を ~/Library/Input Methods に入れる（ADR 0004 §9.2）。
#
#   macos/scripts/install.sh [--signing team|adhoc] [--release|--debug] [--build] [--register] [--enable]
#
#   --build     先に build.sh を同じ引数で流す
#   --register  入力ソースとしてシステムに登録する（候補に出るだけ）
#   --enable    入力ソースを有効にする（システム設定の入力ソースに追加される。明示したときだけ）
#
# 既定では入力ソースの設定を変えない。初回の追加は macos/README.md の手順で行う。
# 今 ikari を使っているときだけ、別の入力ソースを経由して選び直し、新しい版を読み込ませる。
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

SIGNING=team
CONFIG=release
DO_BUILD=0
DO_REGISTER=0
DO_ENABLE=0
BUILD_ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --signing) SIGNING="${2:-}"; BUILD_ARGS+=(--signing "$SIGNING"); shift ;;
    --signing=*) SIGNING="${1#--signing=}"; BUILD_ARGS+=("$1") ;;
    --build) DO_BUILD=1 ;;
    --register) DO_REGISTER=1 ;;
    --enable) DO_ENABLE=1 ;;
    -h | --help) sed -n '2,12p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) parse_common_flag "$1" || die "不明な引数: $1"; BUILD_ARGS+=("$1") ;;
  esac
  shift
done
validate_signing

if [[ "$DO_BUILD" -eq 1 ]]; then
  "$IKARI_SCRIPTS_DIR/build.sh" ${BUILD_ARGS[@]+"${BUILD_ARGS[@]}"}
fi

SRC="$(bundle_out_dir)/$APP_NAME"
DEST="$INSTALL_DIR/$APP_NAME"
[[ -d "$SRC" ]] || die "$SRC がありません。先に build.sh を流すか --build を付けてください"
codesign --verify --deep --strict "$SRC" || die "署名の検証に失敗しました: $SRC"

TOOL_DIR="$IKARI_ROOT/target/macos/tools"
mkdir -p "$TOOL_DIR" "$INSTALL_DIR"
TOOL="$TOOL_DIR/select-input-source"
# 識別子は lib.sh の値を渡す（ツールに直書きしない。ADR 0036 §3.1）
export IKARI_PRODUCT_ID="$BUNDLE_ID"
if [[ ! -x "$TOOL" || "$IKARI_MACOS_DIR/tools/select-input-source.swift" -nt "$TOOL" ]]; then
  log "select-input-source をコンパイル"
  swiftc -O -o "$TOOL" "$IKARI_MACOS_DIR/tools/select-input-source.swift"
fi

# 動いている古い版を止めてから入れ替える（エンジンは終了時に学習を保存する）
stop_processes() { killall ikari IkariEngine IkariSettings ikari-smoke 2>/dev/null || true; }
log "実行中のプロセスを止める"
stop_processes
sleep 0.3

log "配置: $DEST"
rm -rf "${DEST:?}"
ditto "$SRC" "$DEST"
codesign --verify --deep --strict "$DEST" || die "配置後の署名の検証に失敗しました"

stop_processes

[[ "$DO_REGISTER" -eq 0 ]] || "$TOOL" register
[[ "$DO_ENABLE" -eq 0 ]] || "$TOOL" enable
"$TOOL" reselect
# 英数キーの行き先は履歴で決まる。インストールしたてでも ikari (English) に行くよう、英語のモードを一度選んで戻す（#303）
"$TOOL" prime-english
"$TOOL" status

log "完了。初回は macos/README.md の「入力ソースの追加」を行ってください"
