#!/usr/bin/env bash
# ADR 0039 PoC: eval/sets/*.tsv の全部（segments.tsv を除く）と AJIMEE を、辞書を替えて流す（捨てるスクリプト。snapshot.sh が流さないセットも含める）。
#   webwords_allsets.sh <辞書ディレクトリ> <出力ディレクトリ> [<LM の gguf>]
# 出力: <出力>/eval/<セット名>.tsv（--out-cases）。webwords_compare.py で 2 つの出力を比べる。
# LM を渡すと --lm-budget-ms 2000 --lm-deadline-ms 2000（snapshot.sh と同じ）。LM つきのバイナリは target/snapshot-bin に作られているものを使う。
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
DICT=$1
OUT=$2/eval
LM=${3:-}
BIN=target/release/ikari-eval
EXTRA=()
if [ -n "$LM" ]; then
  BIN=target/snapshot-bin/release/ikari-eval
  EXTRA=(--lm "$LM" --lm-budget-ms 2000 --lm-deadline-ms 2000)
fi
mkdir -p "$OUT"
for set in eval/sets/*.tsv; do
  name=$(basename "$set" .tsv)
  [ "$name" = segments ] && continue
  # ONLY="a b c" を渡すと、その名前のセットだけ流す（LM ありは重いので）
  if [ -n "${ONLY:-}" ] && [[ " $ONLY " != *" $name "* ]]; then continue; fi
  "$BIN" --dict "$DICT" --ranker full ${EXTRA[@]+"${EXTRA[@]}"} --set "$set" --out-cases "$OUT/$name.tsv" > "$OUT/$name.md" 2>&1 || echo "warn: $name failed"
done
if [ -z "${ONLY:-}" ] && [ -f data/raw/ajimee/evaluation_items.json ]; then
  "$BIN" --dict "$DICT" --ranker full ${EXTRA[@]+"${EXTRA[@]}"} --set data/raw/ajimee/evaluation_items.json --set-format ajimee --out-cases "$OUT/ajimee.tsv" > "$OUT/ajimee.md" 2>&1 || echo "warn: ajimee failed"
fi
# LM の時間切れ（0 でなければ無効）
if [ -n "$LM" ]; then
  grep -h -i "時間切れ\|timeout" "$OUT"/*.md | head -3 || true
fi
echo "done $OUT"
