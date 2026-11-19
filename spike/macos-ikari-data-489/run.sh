#!/bin/bash
# スパイク #489。捨てるもの。使い方: run.sh <作業ディレクトリ> <adhoc|team>
# 本物の ikari には触れない。~/Library/Application Support/ikari-ime-spike/ と、
# ~/Library/Containers/<spike の識別子> だけを使い、終わりに消す。
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
W="$1"; MODE="$2"
ID=io.github.ikari-zen.inputmethod.ikari.data.spike${3:-}   # 3 番目の引数: 識別子の後ろに足す印（別のコンテナにするため）
ROOT="$HOME/Library/Application Support/ikari-ime-spike"
CONT="$HOME/Library/Containers/$ID"
case "$MODE" in
  adhoc) SIGN=- ;;
  team) SIGN="$(security find-identity -v -p codesigning | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "Apple Development.*/\1/p' | head -1)" ;;
esac
[ -n "$SIGN" ] || { echo "署名の ID が無い"; exit 1; }
mkdir -p "$W/b"
sed "s|io.github.ikari-zen.inputmethod.ikari.data.spike|$ID|" "$HERE/Info.plist" > "$W/Info.plist"
cc -o "$W/b/data-spike" "$HERE/data-spike.c" -Wl,-sectcreate,__TEXT,__info_plist,"$W/Info.plist" || exit 1
cc -o "$W/b/data-spike-noplist" "$HERE/data-spike.c" || exit 1
cc -o "$W/b/spawner" "$HERE/spawner.c" || exit 1
SIGNARGS=(--force --options runtime --timestamp=none -s "$SIGN")
reset() { rm -rf "$ROOT" "$HOME/spike-outside"; mkdir -p "$ROOT"; }
cleanup() { rm -rf "$ROOT" "$HOME/spike-outside" "$CONT"; }
trap cleanup EXIT

echo "##### 署名 = $MODE"
cp "$W/b/data-spike" "$W/data-spike-plain"
codesign "${SIGNARGS[@]}" -i "$ID" --entitlements "$HERE/data.entitlements" "$W/data-spike-plain" 2>&1 | tail -1
codesign -dvv --entitlements - "$W/data-spike-plain" 2>&1 | grep -E "Identifier|TeamIdentifier|flags|Info.plist|Authority"

echo "=== (a)(b)(c) Helpers 以外の場所（作業ディレクトリ）から Terminal で起動。最初から ($ROOT だけある)"
reset; "$W/data-spike-plain" probe; echo "exit=$?"
echo "--- 作られたもの"; find "$ROOT" | sed "s|$HOME|~|"; ls -d "$HOME/spike-outside" 2>&1
echo "--- xattr"; xattr -lr "$ROOT" 2>&1 | head; echo "(end xattr)"
echo "--- 2 回目（fetch/ inbox/ が既にある。mkdir は EEXIST のはず）"
"$W/data-spike-plain" probe | sed -n '/例外の中/,/例外の外/p'
echo "--- コンテナ"; ls "$CONT" 2>&1 | head -3

echo "=== (a) 根が無い状態（データの場所そのものが無い）"
rm -rf "$ROOT"; "$W/data-spike-plain" probe | sed -n '1,8p'

echo "=== 通信"
reset; "$W/data-spike-plain" net

echo "=== (a 代わり) 親が先に fetch/ inbox/ を作った状態"
reset; mkdir "$ROOT/fetch" "$ROOT/inbox"; "$W/data-spike-plain" probe | sed -n '/例外の中/,/HOME 直下/p'

echo "=== (d) Info.plist 無し（0076 §16 の再確認）"
cp "$W/b/data-spike-noplist" "$W/data-spike-nop"
codesign "${SIGNARGS[@]}" -i "$ID" --entitlements "$HERE/data.entitlements" "$W/data-spike-nop" 2>&1 | tail -1
reset; "$W/data-spike-nop" probe 2>&1 | head -3; echo "exit=${PIPESTATUS[0]}"

echo "=== (d) Contents/Helpers に置いた束"
APP="$W/Spike.app"; rm -rf "$APP"; mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Helpers"
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>io.github.ikari-zen.spike.app</string>
<key>CFBundleExecutable</key><string>spawner</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>0</string>
</dict></plist>
EOF
cp "$W/b/data-spike" "$APP/Contents/Helpers/data-spike"
cp "$W/b/spawner" "$APP/Contents/MacOS/spawner"
codesign "${SIGNARGS[@]}" -i "$ID" --entitlements "$HERE/data.entitlements" "$APP/Contents/Helpers/data-spike" 2>&1 | tail -1
codesign "${SIGNARGS[@]}" -i io.github.ikari-zen.spike.spawner "$APP/Contents/MacOS/spawner" 2>&1 | tail -1
codesign "${SIGNARGS[@]}" "$APP" 2>&1 | tail -1
codesign --verify --deep --strict "$APP" && echo "束の検証 OK"
ls "$APP/Contents/_CodeSignature/" ; grep -c "Helpers/data-spike" "$APP/Contents/_CodeSignature/CodeResources"
echo "--- Terminal から Helpers の exe を直接"
reset; "$APP/Contents/Helpers/data-spike" probe | sed -n '1,3p;/例外の中/,/HOME 直下/p'
echo "--- 束の中の spawner（Sandbox なし）から posix_spawn"
reset; "$APP/Contents/MacOS/spawner" "$APP/Contents/Helpers/data-spike" probe | sed -n '1,4p;/例外の中/,$p'
echo "--- spawner から通信"
reset; "$APP/Contents/MacOS/spawner" "$APP/Contents/Helpers/data-spike" net
echo "--- spawner から、環境変数を空にして起こす（HOME なし）"
reset; env -i "$APP/Contents/MacOS/spawner" "$APP/Contents/Helpers/data-spike" probe | sed -n '1,3p;/例外の中/,/例外の外/p'
echo "##### 終わり $MODE"
