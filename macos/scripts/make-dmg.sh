#!/bin/bash
# 署名済みの ikari.app から dmg を作り、検証する（ADR 0080 §5。#583）。build.sh --dmg が呼ぶ。単体でも使える。
#
#   macos/scripts/make-dmg.sh --app <ikari.app> --out <出力の .dmg> [--format ULMO|ULFO|UDZO] [--volname <名前>]
#
# 中身は ikari.app だけ（ADR 0080 §5.1 の「~/Library/Input Methods へのショートカット」と「案内」は、
# 実機の確認（C3、#241）のあとに足す。今は入れない）。スクリプトや、利用者の操作なしに動く物は入れない。
# 署名は ikari.app の物をそのまま使う（ad-hoc でも team でも）。dmg 自体には署名しない。
# 作ったあと、hdiutil verify と、読み取り専用でのマウント（確認して、すぐアンマウント）で、中の ikari.app の署名を検証する。
# 圧縮の形（--format）の既定は ULMO（lzma）。ULFO（lzfse）との比較は PR の説明の測定を参照（ADR 0080 §5.1 は測って決めるとしている）。
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

readonly DEFAULT_FORMAT=ULMO
readonly DEFAULT_VOLNAME=ikari
APP=""
OUT_DMG=""
FORMAT="$DEFAULT_FORMAT"
VOLNAME="$DEFAULT_VOLNAME"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --app) [[ $# -ge 2 ]] || { echo "error: --app に値がありません" >&2; exit 64; }; APP="$2"; shift ;;
    --out) [[ $# -ge 2 ]] || { echo "error: --out に値がありません" >&2; exit 64; }; OUT_DMG="$2"; shift ;;
    --format) FORMAT="${2:-}"; shift ;;
    --volname) VOLNAME="${2:-}"; shift ;;
    -h | --help) sed -n '2,11p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) die "不明な引数: $1" ;;
  esac
  shift
done
[[ -n "$APP" && -n "$OUT_DMG" ]] || die "--app と --out が要ります"
case "$FORMAT" in
  ULMO | ULFO | UDZO) ;;
  *) die "--format は ULMO か ULFO か UDZO です: $FORMAT" ;;
esac
[[ "$(basename "$APP")" == "$APP_NAME" && -d "$APP" ]] || die "--app は $APP_NAME のディレクトリです: $APP"
[[ "$OUT_DMG" == *.dmg ]] || die "--out は .dmg で終わる名前です: $OUT_DMG"
# 配る物に Smoke（開発用の確認の道具）を入れない（ADR 0069 §6）。単体で呼ばれても守る
smoke_found="$(find "$APP" -name 'ikari-smoke*' -print -quit)" || die "$APP を調べられません"
[[ -z "$smoke_found" ]] || die "$APP に ikari-smoke があります（--distribution でビルドした束だけを dmg にする）: $smoke_found"
codesign --verify --deep --strict "$APP" || die "$APP の署名を検証できません（署名してから dmg にする）"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/ikari-dmg.XXXXXX")"
MOUNT="$WORK/mnt"
mounted=0
cleanup() {
  if [[ "$mounted" -eq 1 ]]; then
    hdiutil detach "$MOUNT" -quiet >/dev/null 2>&1 || hdiutil detach "$MOUNT" -force -quiet >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

# dmg に入れる物は ikari.app だけ（ビルドの出力先にある別の物を混ぜない）。ditto は署名と拡張属性を保つ
STAGE="$WORK/stage"
mkdir -p "$STAGE" "$MOUNT" "$(dirname "$OUT_DMG")"
log "dmg の中身を置く: $APP"
ditto "$APP" "$STAGE/$APP_NAME"

log "hdiutil create ($FORMAT): $OUT_DMG"
rm -f "$OUT_DMG"
hdiutil create -quiet -srcfolder "$STAGE" -volname "$VOLNAME" -fs HFS+ -format "$FORMAT" -ov "$OUT_DMG" >/dev/null
[[ -s "$OUT_DMG" ]] || die "$OUT_DMG ができていません"

log "hdiutil verify"
hdiutil verify -quiet "$OUT_DMG" >/dev/null || die "hdiutil verify が失敗しました: $OUT_DMG"

# 読み取り専用でマウントして、中身が ikari.app だけで、署名が通ることを見る（すぐアンマウントする）
log "マウントして確認"
hdiutil attach -quiet -readonly -nobrowse -noautoopen -mountpoint "$MOUNT" "$OUT_DMG" >/dev/null || die "マウントできません: $OUT_DMG"
mounted=1
inside="$(ls -A "$MOUNT" | grep -v '^\.\(fseventsd\|Spotlight-V100\|Trashes\|DS_Store\)$' || true)"
[[ "$inside" == "$APP_NAME" ]] || die "dmg の中身が $APP_NAME だけではありません: $(printf '%s' "$inside" | tr '\n' ' ')"
codesign --verify --deep --strict "$MOUNT/$APP_NAME" || die "dmg の中の $APP_NAME の署名を検証できません"
hdiutil detach "$MOUNT" -quiet >/dev/null
mounted=0

log "完了: $OUT_DMG ($(du -h "$OUT_DMG" | cut -f1))"
