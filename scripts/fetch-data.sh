#!/bin/bash
# 辞書とモデルの元データを data/raw/ に取得する（開発の道具。製品は通信しない: NFR-17）。
#
#   scripts/fetch-data.sh [--data <data のディレクトリ>] [--only sudachi|chive|zenz|ajimee|emoji|abr|postal]... [--check]
#
#   --data   出力先（既定: このリポジトリの data/）。data/raw/ の下に置く
#   --only   指定したものだけ取得する（複数指定できる。既定はすべて）
#   --check  通信せず、あるべきファイルがあり sha256 が合っているかだけを確かめる（足りなければ終了コード 1）
#
# 取得物（URL、版、sha256 はこのファイルの表に固定している。ダウンロード後に sha256 を確かめ、合わなければ捨てる）:
#   sudachi  SudachiDict 20260723 の small_lex / core_lex / notcore_lex / matrix.def（zip）。
#            small + core が Core、notcore を足したものが Full（ADR 0012 §11.3。notcore は 234MB、125 万行）
#            配布元: http://sudachi.s3-website-ap-northeast-1.amazonaws.com/sudachidict-raw/
#            ライセンス: Apache License 2.0（WorksApplications）
#   chive    chiVe 1.3 mc90（tar.gz）
#            配布元: https://sudachi.s3.ap-northeast-1.amazonaws.com/chive/
#            ライセンス: Apache License 2.0（Works Applications。展開した LICENSE を見ること）
#   zenz     zenz-v3.2-small Q5_K_M（GGUF）
#            配布元: https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf
#            ライセンス: モデルカードでは Apache-2.0。派生物の扱いは THIRD_PARTY_NOTICES.md
#   emoji    iamcal/emoji-data の emoji.json（コミット固定。画像は取らない。ADR 0027 §5.2）
#            配布元: https://github.com/iamcal/emoji-data
#            ライセンス: MIT（Copyright (c) 2013 Cal Henderson。THIRD_PARTY_NOTICES.md）
#   abr      デジタル庁 アドレス・ベース・レジストリ（ABR）の町字マスター mt_town_all.csv（zip。ADR 0030）。大字・町名の語彙の元
#            配布元: https://data.address-br.digital.go.jp/mt_town/mt_town_all.csv.zip（認証なし。11.5MB、展開後 156MB）
#            ライセンス: 公共データ利用規約（第1.0版。PDL1.0）。出典の記載と、加工したことの明記が要る（THIRD_PARTY_NOTICES.md）。
#            配布元は日付で更新される（版番号はない）。sha256 が合わなければ捨てるので、更新されたら ADR 0030 の手順で測り直して下の表を直す
#   postal   ABR「町字・郵便番号変換表」abr_post_code.csv（zip。ADR 0032 §7）。住所 <-> 郵便番号の表 postal.tsv の元（読みの列はない）
#            配布元: デジタル庁 ABR のカタログ（dataset.address-br.digital.go.jp）の項目 1098a4f61dab4f52997839d04245132a
#                    https://www.arcgis.com/sharing/rest/content/items/<項目 id>/data（認証なし。6.4MB、展開後 59MB、UTF-8、CRLF、14 列）
#            ライセンス: ABR の利用規約（PDL1.0）。元は日本郵便の郵便番号データ（版は展開した metadata.txt）。THIRD_PARTY_NOTICES.md
#            配布元は月 1 回ほど更新される（版番号はない）。sha256 が合わなければ捨てるので、更新されたら下の表を測り直して直す
#   ajimee   AJIMEE-Bench の評価データ（eval/scripts/fetch_ajimee.sh を呼ぶ）。CC BY-SA 3.0
# ライセンスの確認は人が行う（spec）。ここは確認済みの配布元だけを固定する。
#
# ダウンロードは一時ファイルに書き、sha256 を確かめてから置く。すでに正しいファイルがあれば何もしない。
set -euo pipefail

# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

