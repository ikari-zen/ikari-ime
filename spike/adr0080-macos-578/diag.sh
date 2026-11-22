#!/bin/bash
# スパイク #578。捨てるもの。c1.sh で作った束で、XPC の起動の前に誰がデータを読むかを分ける。
# 使い方: diag.sh <作業dir> <adhoc|team>
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
W="$1"; MODE="$2"
DICT_SRC="${DICT_SRC:-$(git -C "$REPO" worktree list --porcelain | sed -n '1s/^worktree //p')/data/build/dict}"
MODEL_SRC="${MODEL_SRC:-$HOME/Library/Application Support/ikari-ime/models/zenz-v3.2-small-Q5_K_M.gguf}"
SVC="io.github.ikari-zen.spike578.$MODE.probe"
A="$W/Spike578-$MODE.app"; X="$A/Contents/XPCServices/Probe.xpc"; D="$X/Contents/Resources/data"
cc -O2 -o "$W/b/resident" "$HERE/resident.c" || exit 1
R() { "$W/b/resident" "$D/dict/system.dict" "$D/dict/vectors.bin" "$D/models/"*.gguf | sed "s|$D/||"; }
refresh_data() {
  rm -rf "$D/dict" "$D/models"; mkdir -p "$D/dict" "$D/models"
  cp -c "$DICT_SRC"/{system.dict,grammar.bin,vectors.bin,postal.tsv,emoji.tsv,BUILD_INFO} "$D/dict/"
  cp -c "$MODEL_SRC" "$D/models/"
}
t() { local s; s=$(clock_ms); "$@"; echo "  ($(( $(clock_ms) - s ))ms: $*)" | sed "s|$W|W|g"; }
clock_ms() { perl -MTime::HiRes=time -e 'printf "%d", time*1000'; }

echo "== 1. clone で置き直した直後"; refresh_data; R
echo "== 2. 殻（xpc-host）を、無い名前のサービスで起こす（XPC は起きない）"
t "$A/Contents/MacOS/xpc-host" "$SVC.nonexistent" /dev/null | tail -2; R
echo "== 3. 置き直して、本物のサービスで"
refresh_data; R
t "$A/Contents/MacOS/xpc-host" "$SVC" /dev/null | grep -E 'resident|round'; R
echo "== 4. 置き直して、もう一度本物のサービスで（2 回目の置き直し）"
refresh_data
t "$A/Contents/MacOS/xpc-host" "$SVC" /dev/null | grep -E 'system.dict|round'
echo "== 5. 置き直して、codesign --verify --deep --strict だけ"
refresh_data; t codesign --verify --deep --strict "$A"; R
