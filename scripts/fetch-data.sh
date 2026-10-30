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
#
# 中身は `cargo xtask data fetch`（xtask/src/data/fetch.rs。ADR 0043 段 X3）に移した。ここは呼ぶだけの包み。
set -euo pipefail

# 呼び出し元のディレクトリのまま実行する（相対パスの引数が変わらない）
exec cargo run --quiet --manifest-path "$(dirname "${BASH_SOURCE[0]}")/../Cargo.toml" --package xtask -- data fetch "$@"
