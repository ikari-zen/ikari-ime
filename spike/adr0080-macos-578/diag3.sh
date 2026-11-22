#!/bin/bash
# スパイク #578。捨てるもの。「束の主の実行ファイル（殻）が新しくなったとき（cdhash が変わった。アプリの更新に当たる）、
# 最初の exec で束全体が読まれるか」と、その時間。c1.sh のあとに流す。XPC は起こさない。
# 使い方: diag3.sh <作業dir> <adhoc|team> <c1.sh の印> <新しい印>
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
W="$1"; MODE="$2"; TAG="$3"; NONCE="$4"
DICT_SRC="${DICT_SRC:-$(git -C "$REPO" worktree list --porcelain | sed -n '1s/^worktree //p')/data/build/dict}"
MODEL_SRC="${MODEL_SRC:-$HOME/Library/Application Support/ikari-ime/models/zenz-v3.2-small-Q5_K_M.gguf}"
PFX="io.github.ikari-zen.spike578.$MODE$TAG"
case "$MODE" in
  adhoc) SIGN=- ;;
  team) SIGN="$(security find-identity -v -p codesigning | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "Apple Development.*/\1/p' | head -1)" ;;
esac
A="$W/Spike578-$MODE$TAG.app"; X="$A/Contents/XPCServices/Probe.xpc"; D="$X/Contents/Resources/data"
R() { "$W/b/resident" "$D/dict/system.dict" "$D/dict/vectors.bin" "$D/models/"*.gguf | sed "s|$D/||"; }
refresh_data() {
  rm -rf "$D/dict" "$D/models"; mkdir -p "$D/dict" "$D/models"
  cp -c "$DICT_SRC"/{system.dict,grammar.bin,vectors.bin,postal.tsv,emoji.tsv,BUILD_INFO} "$D/dict/"
  cp -c "$MODEL_SRC" "$D/models/"
}
clock_ms() { perl -MTime::HiRes=time -e 'printf "%d", time*1000'; }
common=(--force --options runtime --timestamp=none -s "$SIGN")
run_host() { local s; s=$(clock_ms); "$A/Contents/MacOS/xpc-host" "$PFX.probe.nonexistent" /dev/null >/dev/null 2>&1; echo "  host exec wall=$(( $(clock_ms) - s ))ms"; }

echo "== 殻を印 $NONCE で作り直して署名し直す（データの封印は同じ）"
cc -O2 -o "$A/Contents/MacOS/xpc-host" -x c - < <(cat "$HERE/xpc-host.c"; echo "const char *spike_nonce = \"$NONCE\";") || exit 1
codesign "${common[@]}" -i "$PFX.host" "$A/Contents/MacOS/xpc-host" 2>/dev/null || exit 1
codesign "${common[@]}" "$A" 2>/dev/null || exit 1
refresh_data; R
echo "== 新しい殻の最初の exec"; run_host; R
echo "== 置き直して、同じ殻の 2 回目の exec"; refresh_data; run_host; R
