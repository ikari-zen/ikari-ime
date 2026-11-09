# サードパーティーライセンス

このプロジェクトは、以下のサードパーティーソフトウェアを使用しています。

## 辞書データ

### SudachiDict Core

- **ライセンス**: Apache License 2.0
- **著作権**: WorksApplications
- **説明**: 日本語形態素解析器Sudachi用の辞書。このプロジェクトでは、バージョン 20260723 のSmall LexとCore Lexを使用しています。
- **出典**: http://sudachi.s3-website-ap-northeast-1.amazonaws.com/sudachidict-raw/
- **使用方法**: 初期セットアップで `data/raw/sudachi-20260723/` に配置したCSVファイル（small_lex.csv、core_lex.csv、matrix.def）を、dict-builderで処理してバイナリ辞書に変換します。バイナリ辞書自体（grammar.bin、system.dict）はプロジェクトに同梱されません。使用時に`data/raw/sudachi-20260723/`から毎回生成されます。

詳細は [testdata/sudachi-mini/README.md](testdata/sudachi-mini/README.md) をご覧ください。

#### SudachiDict に含まれるデータの表示（UniDic、NEologd）

SudachiDict の LEGAL（https://github.com/WorksApplications/SudachiDict/blob/develop/LEGAL 。2026-11-10 に取得）によると、`small_lex.csv` と `matrix.def` は UniDic（BSD-3-Clause。The UniDic Consortium）の一部、`core_lex.csv` と `notcore_lex.csv` は NEologd（Toshinori Sato）の一部を含みます。ikari はこれらのファイルから辞書を作るので、LEGAL の表示を次に写します（原文のまま）。

