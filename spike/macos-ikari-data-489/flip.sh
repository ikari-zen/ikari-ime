#!/bin/bash
# 追加の確認: 同じ識別子（＝同じコンテナ）で、署名の方式（ad-hoc ↔ team）を替えると起動できるか。
# 使い方: flip.sh <作業ディレクトリ> <識別子の印。run.sh の 3 番目と同じ> 。先に run.sh を同じ印で流しておく。
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
W="$1"; SUF="$2"
ID=io.github.ikari-zen.inputmethod.ikari.data.spike$SUF
TEAM="$(security find-identity -v -p codesigning | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "Apple Development.*/\1/p' | head -1)"
ROOT="$HOME/Library/Application Support/ikari-ime-spike"
mkdir -p "$W/flip" "$ROOT"
for who in "-" "$TEAM"; do
  cp "$W/b/data-spike" "$W/flip/ds"
  codesign --force --options runtime --timestamp=none -s "$who" -i "$ID" --entitlements "$HERE/data.entitlements" "$W/flip/ds" 2>&1 | tail -1
  echo "--- 署名 ${who:0:6} （同じ識別子、既存のコンテナ）"
  perl -e 'alarm 15; exec @ARGV' "$W/flip/ds" probe 2>&1 | sed -n '1,2p'
  echo "rc=${PIPESTATUS[0]}（142 = 15 秒で打ち切り）"
done
rm -rf "$ROOT"
