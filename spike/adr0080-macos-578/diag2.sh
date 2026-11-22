#!/bin/bash
# スパイク #578。捨てるもの。「署名が新しくなった（cdhash が変わった）XPC サービスの最初の起動」で、
# 起動の前にデータが読まれるか（束の封印の検証か）と、その時間を分ける。c1.sh のあとに流す。
# 使い方: diag2.sh <作業dir> <adhoc|team> <印（毎回違う文字列。cdhash を変えるため）>
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
W="$1"; MODE="$2"; NONCE="$3"
DICT_SRC="${DICT_SRC:-$(git -C "$REPO" worktree list --porcelain | sed -n '1s/^worktree //p')/data/build/dict}"
MODEL_SRC="${MODEL_SRC:-$HOME/Library/Application Support/ikari-ime/models/zenz-v3.2-small-Q5_K_M.gguf}"
SVC="io.github.ikari-zen.spike578.$MODE.probe"; HOST="io.github.ikari-zen.spike578.$MODE.host"
case "$MODE" in
  adhoc) SIGN=- ;;
  team) SIGN="$(security find-identity -v -p codesigning | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "Apple Development.*/\1/p' | head -1)" ;;
esac
A="$W/Spike578-$MODE.app"; X="$A/Contents/XPCServices/Probe.xpc"; D="$X/Contents/Resources/data"
R() { "$W/b/resident" "$D/dict/system.dict" "$D/dict/vectors.bin" "$D/models/"*.gguf | sed "s|$D/||"; }
refresh_data() {
  rm -rf "$D/dict" "$D/models"; mkdir -p "$D/dict" "$D/models"
  cp -c "$DICT_SRC"/{system.dict,grammar.bin,vectors.bin,postal.tsv,emoji.tsv,BUILD_INFO} "$D/dict/"
  cp -c "$MODEL_SRC" "$D/models/"
}
clock_ms() { perl -MTime::HiRes=time -e 'printf "%d", time*1000'; }
common=(--force --options runtime --timestamp=none -s "$SIGN")

echo "== 印 $NONCE で xpc-probe を作り直し、XPC の束と殻を署名し直す（cdhash が変わる）"
cc -O2 -DSPIKE_NONCE="\"$NONCE\"" -o "$X/Contents/MacOS/xpc-probe" -x c - < <(cat "$HERE/xpc-probe.c"; echo "const char *spike_nonce = SPIKE_NONCE;") || exit 1
codesign "${common[@]}" --entitlements "$W/b/probe.entitlements" "$X" 2>/dev/null || exit 1
codesign "${common[@]}" "$A" 2>/dev/null || exit 1
codesign -dvv "$X" 2>&1 | grep -E '^CDHash='
echo "== 置き直す"; refresh_data; R
echo "== 殻を、無い名前のサービスで（殻の exec だけ）"
"$A/Contents/MacOS/xpc-host" "$SVC.nonexistent" /dev/null >/dev/null 2>&1; R
echo "== 本物のサービスで（新しい cdhash の最初の起動）"
s=$(clock_ms); "$A/Contents/MacOS/xpc-host" "$SVC" /dev/null | grep -E 'pid|system.dict|round'; echo "  wall=$(( $(clock_ms) - s ))ms"; R
echo "== 置き直して、もう一度（同じ cdhash の 2 回目の起動。データは冷えている）"
refresh_data
s=$(clock_ms); "$A/Contents/MacOS/xpc-host" "$SVC" /dev/null | grep -E 'pid|system.dict|round'; echo "  wall=$(( $(clock_ms) - s ))ms"
