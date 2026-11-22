#!/bin/bash
# スパイク #578。捨てるもの。「新しい場所に置いた束の、殻の最初の exec」で束全体が読まれるか。データあり・なしで比べる。
# 束を clone で写す（新しい場所・新しい inode。データは冷えている）。XPC は起こさない（殻は Sandbox なし）。
# 使い方: diag4.sh <作業dir> <c1.sh で作った束の名前（例 Spike578-adhoc.r2.app）> <印>
set -u
W="$1"; SRC="$W/$2"; N="$3"
clock_ms() { perl -MTime::HiRes=time -e 'printf "%d", time*1000'; }
R() { local D="$1/Contents/XPCServices/Probe.xpc/Contents/Resources/data"; "$W/b/resident" "$D/dict/system.dict" "$D/dict/vectors.bin" "$D/models/"*.gguf | sed "s|$D/||"; }
run_host() { local s; s=$(clock_ms); "$1/Contents/MacOS/xpc-host" none.nonexistent /dev/null >/dev/null 2>&1; echo "  host exec wall=$(( $(clock_ms) - s ))ms"; }
for kind in data nodata; do
  B="$W/copy-$N-$kind.app"; rm -rf "$B"; cp -cR "$SRC" "$B"
  if [ "$kind" = nodata ]; then
    rm -rf "$B/Contents/XPCServices/Probe.xpc/Contents/Resources/data"
  fi
  echo "== $kind: $(du -sh "$B" | cut -f1)"
  [ "$kind" = data ] && R "$B"
  echo "-- 最初の exec"; run_host "$B"; [ "$kind" = data ] && R "$B"
  echo "-- 2 回目の exec"; run_host "$B"
  rm -rf "$B"
done
