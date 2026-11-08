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

### fst

- **ライセンス**: Unlicense or MIT
- **バージョン**: 0.4.7
- **用途**: 索引データ構造（有限状態トランスデューサー）
- **リポジトリ**: https://github.com/BurntSushi/fst

### memmap2

- **ライセンス**: MIT or Apache-2.0
- **バージョン**: 0.9
- **用途**: メモリ効率的なファイルマッピングによる辞書の読み込み
- **リポジトリ**: https://github.com/RazrFalcon/memmap2-rs

### clap

- **ライセンス**: MIT or Apache-2.0
- **バージョン**: 4.6（derive feature）
- **用途**: コマンドラインオプションの解析（CLI、dict-builder、eval用）
- **リポジトリ**: https://github.com/clap-rs/clap

### tempfile

- **ライセンス**: MIT or Apache-2.0
- **バージョン**: 3
- **用途**: テスト時の一時ディレクトリ・ファイル管理（dev-dependency）
- **リポジトリ**: https://github.com/Stebalien/tempfile

### serde / serde_json

- **ライセンス**: MIT or Apache-2.0
- **バージョン**: serde 1.0、serde_json 1.0
- **用途**: 取り込みファイル（JSONL）の読み込み（import crate）
- **リポジトリ**: https://github.com/serde-rs/serde 、https://github.com/serde-rs/json

### jiff

- **ライセンス**: Unlicense or MIT
- **バージョン**: 0.2
- **用途**: Rewriter（日付や時刻の展開）の現在時刻の取得
- **リポジトリ**: https://github.com/BurntSushi/jiff

### bzip2 / flate2 / regex / sha2

- **ライセンス**: bzip2、flate2、regex、sha2 はいずれも MIT or Apache-2.0。bzip2 が内部で使う libbz2-rs-sys（bzip2 の Rust 実装）は bzip2-1.0.6 ライセンス（BSD 系）
- **バージョン**: bzip2 0.6（libbz2-rs-sys 0.2）、flate2 1、regex 1、sha2 0.10
- **用途**: 辞書の元データを作る道具（dict-builder の `webwords`）が、Wikipedia のダンプ（bz2、gzip）を読み、本文から語を取り出し、入力の sha256 を取る。bzip2、regex、sha2 は製品のバイナリ（アプリ、エンジン）には含まれない。flate2 はエンジンのログの gzip 圧縮にも使う
- **リポジトリ**: https://github.com/trifectatechfoundation/bzip2-rs 、https://github.com/trifectatechfoundation/libbz2-rs-sys 、https://github.com/rust-lang/flate2-rs 、https://github.com/rust-lang/regex 、https://github.com/RustCrypto/hashes

### prost / prost-build / protox

- **ライセンス**: prost、prost-build は Apache-2.0。protox は MIT or Apache-2.0
- **バージョン**: 0.14（prost、prost-build）、0.10（protox）
- **用途**: 殻とエンジンの間のメッセージ（protobuf）の Rust 側の符号化。`.proto` のコンパイルはビルド時に protox で行う（`protoc` は不要）
- **リポジトリ**: https://github.com/tokio-rs/prost 、https://github.com/andrewhickman/protox

## 言語モデルと推論エンジン（フェーズ2のエンジンが使用）

### llama.cpp

- **ライセンス**: MIT（Copyright (c) 2023-2026 The ggml authors）
- **用途**: 言語モデルの推論。Rust クレート `llama-cpp-2` / `llama-cpp-sys-2`（0.1.159。MIT or Apache-2.0）経由で、エンジン（`IkariEngine.xpc`）の静的ライブラリに含まれる。Metal のシェーダは埋め込みで、ネットワークは使わない
- **リポジトリ**: https://github.com/ggml-org/llama.cpp

### zenz-v3.2 small

- **ライセンス**: CC BY-SA 4.0（Hugging Face のモデルページ https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf の表示とモデルカードの `license: cc-by-sa-4.0` による。2026-11-09 に確認。2026-10-26 には Apache-2.0 と書いていたが、現在の表示は CC BY-SA 4.0 で、最終更新は 2026-11-07）。ikari はモデルを配らず、利用者が取得する。モデルを配る場合は、表示（作者、ライセンス、変更の有無）と同じ条件での提供（ShareAlike）の及ぶ範囲を、先に確かめる（#242）。モデルのページには、元になったモデルや学習データのライセンスの記載がない
- **用途**: 候補の再ランキング（既定の唯一の LM）。モデルファイルはバンドルに含めず、`scripts/install-data.sh` が `~/Library/Application Support/ikari-ime/models/` に置く

### kanakanji-edge

- **ライセンス**: 重みは Apache-2.0、語彙は ku-nlp の CC BY-SA 4.0（`eval/reports/phase1-lm-compare.md` の記載による）
- **用途**: 比較実験用の代替 LM（`model_kind = kanakanji-edge`）。既定では使わず、バンドルにも含めない

## Swift 依存パッケージ（macOS の殻とエンジンのサービス）

### swift-protobuf

- **ライセンス**: Apache License 2.0
- **バージョン**: 1.38.x
- **用途**: 殻とエンジンの間のメッセージ（protobuf）の符号化
- **リポジトリ**: https://github.com/apple/swift-protobuf

## ライセンス表示義務

ikari-ime のコードは Apache License 2.0 です（[LICENSE](LICENSE)、[NOTICE](NOTICE)）。辞書、モデル、地名のデータはこのリポジトリに含めません。各自が取得し、取得元の条件に従います。
自分で学習したアダプタなどを他人に配るときは、元のモデルと学習データの条件を確かめて、その条件に従ってください（zenz のモデルページには、元になったモデルと学習データのライセンスの記載がありません）。

## ライセンス確認

各クレートのライセンスは、[Cargo.lock](Cargo.lock) および Rust クレートレジストリで確認できます。