```text
LEGAL NOTICE INFORMATION
========================

- All the files in this distribution are covered under the Apache License
version 2.0. (see the file LICENSE-2.0.txt)


- src/main/text/small_lex.csv contains a part
  of UniDic (https://unidic.ninjal.ac.jp/).
- src/main/text/matrix.def.zip is a part of UniDic.

Copyright (c) 2011-2013, The UniDic Consortium
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

 * Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.

 * Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the
   distribution.

 * Neither the name of the UniDic Consortium nor the names of its
   contributors may be used to endorse or promote products derived
   from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.


- src/main/text/core_lex.csv and src/main/text/notcore_lex.csv contain
a part of NEologd (https://github.com/neologd/mecab-unidic-neologd).

Copyright (C) 2015-2019 Toshinori Sato (@overlast)

      https://github.com/neologd/mecab-unidic-neologd

    i. 本データは、株式会社はてなが提供するはてなキーワード一覧ファイル
       中の表記、及び、読み仮名の大半を使用している。

       はてなキーワード一覧ファイルの著作権は、株式会社はてなにある。

       はてなキーワード一覧ファイルの使用条件に基づき、また、
       データ使用の許可を頂いたことに対する感謝の意を込めて、
       以下に株式会社はてなおよびはてなキーワードへの参照をURLで示す。

       株式会社はてな : http://hatenacorp.jp/information/outline

       はてなキーワード :
       http://developer.hatena.ne.jp/ja/documents/keyword/misc/catalog

   ii. 本データは、日本郵便株式会社が提供する郵便番号データ中の表記、
       及び、読み仮名を使用している。

       日本郵便株式会社は、郵便番号データに限っては著作権を主張しないと
       述べている。

       日本郵便株式会社の郵便番号データに対する感謝の意を込めて、
       以下に日本郵便株式会社および郵便番号データへの参照をURLで示す。

       日本郵便株式会社 :
         http://www.post.japanpost.jp/about/profile.html

       郵便番号データ :
         http://www.post.japanpost.jp/zipcode/dl/readme.html

  iii. 本データは、スナフキん氏が提供する日本全国駅名一覧中の表記、及び
       読み仮名を使用している。

       日本全国駅名一覧の著作権は、スナフキん氏にある。

       スナフキん氏は 「このデータを利用されるのは自由ですが、その際に
       不利益を被ったりした場合でも、スナフキんは一切責任は負えません
       ことをご承知おき下さい」と述べている。

       スナフキん氏に対する感謝の意を込めて、
       以下に日本全国駅名一覧のコーナーへの参照をURLで示す。

       日本全国駅名一覧のコーナー :
         http://www5a.biglobe.ne.jp/~harako/data/station.htm

   iv. 本データは、工藤拓氏が提供する人名(姓/名)エントリデータ中の、
       漢字表記の姓・名とそれに対応する読み仮名を使用している。

       人名(姓/名)エントリデータは被災者・安否不明者の人名の
       表記揺れ対策として、Mozcの人名辞書を活用できるという
       工藤氏の考えによって提供されている。

       工藤氏に対する感謝の意を込めて、
       以下にデータ本体と経緯が分かる情報への参照をURLで示す。

       人名(姓/名)エントリデータ :
         http://chasen.org/~taku/software/misc/personal_name.zip

       上記データが提供されることになった経緯
         http://togetter.com/li/111529

    v. 本データは、Web上からクロールした大量の文書データから抽出した
       表記とそれに対応する読み仮名のデータを含んでいる。

       抽出した表記とそれに対応する読み仮名の組は、上記の i. から iv.
       の言語資源の組み合わせによって得られる組のみを採録した。

       Web 上に文書データを公開して下さっている皆様に感謝いたします。

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

      http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

### chiVe

- **ライセンス**: Apache License 2.0
- **著作権**: Works Applications Co., Ltd.
- **説明**: Sudachi の単語分割で学習した日本語の単語ベクトル（v1.3 mc90）。語の頻度順位（ポリシーの `lemma_rank`）と、話題スコア用の `vectors.bin` の元になる
- **出典**: https://sudachi.s3.ap-northeast-1.amazonaws.com/chive/chive-1.3-mc90.tar.gz（`scripts/fetch-data.sh` が取得）
- **使用方法**: `data/raw/chive/` に置き、dict-builder が `vectors.bin`（語ごとの 300 次元ベクトルを量子化したもの）を作る。`vectors.bin` は `~/Library/Application Support/ikari-ime/dict/` に置かれる（バンドルには含めない）。chiVe 本体は同梱しない

### アドレス・ベース・レジストリ 町字マスター（デジタル庁）

- **ライセンス**: 公共データ利用規約（第1.0版。PDL1.0）。出典の記載と、編集・加工したことおよびその主体の記載が要る
- **出典**: 「アドレス・ベース・レジストリ 町字マスター」（デジタル庁）（https://data.address-br.digital.go.jp/mt_town/mt_town_all.csv.zip）。利用規約: アドレス・ベース・レジストリの利用規約（デジタル庁。最終更新 2026-05-29）
- **加工**: 「アドレス・ベース・レジストリ 町字マスター」（デジタル庁）をもとに ikari-ime が加工して作成した。国が作成した未加工のデータではない。町字マスターの大字・町名（丁目の行の大字・町名を含む。小字と京都の通り名は使わない）の表記と読みから、「大字」「字」の接頭辞の除去、読みの小書きの修正（「チヨウ」→「チョウ」など）、数字・括弧・異体字などを含む表記と、読みが表記に対応しない行の除外、SudachiDict にある語の除外を行い、地名の語彙（読みと表記の組）にして変換辞書に加えている。あわせて、同じ町字マスターの都道府県・郡・市区町村・区の名と大字・町名をつないだ住所の表記と読み（「東京都千代田区千代田」など）も、変換辞書に加えている（ADR 0030）
- **使用方法**: `scripts/fetch-data.sh --only abr` で `data/raw/abr/` に置き、dict-builder の `town` が語彙にして辞書（`system.dict`）に加える。町字マスター本体は同梱しない。辞書は `~/Library/Application Support/ikari-ime/dict/` に置かれる（バンドルには含めない）

### アドレス・ベース・レジストリ 町字・郵便番号変換表（デジタル庁。元は日本郵便の郵便番号データ）

- **ライセンス**: 公共データ利用規約（第1.0版。PDL1.0）。出典の記載と、編集・加工したことおよびその主体の記載が要る。変換表自体の licenseInfo は空で、説明文は ABR の利用規約に従うとある。変換表は日本郵便の郵便番号データをもとにしたものである（再配布の可否は確かめていない。ikari-ime は表を配らず、利用者が取得する）
- **出典**: 「アドレス・ベース・レジストリ 町字・郵便番号変換表」（デジタル庁）（カタログ https://dataset.address-br.digital.go.jp/ の項目 `1098a4f61dab4f52997839d04245132a`。取得は https://www.arcgis.com/sharing/rest/content/items/1098a4f61dab4f52997839d04245132a/data）。郵便番号データの版 2609（更新日 2026-09-24）、取得日 2026-10-23。利用規約: アドレス・ベース・レジストリの利用規約（デジタル庁）
- **加工**: 「アドレス・ベース・レジストリ 町字・郵便番号変換表」（デジタル庁。元は日本郵便の郵便番号データ）をもとに ikari-ime が加工して作成した。国や日本郵便が作成した未加工のデータではない。変換表の各行を町字マスターと (lg_code, machiaza_id) で結び、大字・町名に畳み（丁目と小字の違いは消える）、上の町字マスターと同じ方法で読みを付け、都道府県つき・郡つき、都道府県や郡を省いた形ごとに住所の表記と読み、郵便番号を並べた表（`postal.tsv`）にしている。廃止された町字の行は使わない。町名に畳めない行（表記に数字を含む町など）は表に入れない（ADR 0032）
- **使用方法**: `scripts/fetch-data.sh --only postal` で `data/raw/abr/` に置き、dict-builder の `town` が `postal.tsv` を作る。`postal.tsv` は `~/Library/Application Support/ikari-ime/dict/` に置かれる（バンドルには含めない）。変換表本体は同梱しない

### Wikipedia 日本語版（外来語の語と出現頻度の一覧）

- **ライセンス**: クリエイティブ・コモンズ 表示-継承 4.0 国際（CC BY-SA 4.0）。https://creativecommons.org/licenses/by-sa/4.0/deed.ja
- **出典**: ウィキペディア日本語版（https://ja.wikipedia.org/）の執筆者。データベースダンプ jawiki-<日付>（pages-articles、page、redirect、page_props、categorylinks、linktarget。https://dumps.wikimedia.org/jawiki/<日付>/）。各記事の執筆者は、各記事の履歴を参照
- **加工**: ikari-ime が、上のダンプの記事の本文から片仮名の連なりを取り出し、語ごとの出現回数と出現した記事の数を数え、記事の有無とカテゴリから普通名詞と固有名詞（人名・地名・一般）の別を付け、辞書にある語と迷う語を除いた一覧（`webwords-<日付>.tsv`）を作成した。一覧が含むのは、語（表記）、出現回数、記事の数、区分だけである。記事の本文、文、記事の名前、カテゴリの名前は含まない
- **使用方法**: 一覧は ikari-ime の GitHub の Release に、CC BY-SA 4.0 の表示つきで置く。`scripts/fetch-data.sh --only webwords` で `data/raw/webwords/` に置き、dict-builder の `webwords lex` が利用者の手元で辞書（`system.dict`）に語を加える。Wikipedia の本文は同梱しない。辞書は `~/Library/Application Support/ikari-ime/dict/` に置かれる（バンドルには含めない）

### emoji-data（iamcal/emoji-data）

- **ライセンス**: MIT License
- **著作権**: Copyright (c) 2013 Cal Henderson
- **説明**: 絵文字の名前（short_names。Slack と同じ名前）と Unicode の対応表。ショートコード（`:eyes:` → 👀）の元になる（ADR 0027）
- **出典**: https://github.com/iamcal/emoji-data のコミット `13ee711e222ea17fe537bfea953c687866f16411` の `emoji.json`（`scripts/fetch-data.sh --only emoji` が取得。画像は取らない）
- **使用方法**: `data/raw/emoji-data/` に置き、dict-builder が名前と絵文字だけを取り出して `emoji.tsv` を作る。`emoji.tsv` は `~/Library/Application Support/ikari-ime/dict/` に置かれる（バンドルには含めない）。emoji-data 本体は同梱しない

```
The MIT License (MIT)

