#!/bin/bash
# スパイク（捨てるコード。製品では使わない）。#448 の FC-1。手順は README.md。
#
#   macos/tools/cage-peer-check-spike/build.sh [--identity <署名の名前>] [--out <置き場所>]
#
# 既定は ad-hoc の署名（--identity -）。置き場所の既定は $TMPDIR/ikari-cage-spike（リポジトリの外）。
# 作るもの: CageSpike.app（host。Sandbox なし）と、その中の XPC サービス io.github.ikari-zen.spike.cage.xpc（cage）。
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
identity="-"
out="${TMPDIR:-/tmp}/ikari-cage-spike"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --identity) identity="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    *) echo "build.sh: 知らない引数: $1" >&2; exit 2 ;;
  esac
done

host_id="io.github.ikari-zen.spike.host"
cage_id="io.github.ikari-zen.spike.cage"
app="$out/CageSpike.app"
xpc="$app/Contents/XPCServices/$cage_id.xpc"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$xpc/Contents/MacOS"

clang -Wall -Wextra -O1 -o "$xpc/Contents/MacOS/cage" "$here/cage.c"
clang -Wall -Wextra -O1 -o "$app/Contents/MacOS/host" "$here/host.c" \
  -framework Security -framework CoreFoundation

cat > "$xpc/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>$cage_id</string>
  <key>CFBundleExecutable</key><string>cage</string>
  <key>CFBundleName</key><string>cage</string>
  <key>CFBundlePackageType</key><string>XPC!</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>XPCService</key><dict><key>ServiceType</key><string>Application</string></dict>
</dict></plist>
PLIST

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>$host_id</string>
  <key>CFBundleExecutable</key><string>host</string>
  <key>CFBundleName</key><string>CageSpike</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
</dict></plist>
PLIST

# 内から外へ署名する
codesign --force --sign "$identity" --identifier "$cage_id" "$xpc"
codesign --force --sign "$identity" --identifier "$host_id" "$app"
codesign --verify --deep --strict "$app"
echo "built: $app"