readonly SUDACHI_RAW_URL="http://sudachi.s3-website-ap-northeast-1.amazonaws.com/sudachidict-raw"
# lex は版のディレクトリの下、matrix.def.zip は版に依らず sudachidict-raw/ の直下にある（matrix.def は 2019 年から変わらない）
readonly CHIVE_URL="https://sudachi.s3.ap-northeast-1.amazonaws.com/chive/$CHIVE_NAME.tar.gz"
readonly EMOJI_URL="https://raw.githubusercontent.com/iamcal/emoji-data/$EMOJI_DATA_COMMIT/emoji.json"
readonly ABR_URL="https://data.address-br.digital.go.jp/mt_town/mt_town_all.csv.zip"
readonly POSTAL_URL="https://www.arcgis.com/sharing/rest/content/items/$ABR_POSTAL_ITEM_ID/data"
readonly ZENZ_URL="https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf/resolve/main/ggml-model-Q5_K_M.gguf"

# sha256（ダウンロードしたファイルと、展開したあとのファイル。2026-10-12 に手元の data/raw/ から取った）
readonly SHA_SMALL_LEX_ZIP="b578ac9545899783d5d7e30d5d78d5d9dcf40b36965d4af0663ec2eb041c1093"
readonly SHA_CORE_LEX_ZIP="a2b39e1572adab08a649b1390b134517adc55f1d733c59358b113298788bf31c"
readonly SHA_NOTCORE_LEX_ZIP="a15edc5193b42acfebdc220825d8c7edf0059bf950169d09bcdf6c426a81ecd1"
readonly SHA_MATRIX_ZIP="3ccaed582481b31be170c7200cf1cb2856f2d14c2fa601b3692d97dbad5a3bd1"
readonly SHA_SMALL_LEX_CSV="6cc54808a2359b48fe15105814ca3352dceb4c25305f6956b7a104f7f6611f00"
readonly SHA_CORE_LEX_CSV="3a3c4b68cf0b64893dff6cc14ba690d79a99fefe1564dd751a418317db2e148d"
readonly SHA_NOTCORE_LEX_CSV="6237df5751fea79e638dba54d088520ec3e2386209971e737c6ecc5d69f24950"
readonly SHA_MATRIX_DEF="b20e0da1ed5472e0be2e5cb3c716672fd788e1701e08d54624b58ba52d73d927"
readonly SHA_CHIVE_TAR="885c7db3b8cd8ad1311ac32eafc874007f45010791b3c1f1e934a2aa0c7d22b0"
readonly SHA_CHIVE_TXT="bd1807016d6bf1813021bc9f12b32e71a2ff1121b1fefe0b1b55ddab345503e0"
readonly SHA_ZENZ_SMALL="29c223d4c23327b80fd13ebb5ab2555057a46317997d5da391584ffbef0db673"
readonly SHA_EMOJI_JSON="8736d721991e48c41c57acc46b65e5d32258a45e4bcdebcc6012f420f81e775d" # 2026-10-18 に取得
readonly SHA_ABR_ZIP="3593307e88b1491cba8dc3b90c224d1c8d4ac54b02bb3532e15ea830ce84d10a" # 2026-10-25 に取得（2026-10-22 の d613bb1c… から更新。辞書は同一: ADR 0030 §14）
readonly SHA_ABR_CSV="9238298a7dfb237e22207b89b3a25d63c4437f7d3ded8db3b509068c7f74ca9a"
readonly SHA_POSTAL_ZIP="45e96e61840f070e2185ae5a4f01c7b8a169d82fb6e1c71dfd25cc8f0fe4b8f8" # 2026-10-23 に取得（郵便番号データ版 2609、更新日 2026-09-24）
readonly SHA_POSTAL_CSV="7d9a97984b25110fb6fdcadd5239970f4d038877ec0121252f69770451a6ce92"
readonly SHA_AJIMEE_ITEMS="e9eb668fd6aa14b1e26436f429b5550108af0a1dfd443b8cea0bcb3ab3028fca"

DATA="$IKARI_ROOT/data"
CHECK_ONLY=0
ONLY=()
MISSING=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --data) DATA="${2:-}"; shift ;;
    --only) ONLY+=("${2:-}"); shift ;;
    --check) CHECK_ONLY=1 ;;
    -h | --help) sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "不明な引数: $1" ;;
  esac
  shift
