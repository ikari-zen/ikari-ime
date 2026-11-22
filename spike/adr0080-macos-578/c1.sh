#!/bin/bash
# スパイク #578（ADR 0080 C1）。捨てるもの。使い方: c1.sh <作業dir（ホームの下）> <adhoc|team>
# 試験用の束 Spike578-<mode>.app を作り、IkariEngine と同じ形（XPC の束、App Sandbox、network なし）の
# サービスに約 750MB のデータを持たせて、Sandbox の中から mmap と Engine::open ができるかと時間を測る。
# 本物の ikari、~/Library/Application Support/ikari-ime/（モデルを読んで写すだけ）、入力ソースには触れない。
# 識別子は署名の方式ごとに分ける（#489 の分かったこと 7: 同じ識別子で方式を替えるとコンテナで止まる）。
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
W="$1"; MODE="$2"; TAG="${3:-}"   # 3 番目: 識別子に足す印（新しいコンテナで最初の起動を見るため）
DICT_SRC="${DICT_SRC:-$(git -C "$REPO" worktree list --porcelain | sed -n '1s/^worktree //p')/data/build/dict}"
MODEL_SRC="${MODEL_SRC:-$HOME/Library/Application Support/ikari-ime/models/zenz-v3.2-small-Q5_K_M.gguf}"
PFX="io.github.ikari-zen.spike578.$MODE$TAG"
SVC="$PFX.probe"; ENG="$PFX.engineprobe"; HOST="$PFX.host"
case "$MODE" in
  adhoc) SIGN=- ;;
  team) SIGN="$(security find-identity -v -p codesigning | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "Apple Development.*/\1/p' | head -1)" ;;
  *) echo "mode は adhoc か team"; exit 2 ;;
esac
[ -n "$SIGN" ] || { echo "署名の ID が無い"; exit 1; }
[ -f "$DICT_SRC/system.dict" ] && [ -f "$MODEL_SRC" ] || { echo "実データが無い: $DICT_SRC / $MODEL_SRC"; exit 1; }
mkdir -p "$W/b"
A="$W/Spike578-$MODE$TAG.app"; X="$A/Contents/XPCServices/Probe.xpc"; D="$X/Contents/Resources/data"

plist() { # id exe type [xpc]
  cat <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>$1</string>
<key>CFBundleExecutable</key><string>$2</string>
<key>CFBundlePackageType</key><string>$3</string>
<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
<key>CFBundleVersion</key><string>0</string>
${4:+<key>XPCService</key><dict><key>ServiceType</key><string>Application</string></dict>}
</dict></plist>
EOF
}

echo "##### ビルド ($MODE)"
plist "$ENG" engine-probe APPL > "$W/b/engine-probe.Info.plist"
(cd "$REPO" && cargo rustc --release -q -p ikari-engine --example spike578-engine-probe -- \
  -C "link-arg=-Wl,-sectcreate,__TEXT,__info_plist,$W/b/engine-probe.Info.plist") || exit 1
cc -O2 -o "$W/b/xpc-probe" "$HERE/xpc-probe.c" || exit 1
cc -O2 -o "$W/b/xpc-host" "$HERE/xpc-host.c" || exit 1

rm -rf "$A"; mkdir -p "$A/Contents/MacOS" "$X/Contents/MacOS" "$D/dict" "$D/models"
plist "$HOST" xpc-host APPL > "$A/Contents/Info.plist"
plist "$SVC" xpc-probe 'XPC!' yes > "$X/Contents/Info.plist"
cp "$W/b/xpc-host" "$A/Contents/MacOS/"
cp "$W/b/xpc-probe" "$X/Contents/MacOS/"
cp "$REPO/target/release/examples/spike578-engine-probe" "$X/Contents/MacOS/engine-probe"
refresh_data() { # 同じ中身を clone で置き直す（新しい vnode なので、ファイルのキャッシュに無い状態になる）
  rm -rf "$D/dict" "$D/models"; mkdir -p "$D/dict" "$D/models"
  cp -c "$DICT_SRC"/{system.dict,grammar.bin,vectors.bin,postal.tsv,emoji.tsv,BUILD_INFO} "$D/dict/"
  cp -c "$MODEL_SRC" "$D/models/"
}
refresh_data
du -sh "$A"

# entitlement: engine.entitlements と同じ形。一時例外は存在しない試験用のパスに替える（本物のデータの場所を開けない）
ENT="$W/b/probe.entitlements"
cat > "$ENT" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>com.apple.security.app-sandbox</key><true/>
<key>com.apple.security.temporary-exception.files.home-relative-path.read-write</key>
<array><string>/Library/Application Support/ikari-ime-spike578/</string></array>
</dict></plist>
EOF
common=(--force --options runtime --timestamp=none -s "$SIGN")
echo "##### 署名 ($MODE)"
codesign "${common[@]}" -i "$ENG" --entitlements "$ENT" "$X/Contents/MacOS/engine-probe" || exit 1
codesign "${common[@]}" --entitlements "$ENT" "$X" || exit 1
codesign "${common[@]}" -i "$HOST" "$A/Contents/MacOS/xpc-host" || exit 1
codesign "${common[@]}" "$A" || exit 1
codesign --verify --deep --strict "$A" && echo "verify OK"
grep -c "Resources/data/" "$X/Contents/_CodeSignature/CodeResources" | sed 's/^/CodeResources に載った data のファイル数: /'
codesign -d --entitlements - "$X" 2>&1 | grep -E 'app-sandbox|temporary' | head -3

echo "outside" > "$W/outside.txt"
cc -O2 -o "$W/b/resident" "$HERE/resident.c" || exit 1
R() { "$W/b/resident" "$D/dict/system.dict" "$D/dict/vectors.bin" "$D/models/"*.gguf | sed "s|$D/||"; }
clock_ms() { perl -MTime::HiRes=time -e 'printf "%d", time*1000'; }
echo "##### C1-a XPC（launchd が起こす）: 新しい識別子の最初の起動。冷えた状態（clone で置き直した直後）"
refresh_data; R
echo "--- 殻を、無い名前のサービスで（殻の最初の exec だけ。XPC は起きない）"
s=$(clock_ms); "$A/Contents/MacOS/xpc-host" "$SVC.nonexistent" /dev/null >/dev/null 2>&1; echo "  wall=$(( $(clock_ms) - s ))ms"; R
echo "--- 本物のサービス（最初の起動。コンテナができる）"
s=$(clock_ms); "$A/Contents/MacOS/xpc-host" "$SVC" "$W/outside.txt"; echo "  wall=$(( $(clock_ms) - s ))ms"
echo "##### C1-a XPC: 置き直して 2 回目の起動（データは冷えている）"
refresh_data
"$A/Contents/MacOS/xpc-host" "$SVC" "$W/outside.txt"
echo "##### C1-a XPC: 2 回目（ファイルのキャッシュに載った状態）"
"$A/Contents/MacOS/xpc-host" "$SVC" "$W/outside.txt"
echo "##### C1-b engine-probe engine: 冷えた状態"
refresh_data
"$X/Contents/MacOS/engine-probe" engine
echo "##### C1-b engine-probe engine: 2 回目"
"$X/Contents/MacOS/engine-probe" engine
echo "##### C1-b engine-probe mmap と deny"
"$X/Contents/MacOS/engine-probe" mmap
"$X/Contents/MacOS/engine-probe" deny "$W/outside.txt"
codesign --verify --deep --strict "$A" && echo "置き直したあとも verify OK"
echo "##### 終わり $MODE"
