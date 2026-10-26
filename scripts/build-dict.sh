#!/bin/bash
# 辞書（grammar.bin、system.dict、vectors.bin）を data/raw/ から data/build/dict/ に作る（SudachiDict Full: small + core + notcore。ADR 0012 §11.3）。
#
#   scripts/build-dict.sh [--data <data のディレクトリ>] [--rebuild] [--town-cost <コスト>] [--chain-cost <コスト>] [--chain-municipality-cost <コスト>] [--no-chain] [--policy <ファイル>]
#
#   --rebuild    入力が前回と同じでも作り直す
#   --town-cost  ABR の地名の行のコスト（掃引用。省略すると dict-builder の既定。ADR 0030 §4）
#   --chain-cost 住所の連なり（都道府県 + 市区町村 + 町名）の行のコスト（掃引用。ADR 0030 §12）
#   --chain-municipality-cost 「都道府県 + 市区町村」だけの連なり（町名がない形）の行のコスト（掃引用。ADR 0030 §13）
#   --no-chain   住所の連なりを足さない（掃引・比較用）
#   --policy     ポリシーのファイル（掃引用。省略すると dict-builder/policy/chive.tsv）
#
# 入力: data/raw/sudachi-20260723/{small_lex.csv,core_lex.csv,notcore_lex.csv,matrix.def}、data/raw/chive/chive-1.3-mc90/chive-1.3-mc90.txt、
#       dict-builder/policy/chive.tsv（製品の辞書のポリシー）。足りなければ scripts/fetch-data.sh の案内を出して止まる。
# 任意の入力: data/raw/abr/mt_town_all.csv（ABR の町字マスター。scripts/fetch-data.sh --only abr）があれば、dict-builder の town で
#       大字・町名の地名の lex.csv（data/build/town_lex.csv）を作り、notcore と同じ扱いで辞書に足す（ADR 0030）。なければ足さない（ログに出す）。
# notcore は辞書の行にはなるが、品詞の統計と活用規則には数えない（grammar.bin は Core だけのときと同じ）。固有名詞は d2s で絞り、
#       新語（固有名詞以外で chiVe にない lemma）には lemma_rank を当てない。vectors.bin は Sudachi の分割の平均を chiVe にない lemma に付ける。
# 手順: dict.new に作る -> vectors.bin を作る -> --verify -> dict を dict.prev に、dict.new を dict に入れ替える
#       （旧版を 1 世代だけ残す。spec の「旧版は残し、いつでも戻せる」）。
# BUILD_INFO: data/build/dict/BUILD_INFO に入力の大きさと sha256、ポリシーの sha256、dict-builder と dict-format のツリー、
#       出力の sha256 を書く。入力の部分が前回と同じなら、ビルドを飛ばす（ビルドは決定的なので結果は同じ）。
# emoji.tsv: data/raw/emoji-data/emoji.json があれば data/build/dict/emoji.tsv を作る（ADR 0027 §5。辞書の入力・BUILD_INFO・出力の sha256 には
#       含めない。辞書が最新で飛ばすときも作る。emoji.json がなければ作らず、辞書だけを作る）。
# postal.tsv: data/raw/abr/abr_post_code.csv（ABR の町字・郵便番号変換表。scripts/fetch-data.sh --only postal）と町字マスターの両方があれば、
#       data/build/dict/postal.tsv を作る（ADR 0032。emoji.tsv と同じく辞書の入力・BUILD_INFO・出力の sha256 には含めない。辞書が最新で飛ばすときも
#       作る。なければ作らず、辞書だけを作る）。町字マスターから町名・読みを引くので、辞書のために dict-builder の town を呼ぶ（--no-chain のときも連なりと同じ読みで作る）。
# ポリシーのレポートは data/build/dict-policy-report.md に出す。
# vectors.bin は辞書の dict_id に結び付くので、辞書を作り直したら必ず作り直す（このスクリプトは常に両方を作る）。
set -euo pipefail

# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

DATA="$IKARI_ROOT/data"
REBUILD=0
TOWN_COST=""
CHAIN_COST=""
CHAIN_MUNI_COST=""
NO_CHAIN=0
POLICY_OVERRIDE=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --data) DATA="${2:-}"; shift ;;
    --rebuild) REBUILD=1 ;;
    --town-cost) TOWN_COST="${2:-}"; shift ;;
    --chain-cost) CHAIN_COST="${2:-}"; shift ;;
    --chain-municipality-cost) CHAIN_MUNI_COST="${2:-}"; shift ;;
    --no-chain) NO_CHAIN=1 ;;
    --policy) POLICY_OVERRIDE="${2:-}"; shift ;;
    -h | --help) sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "不明な引数: $1" ;;
  esac
  shift
done
[[ -n "$DATA" ]] || die "--data が空です"

