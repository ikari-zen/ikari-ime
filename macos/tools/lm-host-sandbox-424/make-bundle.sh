#!/bin/bash
# スパイク #424（捨てるコード。マージしない）。製品の檻（ADR 0077: 檻役の entitlement は app-sandbox だけ。子はそれを
# 引き継ぐ）と同じ Sandbox を、評価ツールの子で再現するための束を作る。ikari-lm-host の写しに app-sandbox だけを付け、
# 試験用の識別子の束に入れて ad-hoc で署名する（Sandbox なしの親から起動すると、自分の App Sandbox に入る。
# ADR 0072 §13.2）。モデルはパスでは読めないので、run-product.sh が fd 3 で渡す（製品と同じ）。
#   make-bundle.sh <ikari-lm-host>   →  $TMPDIR/ikari-lm-host-424/LMHost424.app
set -euo pipefail
src="${1:?ikari-lm-host の場所}"
out="${TMPDIR:-/tmp}/ikari-lm-host-424"
id="io.github.ikari-zen.spike.lmhost424"
app="$out/LMHost424.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$src" "$app/Contents/MacOS/ikari-lm-host"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>$id</string>
  <key>CFBundleExecutable</key><string>ikari-lm-host</string>
  <key>CFBundleName</key><string>LMHost424</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
</dict></plist>
PLIST
cat > "$out/ent.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>com.apple.security.app-sandbox</key><true/>
</dict></plist>
PLIST
codesign --force --sign - -o runtime --identifier "$id" --entitlements "$out/ent.plist" "$app"
codesign --verify --strict "$app"
codesign -d --entitlements - "$app" 2>/dev/null | grep -c app-sandbox >/dev/null
echo "built: $app"
