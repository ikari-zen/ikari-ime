#!/bin/bash
# scripts/diag-report.py の出力が、固定のログ（scripts/testdata/diag/）に対する期待（expected.md）と一致するか確かめる。
# 期待を更新するとき: scripts/test-diag-report.sh --update
# テストでは、ユーザーのログ（~/Library/Application Support/ikari-ime/logs/）は読まない。
set -euo pipefail

# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "$IKARI_ROOT"

readonly DATA="scripts/testdata/diag"
readonly EXPECTED="$DATA/expected.md"
readonly REPORT="scripts/diag-report.py"

command -v python3 >/dev/null || die "python3 がありません"

if [[ "${1:-}" == "--update" ]]; then
  python3 -I "$REPORT" "$DATA" > "$EXPECTED"
  log "更新しました: $EXPECTED"
  exit 0
fi

actual="$(python3 -I "$REPORT" "$DATA")"
[[ "$actual" == "$(cat "$EXPECTED")" ]] || {
  diff <(printf '%s\n' "$actual") "$EXPECTED" >&2 || true
  die "diag-report.py の出力が $EXPECTED と違います"
}

# ファイルを直接渡しても（順序が逆でも）同じ出力になる
files="$(python3 -I "$REPORT" "$DATA/engine.log" "$DATA/engine-20261025-000000.log.gz")"
[[ "$files" == "$actual" ]] || die "ファイル指定とディレクトリ指定で出力が違います"

# TSV は行の先頭に節名がつく
tsv="$(python3 -I "$REPORT" --tsv "$DATA")"
printf '%s\n' "$tsv" | grep -q '^ログの健全性	読めた行	33$' || die "TSV に期待の行がありません"

# 日付の絞り込み（2026-10-27 だけ → 古い世代の行は入らない）
since="$(python3 -I "$REPORT" --since 2026-10-27 "$DATA")"
printf '%s\n' "$since" | grep -q '^| 読めた行 | 26 |$' || die "--since の結果が違います"

# smoke のセッションは既定で除く（--include-smoke で戻す）
printf '%s\n' "$actual" | grep -q '^| 除いた smoke の行 | 3 |$' || die "smoke の行を除けていません"
smoke="$(python3 -I "$REPORT" --include-smoke "$DATA")"
printf '%s\n' "$smoke" | grep -q '^| smoke の行（除かない） | 3 |$' || die "--include-smoke の結果が違います"
printf '%s\n' "$smoke" | grep -q '^| pick=5 | 1 | 33.3% |$' || die "--include-smoke で smoke の合成が入っていません"
printf '%s\n' "$actual" | grep -q '^| pick=5 | 0 | 0.0% |$' || die "smoke の合成が既定で入っています"

# app の無い begin_session（secure のセッション、古い版の行）があっても落ちない（#63）
noapp="$(mktemp -d)"
trap 'rm -rf "$noapp"' EXIT
printf '%s\n' \
  "1793114850000 begin_session 5us lm=ready err=- session=1" \
  "1793114851000 convert 40us lm=ready err=- segments=1 cache=hit lm_applied=1 ctx_rev=1" \
  "1793114852000 end_session 3us lm=ready err=- session=1" > "$noapp/engine.log"
python3 -I "$REPORT" "$noapp" > /dev/null || die "app の無い begin_session で diag-report.py が落ちました"

# 先行計算に補った left（ADR 0038）の集計: fill_eq、fill_tail、fill_dleft、by=content
fill="$(mktemp -d)"
trap 'rm -rf "$noapp" "$fill"' EXIT
printf '%s\n' \
  "1793114850000 convert 40us lm=ready err=- session=1 conv=1 segments=1 cache=hit lm_applied=1 by=content gap=900 confident=0 fill=2 fill_eq=1 fill_tail=1 fill_dleft=+0 ctx_rev=2" \
  "1793114851000 convert 40us lm=ready err=- session=1 conv=2 segments=1 cache=miss lm_applied=0 miss=context gap=900 confident=0 fill=2 fill_eq=0 fill_tail=0 fill_dleft=+1 ctx_rev=3" \
  "1793114852000 convert 40us lm=ready err=- session=1 conv=3 segments=1 cache=miss lm_applied=0 miss=context gap=900 confident=0 fill=3 fill_eq=0 fill_tail=1 fill_dleft=-2 ctx_rev=4" \
  "1793114853000 convert 40us lm=ready err=- session=1 conv=4 segments=1 cache=hit lm_applied=1 gap=900 confident=0 ctx_rev=4" > "$fill/engine.log"