done
[[ -n "$DATA" ]] || die "--data が空です"
RAW="$DATA/raw"

wanted() { # <名前>
  [[ ${#ONLY[@]} -eq 0 ]] && return 0
  local item
  for item in "${ONLY[@]}"; do [[ "$item" == "$1" ]] && return 0; done
  return 1
}

# 正しいファイルがあるか
has_valid() { # <path> <sha256>
  [[ -f "$1" ]] && [[ "$(sha256_of "$1")" == "$2" ]]
}

download() { # <url> <出力> <sha256>
  local url="$1" dest="$2" want="$3" tmp got
  mkdir -p "$(dirname "$dest")"
  tmp="$(mktemp "$dest.tmp.XXXXXX")"
  log "ダウンロード: $url"
  if ! curl --fail --silent --show-error --location --retry 3 --output "$tmp" "$url"; then
    rm -f "$tmp"
    die "ダウンロードに失敗しました: $url"
  fi
  got="$(sha256_of "$tmp")"
  if [[ "$got" != "$want" ]]; then
    rm -f "$tmp"
    die "sha256 が一致しません: ${url}（期待 ${want}、実際 ${got}）。配布元が更新された可能性があります"
  fi
  mv -f "$tmp" "$dest"
}

# 足りなければ取得する（--check のときは足りないことを報告するだけ）
ensure() { # <path> <sha256> <url>
  if has_valid "$1" "$2"; then
    printf 'ok         %s\n' "$1"
  elif [[ "$CHECK_ONLY" -eq 1 ]]; then
    printf 'MISSING    %s\n' "$1"
    MISSING=1
  else
    download "$3" "$1" "$2"
    printf 'downloaded %s\n' "$1"
  fi
}

fetch_sudachi() {
  local dir="$RAW/$SUDACHI_DIR_NAME" name zip csv sha_zip sha_csv url
  mkdir -p "$dir"
  # 名前、zip の sha256、展開後のファイル名、その sha256、URL
  local rows=(
    "small_lex|$SHA_SMALL_LEX_ZIP|small_lex.csv|$SHA_SMALL_LEX_CSV|$SUDACHI_RAW_URL/$SUDACHI_VERSION/small_lex.zip"
    "core_lex|$SHA_CORE_LEX_ZIP|core_lex.csv|$SHA_CORE_LEX_CSV|$SUDACHI_RAW_URL/$SUDACHI_VERSION/core_lex.zip"
    "notcore_lex|$SHA_NOTCORE_LEX_ZIP|notcore_lex.csv|$SHA_NOTCORE_LEX_CSV|$SUDACHI_RAW_URL/$SUDACHI_VERSION/notcore_lex.zip"
    "matrix.def|$SHA_MATRIX_ZIP|matrix.def|$SHA_MATRIX_DEF|$SUDACHI_RAW_URL/matrix.def.zip"
  )
  local row
  for row in "${rows[@]}"; do
    IFS='|' read -r name sha_zip csv sha_csv url <<<"$row"
    zip="$dir/$name.zip"
    if has_valid "$dir/$csv" "$sha_csv"; then
      printf 'ok         %s\n' "$dir/$csv"
      continue
    fi
    if [[ "$CHECK_ONLY" -eq 1 ]]; then
      printf 'MISSING    %s\n' "$dir/$csv"
      MISSING=1
      continue
    fi
    ensure "$zip" "$sha_zip" "$url"
    log "展開: $zip"
    unzip -o -q "$zip" -d "$dir"
    has_valid "$dir/$csv" "$sha_csv" || die "展開した $csv の sha256 が一致しません"
    printf 'extracted  %s\n' "$dir/$csv"
  done
}

fetch_chive() {
  local dir="$RAW/chive" txt="$RAW/chive/$CHIVE_NAME/$CHIVE_NAME.txt"
  if has_valid "$txt" "$SHA_CHIVE_TXT"; then
    printf 'ok         %s\n' "$txt"
    return
  fi
  if [[ "$CHECK_ONLY" -eq 1 ]]; then
    printf 'MISSING    %s\n' "$txt"
    MISSING=1
    return
  fi
  ensure "$dir/$CHIVE_NAME.tar.gz" "$SHA_CHIVE_TAR" "$CHIVE_URL"
  log "展開: $dir/$CHIVE_NAME.tar.gz"
  tar -xzf "$dir/$CHIVE_NAME.tar.gz" -C "$dir"
  has_valid "$txt" "$SHA_CHIVE_TXT" || die "展開した $CHIVE_NAME.txt の sha256 が一致しません"
  printf 'extracted  %s\n' "$txt"
}

fetch_zenz() {
  ensure "$RAW/zenz/$ZENZ_MODEL_FILE" "$SHA_ZENZ_SMALL" "$ZENZ_URL"
}

fetch_emoji() {
  ensure "$RAW/$EMOJI_DATA_JSON" "$SHA_EMOJI_JSON" "$EMOJI_URL"
}

fetch_abr() {
  local dir="$RAW/$ABR_DIR_NAME" csv="$RAW/$ABR_TOWN_CSV"
  if has_valid "$csv" "$SHA_ABR_CSV"; then
    printf 'ok         %s\n' "$csv"
    return
  fi
  if [[ "$CHECK_ONLY" -eq 1 ]]; then
    printf 'MISSING    %s\n' "$csv"
    MISSING=1
    return
  fi
  ensure "$dir/$(basename "$csv").zip" "$SHA_ABR_ZIP" "$ABR_URL"
  log "展開: $dir/$(basename "$csv").zip"
  unzip -o -q "$dir/$(basename "$csv").zip" -d "$dir"
  has_valid "$csv" "$SHA_ABR_CSV" || die "展開した $(basename "$csv") の sha256 が一致しません"
  printf 'extracted  %s\n' "$csv"
}

fetch_postal() {
  local dir="$RAW/$ABR_DIR_NAME" csv="$RAW/$ABR_POSTAL_CSV" zip="$RAW/$ABR_DIR_NAME/abr_post_code.zip"
  if has_valid "$csv" "$SHA_POSTAL_CSV"; then
    printf 'ok         %s\n' "$csv"
    return
  fi
  if [[ "$CHECK_ONLY" -eq 1 ]]; then
    printf 'MISSING    %s\n' "$csv"
    MISSING=1
    return
  fi
  ensure "$zip" "$SHA_POSTAL_ZIP" "$POSTAL_URL"
  log "展開: $zip"
  unzip -o -q "$zip" -d "$dir"
  has_valid "$csv" "$SHA_POSTAL_CSV" || die "展開した abr_post_code.csv の sha256 が一致しません"
  printf 'extracted  %s\n' "$csv"
}

fetch_ajimee() {
  local items="$RAW/ajimee/evaluation_items.json"
  if has_valid "$items" "$SHA_AJIMEE_ITEMS"; then
    printf 'ok         %s\n' "$items"
  elif [[ "$CHECK_ONLY" -eq 1 ]]; then
    printf 'MISSING    %s\n' "$items"
    MISSING=1
  else
    IKARI_AJIMEE_DIR="$RAW/ajimee" "$IKARI_ROOT/eval/scripts/fetch_ajimee.sh"
    has_valid "$items" "$SHA_AJIMEE_ITEMS" || die "AJIMEE の evaluation_items.json の sha256 が一致しません"
  fi
}

printf 'data: %s\n' "$DATA"
if ! wanted sudachi && ! wanted chive && ! wanted zenz && ! wanted ajimee && ! wanted emoji && ! wanted abr && ! wanted postal; then
  die "--only は sudachi / chive / zenz / ajimee / emoji / abr / postal のどれかです"
fi
if [[ "$CHECK_ONLY" -eq 0 ]]; then
  command -v curl >/dev/null || die "curl がありません"
fi
if wanted sudachi; then fetch_sudachi; fi
if wanted chive; then fetch_chive; fi
if wanted zenz; then fetch_zenz; fi
if wanted emoji; then fetch_emoji; fi
if wanted abr; then fetch_abr; fi
if wanted postal; then fetch_postal; fi
if wanted ajimee; then fetch_ajimee; fi
exit "$MISSING"
