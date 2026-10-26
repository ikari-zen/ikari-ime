#!/bin/bash
# バンドルの中の ikari-smoke を流す（XPC 経由の E2E。ADR 0004 §10.1）。
#
#   macos/scripts/smoke.sh [--signing team|adhoc] [--release|--debug] [--installed] [ikari-smoke の引数...]
#
#   --installed  ~/Library/Input Methods の ikari.app を使う（既定は build.sh の出力）
#   ikari を使っている間は IME のエンジンが engine.lock を持つので、ikari-smoke は終了コード 2 で止まる（ABC などに切り替える）
#   例: smoke.sh --signing adhoc --recovery-limit-ms 3000
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

SIGNING=team
CONFIG=release
INSTALLED=0
PASS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --signing) SIGNING="${2:-}"; shift ;;
    --signing=*) SIGNING="${1#--signing=}" ;;
    --installed) INSTALLED=1 ;;
    *) parse_common_flag "$1" || PASS+=("$1") ;;
  esac
  shift
done
validate_signing

if [[ "$INSTALLED" -eq 1 ]]; then APP="$INSTALL_DIR/$APP_NAME"; else APP="$(bundle_out_dir)/$APP_NAME"; fi
[[ -x "$APP/Contents/MacOS/ikari-smoke" ]] || die "$APP/Contents/MacOS/ikari-smoke がありません"
exec "$APP/Contents/MacOS/ikari-smoke" ${PASS[@]+"${PASS[@]}"}