fillout="$(python3 -I "$REPORT" "$fill")"
for row in \
  '| by=content | 1 | 25.0% |' \
  '| LM なし（lm_applied=0） | 2 | 50.0% |' \
  '| fill のある convert | 3 | 75.0% |' \
  '| fill_eq=1 | 1 | 33.3% |' \
  '| fill_eq=0 | 2 | 66.7% |' \
  '| fill_eq=0 fill_tail=1 | 1 | 50.0% |' \
  '| fill_eq=0 fill_tail=0 | 1 | 50.0% |' \
  '| fill_eq=0 fill_dleft=-2 | 1 | 50.0% |' \
  '| fill_eq=0 fill_dleft=+1 | 1 | 50.0% |'; do
  printf '%s\n' "$fillout" | grep -qxF "$row" || die "fill の集計に期待の行がありません: $row"
done

# アプリ別の convert と miss=context（#109）: 行の app= を優先し、なければ session= で begin_session の app= に結ぶ。結べない行は (不明)
appd="$(mktemp -d)"
trap 'rm -rf "$noapp" "$fill" "$appd"' EXIT
printf '%s\n' \
  "1793114850000 convert 40us lm=ready err=- session=9 conv=1 segments=1 cache=miss lm_applied=0 miss=context" \
  "1793114850500 begin_session 5us lm=ready err=- session=1 app=com.example.Editor" \
  "1793114851000 convert 40us lm=ready err=- session=1 conv=2 segments=1 cache=miss lm_applied=0 miss=context" \
  "1793114852000 convert 40us lm=ready err=- session=1 conv=3 segments=1 cache=hit lm_applied=1" \
  "1793114853000 convert 40us lm=ready err=- session=1 conv=4 segments=1 cache=miss lm_applied=0 miss=reading" \
  "1793114854000 convert 40us lm=ready err=- session=1 conv=5 segments=1 cache=miss lm_applied=0 miss=context app=com.example.Terminal" \
  "1793114855000 convert 40us lm=ready err=- session=7 conv=6 segments=1 cache=hit lm_applied=1 app=none" \
  "1793114856000 convert 40us lm=ready err=- session=8 conv=7 segments=1 cache=hit lm_applied=1" > "$appd/engine.log"
appout="$(python3 -I "$REPORT" "$appd")"
for row in \
  '| com.example.Editor | 3 | 1 | 33.3% | 2 | 66.7% |' \
  '| (不明) | 2 | 1 | 50.0% | 1 | 50.0% |' \
  '| com.example.Terminal | 1 | 1 | 100.0% | 1 | 100.0% |' \
  '| none | 1 | 0 | 0.0% | 0 | 0.0% |'; do
  printf '%s\n' "$appout" | grep -qxF "$row" || die "アプリ別の集計に期待の行がありません: $row"
done

# smoke のセグメントのテストは、app= のない begin_session で始まる。smoke の begin_session があるエンジンの起動（epoch）の
# 行は、app= のない begin_session があっても smoke として除く。app が smoke 以外の begin_session がある epoch は除かない。
smk="$(mktemp -d)"
trap 'rm -rf "$noapp" "$fill" "$appd" "$smk"' EXIT
printf '%s\n' \
  "1793114850000 begin_session 5us lm=ready err=- session=1 app=com.example.inputmethod.ikari.smoke" \
  "1793114850100 convert 40us lm=ready err=- session=1 conv=1 segments=1 cache=hit lm_applied=1 ctx_rev=1" \
  "1793114850200 end_session 3us lm=ready err=- session=1" \
  "1793114850300 remove_user_dict 87us lm=ready err=NOT_FOUND" \
  "1793114850400 begin_session 1us lm=ready err=- session=2" \
  "1793114850500 convert 337us lm=ready err=- session=2 conv=4 segments=2 cache=miss lm_applied=0 miss=none" \
  "1793114850600 segment_page 58us lm=ready err=- conv=4" > "$smk/engine.log"
smkout="$(python3 -I "$REPORT" "$smk")"
printf '%s\n' "$smkout" | grep -qxF '| 除いた smoke の行 | 7 |' || die "app= のない begin_session を含む smoke の epoch を除けていません"
printf '%s\n' "$smkout" | grep -qxF '| miss=none | 1 | 100.0% |' && die "smoke のセグメントのテストの miss=none が通常の使用に数えられています"
printf '%s\n' \
  "1793114850000 begin_session 5us lm=ready err=- session=1 app=com.example.inputmethod.ikari.smoke" \
  "1793114850100 begin_session 5us lm=ready err=- session=2 app=com.example.Editor" \
  "1793114850200 begin_session 1us lm=ready err=- session=3" \
  "1793114850300 convert 337us lm=ready err=- session=3 conv=4 segments=2 cache=miss lm_applied=0 miss=none" > "$smk/engine.log"
mixout="$(python3 -I "$REPORT" "$smk")"
# smoke のセッション自身の 1 行だけが smoke。利用者のセッションの行（app= のないものを含む）は除かない
printf '%s\n' "$mixout" | grep -qxF '| 除いた smoke の行 | 1 |' || die "利用者のアプリの begin_session がある epoch を smoke として除いています"
printf '%s\n' "$mixout" | grep -qxF '| miss=none | 1 | 100.0% |' || die "利用者の epoch の miss=none が数えられていません"

log "diag-report のテスト: OK"
