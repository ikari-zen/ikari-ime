#!/bin/bash
# スパイク #536（捨てるコード。マージしない）。../cage-peer-check-spike の束を作り、host をこの測定用に差し替える。
#   macos/tools/cage-wait-spike/build.sh   →  $TMPDIR/ikari-cage-wait-spike/CageSpike.app
#   "$TMPDIR/ikari-cage-wait-spike/CageSpike.app/Contents/MacOS/host" 10
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="${TMPDIR:-/tmp}/ikari-cage-wait-spike"
"$here/../cage-peer-check-spike/build.sh" --out "$out" >/dev/null
app="$out/CageSpike.app"
clang -Wall -Wextra -O1 -o "$app/Contents/MacOS/host" "$here/host.c"
codesign --force --sign - --identifier io.github.ikari-zen.spike.host "$app"
codesign --verify --deep --strict "$app"
echo "built: $app"