Copyright (c) 2013 Cal Henderson

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Rust依存クレート

配るバイナリ（ikari.app に入る Rust の成果物 `ikari-engine-ffi`、`ikari-shell-ffi`）が静的に含む依存クレートのライセンスと著作権表示は、[THIRD_PARTY_LICENSES_RUST.md](THIRD_PARTY_LICENSES_RUST.md) にまとめてあります。cargo-about で機械的に作った一覧で（`cargo xtask licenses`。設定は [about.toml](about.toml)）、許可しているライセンス（Apache-2.0、MIT、Unicode-3.0）以外が入ると作れません。設定アプリの「ライセンス」画面には、このファイルのあとに続けて出ます。

辞書を作る道具（dict-builder の `webwords` が使う bzip2、regex、sha2 など）、評価、CLI、xtask は、配るバイナリに含まれないので、一覧に入れていません。

## 言語モデルと推論エンジン（フェーズ2のエンジンが使用）

### llama.cpp

- **ライセンス**: MIT（Copyright (c) 2023-2026 The ggml authors）
- **用途**: 言語モデルの推論。Rust クレート `llama-cpp-2` / `llama-cpp-sys-2`（0.1.160。MIT or Apache-2.0）経由で、エンジン（`IkariEngine.xpc`）の静的ライブラリに含まれる。Metal のシェーダは埋め込みで、ネットワークは使わない
- **リポジトリ**: https://github.com/ggml-org/llama.cpp
- **許諾文（全文。llama-cpp-sys-2 に入っている llama.cpp の `LICENSE`）**:

```text
MIT License

Copyright (c) 2023-2026 The ggml authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### zenz-v3.2 small

- **ライセンス**: CC BY-SA 4.0（Hugging Face のモデルページ https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf の表示とモデルカードの `license: cc-by-sa-4.0` による。2026-11-09 に確認。2026-10-26 には Apache-2.0 と書いていたが、現在の表示は CC BY-SA 4.0 で、最終更新は 2026-11-07）。ikari はモデルを配らず、利用者が取得する。モデルを配る場合は、表示（作者、ライセンス、変更の有無）と同じ条件での提供（ShareAlike）の及ぶ範囲を、先に確かめる（#242）。モデルのページには、元になったモデルや学習データのライセンスの記載がない
- **用途**: 候補の再ランキング（既定の唯一の LM）。モデルファイルはバンドルに含めず、`scripts/install-data.sh` が `~/Library/Application Support/ikari-ime/models/` に置く

### kanakanji-edge

- **ライセンス**: 重みは Apache-2.0、語彙は ku-nlp の CC BY-SA 4.0（`eval/reports/phase1-lm-compare.md` の記載による）
- **用途**: 比較実験用の代替 LM（`model_kind = kanakanji-edge`）。既定では使わず、バンドルにも含めない

## Swift 依存パッケージ（macOS の殻とエンジンのサービス）

### swift-protobuf

- **ライセンス**: Apache License 2.0
- **バージョン**: 1.39.0（`macos/Package.resolved`）
- **著作権**: Apple Inc. and the Swift Project authors
- **許諾文**: Apache License 2.0 の全文は [THIRD_PARTY_LICENSES_RUST.md](THIRD_PARTY_LICENSES_RUST.md) の Apache-2.0 の項（同じ条文）と [LICENSE](LICENSE) にある
- **用途**: 殻とエンジンの間のメッセージ（protobuf）の符号化
- **リポジトリ**: https://github.com/apple/swift-protobuf

## ライセンス表示義務

ikari-ime のコードは Apache License 2.0 です（[LICENSE](LICENSE)、[NOTICE](NOTICE)）。辞書、モデル、地名のデータはこのリポジトリに含めません。各自が取得し、取得元の条件に従います。
自分で学習したアダプタなどを他人に配るときは、元のモデルと学習データの条件を確かめて、その条件に従ってください（zenz のモデルページには、元になったモデルと学習データのライセンスの記載がありません）。

## ライセンス確認

Rust 依存の一覧は、[Cargo.lock](Cargo.lock) が変わったあとに `cargo xtask licenses` で作り直します（cargo-about が要る: `cargo install cargo-about --locked --features cli`）。最新かどうかは `cargo xtask licenses --check` で確かめられ、`scripts/check.sh` も流します（cargo-about がなければ飛ばして警告します）。
