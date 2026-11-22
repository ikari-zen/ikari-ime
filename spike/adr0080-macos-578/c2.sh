#!/bin/bash
# スパイク #578（ADR 0080 C2）。捨てるもの。使い方: c2.sh <作業dir> <adhoc|team> <data|nodata>
# build.sh が作った ikari.app（target/macos/release-team）を写し、IkariEngine.xpc/Contents/Resources/data/ に
# 約 750MB を置いて（data のとき）、build.sh の sign_bundle と同じ順・同じ引数で署名し直し、時間を測る。
# そのあと codesign --verify --deep --strict の時間（データが冷えた状態と、温まった状態）。束は起動しない。
# install しない。本物の ikari の束（~/Library/Input Methods）には触れない。
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
W="$1"; MODE="$2"; KIND="$3"
SRC_APP="${SRC_APP:-$REPO/target/macos/release-team/ikari.app}"
DICT_SRC="${DICT_SRC:-$(git -C "$REPO" worktree list --porcelain | sed -n '1s/^worktree //p')/data/build/dict}"
MODEL_SRC="${MODEL_SRC:-$HOME/Library/Application Support/ikari-ime/models/zenz-v3.2-small-Q5_K_M.gguf}"
T="$REPO/macos/scripts/templates"
case "$MODE" in
  adhoc) SIGN=- ;;
  team) SIGN="$(security find-identity -v -p codesigning | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "Apple Development.*/\1/p' | head -1)" ;;
  *) echo "mode は adhoc か team"; exit 2 ;;
esac
[ -n "$SIGN" ] || { echo "署名の ID が無い"; exit 1; }
[ -d "$SRC_APP" ] || { echo "$SRC_APP が無い（macos/scripts/build.sh --signing team --release）"; exit 1; }
mkdir -p "$W"
APP="$W/c2-$MODE-$KIND/ikari.app"; XPC="$APP/Contents/XPCServices/IkariEngine.xpc"; CAGE="$APP/Contents/XPCServices/IkariLMCage.xpc"
D="$XPC/Contents/Resources/data"
rm -rf "$W/c2-$MODE-$KIND"; mkdir -p "$W/c2-$MODE-$KIND"; cp -R "$SRC_APP" "$APP"
refresh_data() {
  rm -rf "$D"; mkdir -p "$D/dict" "$D/models"
  cp -c "$DICT_SRC"/{system.dict,grammar.bin,vectors.bin,postal.tsv,emoji.tsv,BUILD_INFO} "$D/dict/"
  cp -c "$MODEL_SRC" "$D/models/"
}
[ "$KIND" = data ] && refresh_data
echo "##### C2 $MODE $KIND: 束の大きさ $(du -sh "$APP" | cut -f1)（data: $(du -sh "$D" 2>/dev/null | cut -f1)）"

clock_ms() { perl -MTime::HiRes=time -e 'printf "%d", time*1000'; }
idof() { codesign -dv "$1" 2>&1 | sed -n 's/^Identifier=//p'; }
common=(--force --options runtime --timestamp=none -s "$SIGN")
step() { # 表示名 codesign の引数...
  local name="$1" s; shift; s=$(clock_ms)
  codesign "${common[@]}" "$@" 2>/dev/null || { echo "署名に失敗: $name"; exit 1; }
  printf '  %-28s %6dms\n' "$name" $(( $(clock_ms) - s ))
}
# 識別子は写した元の署名から取る（build.sh の lib.sh と同じ値）
SMOKE_ID="$(idof "$APP/Contents/MacOS/ikari-smoke")"; DATA_ID="$(idof "$APP/Contents/Helpers/ikari-data")"
LMH_ID="$(idof "$CAGE/Contents/MacOS/ikari-lm-host")"; CAGE_ID="$(idof "$CAGE")"; SET_ID="$(idof "$APP/Contents/Helpers/IkariSettings.app")"
echo "--- 署名（build.sh の sign_bundle の順。--force で付け直す）"
total=$(clock_ms)
step ikari-smoke -i "$SMOKE_ID" "$APP/Contents/MacOS/ikari-smoke"
step ikari-data -i "$DATA_ID" --entitlements "$T/ikari-data.entitlements" "$APP/Contents/Helpers/ikari-data"
step ikari-lm-host -i "$LMH_ID" "$CAGE/Contents/MacOS/ikari-lm-host"
step IkariLMCage.xpc -i "$CAGE_ID" --entitlements "$T/lm-cage.entitlements" "$CAGE"
step IkariEngine.xpc --entitlements "$T/engine.entitlements" "$XPC"
step IkariSettings.app -i "$SET_ID" --entitlements "$T/settings.entitlements" "$APP/Contents/Helpers/IkariSettings.app"
step ikari.app "$APP"
echo "  合計                         $(( $(clock_ms) - total ))ms"
codesign -dv "$APP" 2>&1 | grep -E '^(Signature|TeamIdentifier)=' | sed 's/^/  /'
[ "$KIND" = data ] && { printf '  CodeResources に載った data のファイル: '; grep -c 'Resources/data/' "$XPC/Contents/_CodeSignature/CodeResources"; }

verify() { local s rc; s=$(clock_ms); codesign --verify --deep --strict "$APP" 2>&1 | tail -2; rc=${PIPESTATUS[0]}; echo "  verify --deep --strict ($1): exit=$rc $(( $(clock_ms) - s ))ms"; }
echo "--- 検証"
verify "温まった状態（署名の直後）"
verify "温まった状態 2 回目"
if [ "$KIND" = data ]; then
  refresh_data; verify "冷えた状態（clone で置き直した直後）"
  refresh_data; verify "冷えた状態 2 回目"
  echo "--- 改ざんの検出（vectors.bin の 1 バイトを変える）"
  printf 'X' | dd of="$D/dict/vectors.bin" bs=1 seek=1000 conv=notrunc 2>/dev/null
  verify "1 バイト変えたあと（失敗するはず）"
fi
rm -rf "$W/c2-$MODE-$KIND"
echo "##### 終わり（写した束は消した）"