RAW="$DATA/raw"
BUILD="$DATA/build"
SUDACHI="$RAW/$SUDACHI_DIR_NAME"
CHIVE="$RAW/chive/$CHIVE_NAME/$CHIVE_NAME.txt"
POLICY="$IKARI_ROOT/$DICT_POLICY"
if [[ -n "$POLICY_OVERRIDE" ]]; then POLICY="$POLICY_OVERRIDE"; fi
BUILDER="$IKARI_ROOT/target/release/ikari-dict-builder"
INPUTS=("$SUDACHI/small_lex.csv" "$SUDACHI/core_lex.csv" "$SUDACHI/notcore_lex.csv" "$SUDACHI/matrix.def" "$CHIVE")
# ABR の町字マスター（任意。ADR 0030）。あれば入力に数える
ABR_CSV="$RAW/$ABR_TOWN_CSV"
TOWN_LEX="$BUILD/$ABR_TOWN_LEX_FILE"
CHAIN_LEX="$BUILD/$ABR_CHAIN_LEX_FILE"
USE_ABR=0
if [[ -f "$ABR_CSV" ]]; then
  USE_ABR=1
  INPUTS+=("$ABR_CSV")
else
  log "ABR の町字マスターがないので、大字・町名は足しません（${ABR_CSV}。scripts/fetch-data.sh --only abr で取得できます）"
fi

for f in "${INPUTS[@]}"; do
  [[ -f "$f" ]] || die "$f がありません。scripts/fetch-data.sh で取得してください（--data を使うときは同じ値を渡す）"
done
[[ -f "$POLICY" ]] || die "$POLICY がありません"

# ツリーの識別子（コミット済みの内容）。作業ツリーに未コミットの変更があれば +dirty を付ける
tree_id() { # <ディレクトリ>
  local id
  id="$(git -C "$IKARI_ROOT" rev-parse "HEAD:$1" 2>/dev/null)" || { printf 'unknown'; return; }
  if [[ -n "$(git -C "$IKARI_ROOT" status --porcelain -- "$1" 2>/dev/null)" ]]; then id="$id+dirty"; fi
  printf '%s' "$id"
}

# 入力の部分（BUILD_INFO の output. 以外の行）
input_info() {
  local f
  printf 'format=1\n'
  for f in "${INPUTS[@]}"; do
    printf 'input.%s=%s %s\n' "$(basename "$f")" "$(wc -c <"$f" | tr -d ' ')" "$(sha256_of "$f")"
  done
  if [[ "$USE_ABR" -eq 1 ]]; then printf 'town.cost=%s\n' "${TOWN_COST:-default}"
    if [[ "$NO_CHAIN" -eq 1 ]]; then printf 'town.chain=off\n'; else printf 'town.chain-cost=%s\ntown.chain-municipality-cost=%s\n' "${CHAIN_COST:-default}" "${CHAIN_MUNI_COST:-default}"; fi
  fi
  printf 'policy.%s=%s\n' "$(basename "$POLICY")" "$(sha256_of "$POLICY")"
  printf 'builder.dict-builder=%s\n' "$(tree_id dict-builder)"
  printf 'builder.dict-format=%s\n' "$(tree_id dict-format)"
}

# 絵文字の表を作る（なければ飛ばす）。決定的なので毎回作り直してよい。辞書の入れ替えのあとに呼ぶ
build_emoji() {
  local json="$RAW/$EMOJI_DATA_JSON"
  if [[ ! -f "$json" ]]; then
    log "$EMOJI_TABLE_FILE は作りません（$json がありません。scripts/fetch-data.sh --only emoji で取得できます）"
    return
  fi
  [[ -x "$BUILDER" ]] || (cd "$IKARI_ROOT" && cargo build --release -p ikari-dict-builder)
  log "$EMOJI_TABLE_FILE を作る"
  mkdir -p "$BUILD/dict"
  "$BUILDER" emoji --json "$json" --commit "$EMOJI_DATA_COMMIT" --out "$BUILD/dict/$EMOJI_TABLE_FILE"
}

# 住所と郵便番号の表を作る（変換表と町字マスターがなければ飛ばす）。決定的なので毎回作り直してよい。辞書の入れ替えのあとに呼ぶ。
# town は辞書のためのファイルも同時に書くので、出力先は postal 専用の一時ディレクトリにして捨てる（辞書の lex.csv には触れない）
build_postal() {
  local csv="$RAW/$ABR_POSTAL_CSV" tmp
  if [[ ! -f "$csv" ]]; then
    log "$POSTAL_TABLE_FILE は作りません（$csv がありません。scripts/fetch-data.sh --only postal で取得できます）"
    return
  fi
  if [[ "$USE_ABR" -ne 1 ]]; then
    log "$POSTAL_TABLE_FILE は作りません（町字マスター $ABR_CSV がありません。scripts/fetch-data.sh --only abr で取得できます）"
    return
  fi
  [[ -x "$BUILDER" ]] || (cd "$IKARI_ROOT" && cargo build --release -p ikari-dict-builder)
  log "$POSTAL_TABLE_FILE を作る"
  mkdir -p "$BUILD/dict"
  tmp="$(mktemp -d "$BUILD/postal.XXXXXX")"
  "$BUILDER" town --csv "$ABR_CSV" --lex "$SUDACHI/small_lex.csv" --lex "$SUDACHI/core_lex.csv" --lex "$SUDACHI/notcore_lex.csv" \
    --out "$tmp/town_lex.csv" --postal-csv "$csv" --postal-out "$tmp/$POSTAL_TABLE_FILE" || { rm -rf "$tmp"; die "$POSTAL_TABLE_FILE を作れませんでした"; }
  mv -f "$tmp/$POSTAL_TABLE_FILE" "$BUILD/dict/$POSTAL_TABLE_FILE"
  rm -rf "$tmp"
}

