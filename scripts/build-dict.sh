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
#
# 中身は `cargo xtask dict`（xtask/src/dict.rs。ADR 0043 段 X2）に移した。ここは呼ぶだけの包み。
set -euo pipefail

# 呼び出し元のディレクトリのまま実行する（相対パスの引数が変わらない）
exec cargo run --quiet --manifest-path "$(dirname "${BASH_SOURCE[0]}")/../Cargo.toml" --package xtask -- dict "$@"