INFO_FILE="$BUILD/dict/BUILD_INFO"
log "入力を確かめる（sha256）"
NEW_INPUT_INFO="$(input_info)"

if [[ "$REBUILD" -eq 0 && -f "$INFO_FILE" ]]; then
  complete=1
  for f in "${DICT_FILES[@]}"; do [[ -f "$BUILD/dict/$f" ]] || complete=0; done
  if [[ "$complete" -eq 1 ]] && [[ "$(grep -v '^output\.' "$INFO_FILE")" == "$NEW_INPUT_INFO" ]]; then
    log "辞書は最新です（入力が BUILD_INFO と同じ）。--rebuild で作り直せます: $BUILD/dict"
    build_emoji
    build_postal
    exit 0
  fi
fi

log "ikari-dict-builder をビルド"
(cd "$IKARI_ROOT" && cargo build --release -p ikari-dict-builder)
[[ -x "$BUILDER" ]] || die "$BUILDER がありません"

NEW="$BUILD/dict.new"
rm -rf "$NEW"
mkdir -p "$BUILD"
# 失敗したら途中のものを残さない
trap 'rm -rf "$NEW"' EXIT

# 辞書に足す lex.csv の並び（vectors.bin を作るときにも同じ順で渡す）
LEX_ARGS=(--lex "$SUDACHI/small_lex.csv" --lex "$SUDACHI/core_lex.csv" --lex "$SUDACHI/notcore_lex.csv")
NOTCORE_ARGS=(--notcore-lex "$SUDACHI/notcore_lex.csv")
if [[ "$USE_ABR" -eq 1 ]]; then
  log "ABR の町字マスターから地名の lex.csv を作る -> $TOWN_LEX"
  mkdir -p "$BUILD"
  TOWN_ARGS=()
  if [[ -n "$TOWN_COST" ]]; then TOWN_ARGS+=(--cost "$TOWN_COST"); fi
  if [[ "$NO_CHAIN" -eq 0 ]]; then
    TOWN_ARGS+=(--chain-out "$CHAIN_LEX")
    if [[ -n "$CHAIN_COST" ]]; then TOWN_ARGS+=(--chain-cost "$CHAIN_COST"); fi
    if [[ -n "$CHAIN_MUNI_COST" ]]; then TOWN_ARGS+=(--chain-municipality-cost "$CHAIN_MUNI_COST"); fi
  fi
  "$BUILDER" town --csv "$ABR_CSV" "${LEX_ARGS[@]}" ${TOWN_ARGS[@]+"${TOWN_ARGS[@]}"} --out "$TOWN_LEX"
  LEX_ARGS+=(--lex "$TOWN_LEX")
  NOTCORE_ARGS+=(--notcore-lex "$TOWN_LEX")
  # 住所の連なり: 同じ行の Core（Sudachi の郵便番号の住所）より優先する（ADR 0030 §12）
  if [[ "$NO_CHAIN" -eq 0 ]]; then
    LEX_ARGS+=(--lex "$CHAIN_LEX")
    NOTCORE_ARGS+=(--chain-lex "$CHAIN_LEX")
  fi
fi

log "辞書を作る -> $NEW"
"$BUILDER" \
  --policy "$POLICY" --chive-vocab "$CHIVE" \
  --lex "$SUDACHI/small_lex.csv" --lex "$SUDACHI/core_lex.csv" "${NOTCORE_ARGS[@]}" \
  --matrix "$SUDACHI/matrix.def" --label "sudachi-full-$SUDACHI_VERSION" \
  --out "$NEW" --policy-report "$BUILD/dict-policy-report.md"

log "vectors.bin を作る"
"$BUILDER" vectors --chive "$CHIVE" --dict "$NEW" --out "$NEW/vectors.bin" "${LEX_ARGS[@]}"

log "検証"
"$BUILDER" --verify "$NEW"

{
  printf '%s\n' "$NEW_INPUT_INFO"
  for f in "${DICT_FILES[@]}"; do
    [[ -f "$NEW/$f" ]] || die "$NEW/$f ができていません"
    printf 'output.%s=%s\n' "$f" "$(sha256_of "$NEW/$f")"
  done
} >"$NEW/BUILD_INFO"

log "入れ替え: dict -> dict.prev、dict.new -> dict"
rm -rf "$BUILD/dict.prev"
if [[ -d "$BUILD/dict" ]]; then mv "$BUILD/dict" "$BUILD/dict.prev"; fi
mv "$NEW" "$BUILD/dict"
trap - EXIT
build_emoji
build_postal

log "完了: $BUILD/dict"
grep '^output\.' "$BUILD/dict/BUILD_INFO"
