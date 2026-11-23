# ビルド済み配布に向けた辞書・モデル・一覧のライセンスと再配布の調査（2026-11）

- 状態: 調査メモ（#242 の回答。**法的助言ではない**。外部への問い合わせはしていない）
- 確認日: 2026-11-22
- 関連: #242、#490、#162、#99、#226、[ADR 0055](../adr/0055-web-words-classify-and-build.md)、[ADR 0060](../adr/0060-user-selectable-lm-model.md)

## 0. 目的と前提（2026-11-19 ユーザー決定）

配布物を「各自が元データを取得してビルドする」形から、**ビルド済みの辞書・公式のままのモデルを配布物に同梱する**形（通常版および通信を持たないオフライン版。#490）へ移すにあたり、各データのライセンス・再配布の可否・表示義務・ShareAlike の及ぶ範囲・`#162` へ残す対象を一次資料にもとづいて整理する。

前提となるユーザー決定（2026-11-19。[ADR 0060](../adr/0060-user-selectable-lm-model.md) 更新および #242 コメント）:

1. 通常版・オフライン版とも、**公式のままの `zenz-v3.2-small`（無改変、sha256 固定）** とデプロイ時点の辞書を配布物に同梱する方針。
2. `zenz` の作者への問い合わせは行わない。使えなくなった場合（表示変更・指摘等）はその時点で対応を考える。
3. 返事を待たずにできる備え（Hugging Face のコミットと `README.md` 原文の記録、sha256 の固定、確認日つきの NOTICE 表示）を行い、残るリスク（元モデルと学習データの条件不明、実行時利用が翻案に当たるかの解釈）は受け入れて進める。

---

## 1. 配布予定データのライセンス・再配布一覧表

確認日: 2026-11-22（一次資料の取得・照合による。法的な適法性を保証するものではない）。

| データ | 配布物での形 | ライセンス | 再配布の可否 | 表示などの義務 | 根拠の URL（確認日: 2026-11-22） |
| --- | --- | --- | --- | --- | --- |
| **zenz-v3.2-small** | `models/zenz-v3.2-small-Q5_K_M.gguf`（公式ファイルを無改変で同梱） | CC BY-SA 4.0 | 可（条件つき） | 作者表示（Keita Miwa）、ライセンス名と URL、取得元 URL、変更なし（公式のまま）、無保証の言及、追加制限の禁止。改変・再配布時は ShareAlike（§2.2・§2.3） | [HF モデルページ](https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf)（commit `ef3e66e3665fe6a19598e6578e9b88a994b647fd`）、[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)、[Legal Code](https://creativecommons.org/licenses/by-sa/4.0/legalcode.txt) |
| *（参考）ku-nlp/gpt2-small-japanese-char* | v2.5 の元モデル（v3.2 での使用は未記載・未確認） | CC BY-SA 4.0 | 可（条件つき） | v3.2 への引き継ぎは不明。参考情報として注記するか、記載がない事実を明記する（§2.4） | [ku-nlp/gpt2-small-japanese-char](https://huggingface.co/ku-nlp/gpt2-small-japanese-char)、[zenz-v2.5-small](https://huggingface.co/Miwa-Keita/zenz-v2.5-small) |
| **SudachiDict Full 20260723** | `small_lex.csv` / `core_lex.csv` / `notcore_lex.csv` / `matrix.def` → `dict/grammar.bin`・`dict/system.dict` | Apache-2.0（`LEGAL` に UniDic BSD-3-Clause、NEologd Apache-2.0 + 5 出典付記を含む） | 可 | Apache-2.0 本文の同梱、著作権表示（`Copyright (c) 2017-2023 Works Applications Co., Ltd.`）、改変の明記（コスト調整・バイナリ変換）、`LEGAL`（UniDic BSD-3-Clause 全文・NEologd 出典表示）の保持 | [SudachiDict](https://github.com/WorksApplications/SudachiDict)（develop `830bf9e1…`）、[LICENSE-2.0.txt](https://github.com/WorksApplications/SudachiDict/blob/develop/LICENSE-2.0.txt)、[LEGAL](https://github.com/WorksApplications/SudachiDict/blob/develop/LEGAL)、[配布元](http://sudachi.s3-website-ap-northeast-1.amazonaws.com/sudachidict-raw/) |
| **chiVe 1.3 mc90** | `chive-1.3-mc90.txt` → `dict/vectors.bin` および `system.dict` 構築時の頻度順位 | Apache-2.0 | 可 | Apache-2.0 本文の同梱、著作権表示（`Copyright (c) 2024 Works Applications Co., Ltd. All rights reserved.`）、改変の明記（300 次元ベクトルの量子化・頻度利用） | [chiVe](https://github.com/WorksApplications/chiVe)（master `0cb4961e…`）、[LICENSE](https://github.com/WorksApplications/chiVe/blob/master/LICENSE)、[配布元](https://sudachi.s3-ap-northeast-1.amazonaws.com/chive/chive-1.3-mc90.tar.gz) |
| **ABR 町字マスター（デジタル庁）** | `mt_town_all.csv` → `system.dict` の大字・町名と住所の連なり | 公共データ利用規約（第1.0版 / PDL1.0。CC BY 4.0 互換） | 可 | 出典の記載、編集・加工したことおよびその主体（ikari-ime）の記載。国等が作成した未加工データに見せない | [配布元 zip](https://data.address-br.digital.go.jp/mt_town/mt_town_all.csv.zip)、[ABR 利用規約](https://www.digital.go.jp/policies/base_registry_address_tos)（最終更新 2026-05-29）、[PDL1.0](https://www.digital.go.jp/resources/open_data/public_data_license_v1.0) |
| **ABR 町字・郵便番号変換表（デジタル庁・元は日本郵便）** | `abr_post_code.csv` → `dict/postal.tsv` | デジタル庁 ABR 利用規約（PDL1.0）＋日本郵便「著作権を主張しない」 | 可（条件つき。参考情報・無保証・継続提供未保証の前提） | PDL1.0 の出典・加工・主体の記載（国の未加工データに見せない）。日本郵便データの出典も併記（§4） | [ArcGIS メタデータ](https://www.arcgis.com/sharing/rest/content/items/1098a4f61dab4f52997839d04245132a?f=json)、[ABR 利用規約](https://www.digital.go.jp/policies/base_registry_address_tos)、[日本郵便 説明](https://www.post.japanpost.jp/service/search/zipcode/download/readme.html) |
| **Wikipedia 日本語版由来 外来語の一覧（`webwords`）** | GitHub Release（`webwords-20261101.tsv.xz`）および `system.dict` への組み込み | CC BY-SA 4.0 | 一覧単体は可（条件つき）。`system.dict` へ混ぜて同梱する場合の SA 範囲は §3 で要選択 | 出典（ウィキペディア日本語版の執筆者、ダンプ版）、CC BY-SA 4.0 と URL、加工の明記（本文を含まない）。`system.dict` に混ぜる場合は §3 の案 A / B に従う | [jawiki-20261101](https://dumps.wikimedia.org/jawiki/20261101/)、[Wikimedia 利用規約](https://foundation.wikimedia.org/wiki/Policy:Terms_of_Use)、[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) |
| **emoji-data（iamcal/emoji-data）** | `emoji.json`（commit `13ee711e…`）→ `dict/emoji.tsv`（画像なし） | MIT License | 可 | 著作権表示（`Copyright (c) 2013 Cal Henderson`）および MIT 許諾文全文の添付（記載済み） | [iamcal/emoji-data](https://github.com/iamcal/emoji-data)、[LICENSE](https://raw.githubusercontent.com/iamcal/emoji-data/13ee711e222ea17fe537bfea953c687866f16411/LICENSE) |

### 1.1 配布物に同梱しないデータ・自作データ

| データ | 状態 | ライセンス | 扱い |
| --- | --- | --- | --- |
| **kanakanji-edge** | 比較実験用 | 重み Apache-2.0 / 語彙 ku-nlp CC BY-SA 4.0（[THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md) の記載） | 配布物に同梱しない |
| **AJIMEE-Bench** | 評価専用 | CC BY-SA 3.0（[fetch.rs](../../xtask/src/data/fetch.rs) の記載） | 評価専用。リポジトリにも配布物にも入れない |
| **ikari 自作データ**（`core/assets/*`、`dict-builder/policy/*`、`dict-builder/webwords/categories.tsv`） | ビルド埋め込み・リポジトリ管理 | Apache-2.0（[LICENSE](../../LICENSE)） | ikari 本体の一部として Apache-2.0 で配布 |

---

## 2. 確認項目への回答: `zenz-v3.2-small` を公式のまま配布物に同梱する場合

### 2.1 返事を待たずにできる備えの記録（2026-11-19 決定）

2026-11-22 に Hugging Face API および raw URL から確認・固定した事実:

- リポジトリ: `https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf`
- コミット履歴（`commits/main` 全 3 件）:
  1. `ef3e66e3665fe6a19598e6578e9b88a994b647fd`（`2026-11-07T07:48:24.000Z`）: `Update README.md`（author: `Miwa-Keita`）
  2. `c67e03e07d215c869f591b274c1631170d3e11fe`（`2026-05-06T08:44:12.000Z`）: `Upload ggml-model-Q5_K_M.gguf`
  3. `685fe1599bab198204abcc417e4bdc9ab09c74cc`（`2026-05-06T08:41:41.000Z`）: `initial commit`
- 確認時点の `README.md` 原文（29 バイト、全文）:

  ```markdown
  ---
  license: cc-by-sa-4.0
  ---
  ```

- 対象ファイル `ggml-model-Q5_K_M.gguf`:
  - `lfs.sha256`: `29c223d4c23327b80fd13ebb5ab2555057a46317997d5da391584ffbef0db673`
  - `size`: `73871936` バイト
  - 上記 2 点は、[resource/src/models.rs](../../resource/src/models.rs) の `ZENZ_SMALL_SHA256` / `ZENZ_SMALL_SIZE`、および [xtask/src/data/fetch.rs](../../xtask/src/data/fetch.rs) の `SHA_ZENZ_SMALL` と一致している。

### 2.2 CC BY-SA 4.0 の表示要件（内容・場所・変更の有無）

CC BY-SA 4.0 Legal Code Section 3(a)(1)〜(2) および Section 2(a)(5)(C) にもとづく要件:

1. **表示する内容**（提供者が供給した情報がある場合その保持が必須。無い場合は合理的範囲で記載）:
   - 作者・帰属先（Section 3(a)(1)(A)(i)）: `Keita Miwa`（Hugging Face 上の公開アカウント名 `Miwa-Keita`）
   - 著作権表示・無保証（Section 3(a)(1)(A)(ii), (iv)）: 原文の無保証・責任制限への言及（Section 5 の趣旨）
   - ライセンス言及とリンク（Section 3(a)(1)(A)(iii), Section 3(a)(1)(C)）: `CC BY-SA 4.0`（`https://creativecommons.org/licenses/by-sa/4.0/` および日本語概要 `https://creativecommons.org/licenses/by-sa/4.0/deed.ja`）
   - 元の著作物への URI（Section 3(a)(1)(A)(v)）: `https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf`（確認時点のコミットハッシュも併記）
   - 変更の有無の表示（Section 3(a)(1)(B)）: 公式の GGUF を無改変（sha256 一致）で同梱するため、**「変更なし（公式のまま。配布元ファイル名 `ggml-model-Q5_K_M.gguf` を `zenz-v3.2-small-Q5_K_M.gguf` として同梱）」** と明記する。
   - 追加制限の禁止（Section 2(a)(5)(C)）: モデルファイルに対して追加の制限や DRM 等の技術的保護手段を課さない。
2. **表示する場所**（Section 3(a)(2)「媒体・手段・文脈に応じた合理的な方法」）:
   - [THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md) に、ikari 本体（Apache-2.0）とは独立した節を設け、「このライセンスはモデルファイルだけに適用され、ikari 本体には及ばない」旨を明記する。
   - **macOS**: [macos/scripts/build.sh](../../macos/scripts/build.sh) が `THIRD_PARTY_NOTICES.md`（と `THIRD_PARTY_LICENSES_RUST.md` の結合版）を設定アプリ `IkariSettings.app/Contents/Resources/THIRD_PARTY_NOTICES.md` に同梱し、設定アプリの「このソフトについて（ライセンス）」画面で表示する。
   - **Windows**: 現時点の [windows/installer/ikari.iss](../../windows/installer/ikari.iss)（118 行目）は `THIRD_PARTY_LICENSES_RUST.md` のみを配置しており、`THIRD_PARTY_NOTICES.md` を同梱していない。モデル・辞書の同梱を実装する際（#490 / #99）は、`THIRD_PARTY_NOTICES.md` も必ずインストーラの配布物（および設定画面）へ含める必要がある。
3. **同梱実装時（#490）に `THIRD_PARTY_NOTICES.md` へ入れる表示文面案**:

```markdown
### zenz-v3.2 small（モデルファイル単体）

- **適用対象**: 同梱するモデルファイル `zenz-v3.2-small-Q5_K_M.gguf`（この節のライセンス条件はこのモデルファイルにのみ適用され、ikari-ime 本体のソフトウェア（Apache License 2.0）には適用されません）
- **作者**: Keita Miwa（Hugging Face: Miwa-Keita）
- **ライセンス**: クリエイティブ・コモンズ 表示-継承 4.0 国際（CC BY-SA 4.0。https://creativecommons.org/licenses/by-sa/4.0/ / 日本語: https://creativecommons.org/licenses/by-sa/4.0/deed.ja ）
- **出典**: https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf （確認日: 2026-11-22、commit `ef3e66e3665fe6a19598e6578e9b88a994b647fd`、元ファイル名 `ggml-model-Q5_K_M.gguf`、sha256 `29c223d4c23327b80fd13ebb5ab2555057a46317997d5da391584ffbef0db673`）
- **変更の有無**: 変更なし（配布元のファイルを無改変のまま同梱）
- **無保証**: 本モデルはライセンスに基づき「現状のまま（AS-IS）」で提供され、作者および配布者による保証はありません（CC BY-SA 4.0 Section 5）。
- **注記**: 配布元のモデルページには元になったモデルおよび学習データの記載がありません（参考として、同一作者の `zenz-v2.5-small` では元モデルに `ku-nlp/gpt2-small-japanese-char`（CC BY-SA 4.0）が明記されています）。
```

### 2.3 ShareAlike が及ぶ範囲（結論と根拠）

#### 結論（事実と解釈の切り分け）

1. **無改変の別ファイルとしての同梱（結論: 集合物に近く、ikari 本体や辞書には及ばないと読める）**
   - 公式の `zenz-v3.2-small`（GGUF）を無改変のまま独立したファイル（`models/zenz-v3.2-small-Q5_K_M.gguf`）として配布物に同梱する行為は、CC BY-SA 4.0 上の「集合物（Collection）」への収録にあたり、モデル自体の翻案（Adapted Material）には当たらないと読める。
   - したがって、ikari-ime 本体のコード（Apache-2.0）や独立した辞書ファイル（別途 §3 で扱う）へ ShareAlike が自動的に波及するとは解されない。ただし、モデルファイル自体は引き続き CC BY-SA 4.0 で提供され、下流の受領者に対して追加の制限（Section 2(a)(5)(C)）を課してはならない。
2. **実行時に別プロセスから読み込む使い方（結論: グレーが残るが、翻案に当たる可能性は低いと推測）**
   - プログラムが実行時に CC BY-SA 4.0 のモデルファイルを読み込み、推論結果（数値 NLL）を変換順位付けに使う形態で、プログラム自体が「翻案物」になるかについては、CC の FAQ・条文に一律の判断基準がなく、解釈上のグレーが残る。
   - 推測（未確認）: ikari-ime は GGUF 推論を汎用エンジン `llama.cpp`（別プロセス `ikari-lm-host`）の標準入出力境界の向こうに分離し、モデルに依存しない汎用的なプロンプト形式と NLL スコアの仕組みを持ち（ADR 0060）、利用者が任意に別モデルへ差し替え可能な構成を採るため、特定モデルの著作物的表現をコードに取り込んだ翻案物に該当する可能性は低いと考えられる。この残存リスクについては 2026-11-19 のユーザー決定で受容済み。
3. **ikari 側で量子化・微調整（ファインチューニング）して配る場合（注意: 翻案物に当たる）**
   - もし配布元モデルを ikari 側で再量子化・再学習・アダプタ統合等して配布する場合は Adapted Material（または改変物）となり、その改変後モデルに CC BY-SA 4.0（Section 3(b)）を適用する必要がある。通常版・オフライン版では公式ファイルを無改変（sha256 一致）で同梱するため該当しない。

#### 根拠（一次資料）

- **CC BY-SA 4.0 Legal Code**（`https://creativecommons.org/licenses/by-sa/4.0/legalcode.txt`）:
  - **Section 1(a) Adapted Material**: 翻訳・改変・編曲・変形その他、著作権等に基づく許諾を要する態様で原著作物を変更して派生したもの。
  - **Section 2(a)(4)**: 媒体・形式の変換や技術的改変のみでは Adapted Material は生じない。
  - **Section 3(b) ShareAlike**: ShareAlike の継承義務は **「自身が作成した Adapted Material を共有する場合」**（if You Share Adapted Material You produce）に課される。
  - **Section 2(a)(5)(A), (C)**: 原著作物の受領者へ原ライセンスの申出が自動的に成立し、追加・異なる条件や技術的保護手段で受領者の権利行使を制限できない。
- **Creative Commons 公式 FAQ**（`https://creativecommons.org/faq/`）:
  - `#when-is-my-use-considered-an-adaptation`: 改変が新たな創作性を伴う翻案に当たるかは適用される著作権法による。別個の独立した著作物としてそのまま並べる収録は翻案ではない。
  - `#if-i-create-a-collection-that-includes-a-work-offered-under-a-cc-license-which-licenses-may-i-choose-for-the-collection`: 集合物全体には別のライセンスを選べる（ただし集合物内の元の CC 著作物のライセンス条件はそのまま残る）。
  - `#if-cc-sa-licensed-content-is-included-in-a-database-does-the-entire-database-have-to-be-licensed-under-an-sa-license`: ShareAlike は公に共有する改変物にのみかかり、未改変の著作物を収録した集合物・コンピレーション全体には波及しない。
  - `#can-i-apply-a-creative-commons-license-to-software`: CC ライセンスはソフトウェアコードには非推奨であり、CC BY-SA 4.0 は一体不可分に統合される場合のために GPLv3 との一方向互換性のみを設ける（Apache-2.0 との互換性条項はない）。

### 2.4 元になったモデルと学習データの条件

- **確認できた事実（2026-11-22）**:
  - `Miwa-Keita/zenz-v3.2-small-gguf`（`README.md`）および `Miwa-Keita/zenz-v3.1-small`（`README.md`）には、元モデル・学習データの記載がない。
  - `Miwa-Keita/zenz-v3.2-small` は非公開または不存在（HTTP 401）。
  - 同一作者による旧版 `Miwa-Keita/zenz-v2.5-small`（`README.md`）には、以下が明記されている:
    - 元モデル（`base_model`）: `ku-nlp/gpt2-small-japanese-char`（CC BY-SA 4.0。学習データ: 日本語 Wikipedia 20221020・CC-100 日本語部分・OSCAR 日本語部分）。
    - 学習データ（`datasets`）: `Miwa-Keita/zenz-v2.5-dataset`（全体は CC BY-SA 4.0、内訳として `train_wikipedia.jsonl` が CC BY-SA 4.0、`train_llm-jp-corpus-v3.jsonl` が `llm-jp-corpus-v3` の Common Crawl 由来データで ODC-BY および Common Crawl Terms of Use に基づく）。
- **推測（未確認）**:
  - v3.2 についても同系統のアーキテクチャ（GGUF メタデータ上で `gpt2`, `context_length: 1024`, `total: 95,058,432`）や類似データで構築された可能性が高いが、モデルカードに記載がないため v3.2 の学習データに NC/ND 条件や第三者権利の制約が混入していないかは検証不能である。
  - この点は 2026-11-19 のユーザー決定で残存リスクとして受容済み。

### 2.5 作者への確認

- 2026-11-19 のユーザー決定により、作者への確認（問い合わせ）は行わない。表示変更や作者からの指摘等で配布できなくなった時点で、自由指定モデルの機能（ADR 0060）等で代替・対応する。

---

## 3. ビルド済み辞書（`system.dict`）に `webwords`（CC BY-SA 4.0）を組み込む場合の論点

[ADR 0055](../adr/0055-web-words-classify-and-build.md) §0.3・§10 では「ビルド済みの辞書を配る段で、ShareAlike が辞書に及ぶか・表示の置き場所をもう一度確かめる」とされていた。通常版・オフライン版でデプロイ時点の辞書を同梱する（#490）にあたり、再確認した結果をまとめる。

### 3.1 事実の整理

1. `system.dict` は、`dict-builder` により以下の入力を 1 つのバイナリ辞書へ統合して生成される（[xtask/src/dict.rs](../../xtask/src/dict.rs)）:
   - SudachiDict Full（Apache-2.0 ＋ UniDic BSD-3-Clause ＋ NEologd Apache-2.0）
   - chiVe 1.3 mc90 の語彙頻度順位（Apache-2.0）
   - ABR 町字マスター（PDL1.0、CC BY 4.0 互換）
   - `webwords`（`webwords-20261101.tsv`、Wikipedia 日本語版由来。GitHub Release では CC BY-SA 4.0 の表示つきで配布）
2. `webwords` 一覧は記事本文や文を含まず「表記・出現回数・記事数・区分（4 種）」のみを統計的に抽出したものであり、日本の著作権法第 30 条の 4（情報解析）や事実データの非著作物性により著作権制限が適用される場合、CC BY-SA 4.0 Section 2(a)(2)・8(a) に従いライセンス条件は及ばない（ADR 0055 §0.3）。
3. 一方で、CC BY-SA 4.0 Section 4（Sui Generis Database Rights）は、EU データベース権等が成立する法域においてデータベースの実質的部分を別のデータベースへ組み込むと、組み込み先のデータベース全体が Adapted Material になる旨を定める（Section 4(b)）。また、日本の外の利用者に対して「一覧には CC BY-SA 4.0 を表示しているのに、同じ語彙を組み込んだ `system.dict` には一切 CC BY-SA 4.0 を示さない」と説明と実態が不整合になる恐れがある。
4. SudachiDict・UniDic・chiVe・ABR（PDL1.0 / CC BY 4.0 互換）はいずれも許容型ライセンスであり、下流の結合データ全体を CC BY-SA 4.0 で提供しても、元の著作権表示・NOTICE・`LEGAL`・出典表示を保持する限りライセンス上の矛盾は起きない。

### 3.2 同梱設計（#490）向けの選択肢と推奨

| 案 | 内容 | 利点 | 欠点・リスク |
| --- | --- | --- | --- |
| **案 A（推奨）**: `webwords` を含めた `system.dict` を同梱し、`system.dict` 全体を CC BY-SA 4.0 として表示する | `THIRD_PARTY_NOTICES.md` で「`system.dict` は `webwords`（CC BY-SA 4.0）の語彙を組み込んでいるため、`system.dict` ファイル単体は CC BY-SA 4.0（および SudachiDict・chiVe・ABR の各表示）に従う。ikari 本体コードは Apache-2.0 のまま独立」と明記する | 新語・IT 外来語の変換品質を維持できる。EU データベース権等を含む国際的な解釈リスクを抑えられ、Release の表示とも一貫する | `system.dict` 自体は CC BY-SA 4.0 の扱いになる（ただし Apache-2.0 本体とは別ファイルで集合物） |
| **案 B**: 同梱用 `system.dict` は `--no-webwords` でビルドし、`webwords` を切り離す | 同梱辞書を Apache-2.0 / BSD-3-Clause / PDL1.0 のみで構成し、`webwords` は各自取得（#162）または将来の別辞書スロットへ分離する | `system.dict` から ShareAlike 要素を完全に除ける | 同梱状態のままでは外来語セットの精度が下がる（ADR 0055 §4.5）。オフライン環境でも取り込めない限り劣化が残る |
| **案 C（非推奨）**: `webwords` を含めたまま `system.dict` には CC BY-SA 4.0 を適用しない | 30 条の 4 と事実データの非著作物性を根拠に、SA を付けずに配布する | SA 表示が増えない | 国外法域（EU データベース権等）での解釈リスクが残り、Release 一覧への CC BY-SA 4.0 表示との整合が説明しにくい |

---

## 4. 郵便番号変換表（ABR 町字・郵便番号変換表）の未確認事項の解消

2026-11-08 時点の洗い出しでは、日本郵便側の説明ページが未取得のため再配布の可否が「不明」とされていた。2026-11-22 に一次資料を直接確認した結果:

1. **ArcGIS メタデータ**（`https://www.arcgis.com/sharing/rest/content/items/1098a4f61dab4f52997839d04245132a?f=json`）:
   - `licenseInfo` は空ではなく、`https://www.digital.go.jp/policies/base_registry_address_tos`（アドレス・ベース・レジストリの利用規約）へのリンクが設定されている。
   - 説明文（`description`）に「郵便番号については、日本郵便株式会社がウェブサイトに掲載される郵便番号データの説明（`https://www.post.japanpost.jp/service/search/zipcode/download/readme.html`）をご参照ください。アドレス・ベース・レジストリについては、デジタル庁のアドレス・ベース・レジストリの利用規約に従って利用してください。」と明記されている。
   - 補足（2026-11-23 照合）: ArcGIS 側の公開説明文は月次更新で上書きされるため、2026-10-23 取得時点の版番号（郵便番号データの版 2609、更新日 2026-09-24）は後から配信元では追認できない。表示には取得日と、取得時点の版である旨（配信元の上書きにより後から追認できない旨）を併記する。
2. **日本郵便「郵便番号データの説明」**（`https://www.post.japanpost.jp/service/search/zipcode/download/readme.html` および `https://www.post.japanpost.jp/zipcode/dl/readme.html`）:
   - 「使用・再配布・移植・改良について: **郵便番号データに限っては日本郵便株式会社は著作権を主張しません。自由に配布していただいて結構です。**」と明記されている。
3. **結論**: `postal.tsv` は日本郵便側・デジタル庁側（PDL1.0）のいずれの規約からも**再配布が可能**（条件: PDL1.0 に従い出典・加工・主体を明記し、国の未加工データに見せないこと、また日本郵便データの出典も併記すること。参考情報・無保証の前提）。ただし約 21.8MB の容量と月次更新の性質から、同梱に含めるか各自取得に残すかは #490 で設計上の採否を決める。

---

## 5. 配れないもの／各自取得に残すものの方針（#162 への引き継ぎ）

今回の一次資料調査の結果、**「ライセンス上再配布が禁止されており、絶対に同梱できないデータ」はなかった**。

一方で、運用・サイズ・リスク・用途の理由から、**同梱せず各自取得（#162）または配布対象外とする方針**を次のとおり整理する:

| 区分 | 対象 | 理由と #162 / #490 での扱い |
| --- | --- | --- |
| **各自取得（#162）に残す** | ユーザーが自由指定する LM モデル（ローカルファイル / URL 取得） | ADR 0060 の方針。公式同梱の `zenz-v3.2-small` 以外のモデルは利用者が取得・導入する（オフライン版はローカルファイルのみ、#246） |
| **設計次第で各自取得に残しうる** | `webwords`（§3 で案 B を採る場合） | ShareAlike を同梱 `system.dict` から外す選択をした場合は、従来どおり Release 資産から各自取得またはオフライン bundle（#246）で導入する |
| **設計次第で各自取得に残しうる** | `postal.tsv`（約 21.8MB、月 1 回更新） | ライセンス上は同梱可能だが、容量と更新頻度の観点から #490 で同梱要否を決める（外す場合は #162 の仕組みで導入・更新） |
| **将来の退避先（#162）** | `zenz-v3.2-small` が将来配布できなくなった場合 | 作者の方針変更・取り下げ等で配布不能になった場合は、同梱から外し、#162・ADR 0060 の自由指定・各自取得経路へ退避する |
| **配布対象外（取得専用）** | `kanakanji-edge`（比較実験用）、`AJIMEE-Bench`（評価用） | 製品では使わない開発・評価用データのため配布物に含めない |

---

## 6. 同梱の実装（#490 / #99）時に改める必要のある既存箇所

[ADR 0060](../adr/0060-user-selectable-lm-model.md)「更新（2026-11-19）」に従い、同梱の実装が入るまでは同梱を先取りした書き換えを行わない。実装時に合わせて更新が必要な箇所を次にまとめる。

1. **[THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md)**:
   - 同梱対象の辞書（SudachiDict、`vectors.bin`、ABR、`postal.tsv`、`emoji.tsv`、`webwords`）の「バンドルには含めない」「利用者の手元で生成」の記載を、同梱の実態に合わせて改める。
   - `zenz-v3.2 small` の節を §2.2 の表示文面案（確認日・作者・HF コミット・sha256・変更なし・モデルファイル単体適用・元モデル未記載の注記）に更新し、末尾の「ライセンス表示義務」の「辞書、モデル、地名のデータはこのリポジトリに含めません。各自が取得し〜」を「配布物に含まれる辞書・モデルの扱い」に合わせて改める。
   - （補足）`llama.cpp` の節（225 行目）にある「`IkariEngine.xpc` の静的ライブラリに含まれる」という記述は、別プロセス `ikari-lm-host` 化（ADR 0060 Q4、ADR 0072 §5）後の実態に合わせて直す。
   - ※注意: [xtask/src/webwords/publish.rs](../../xtask/src/webwords/publish.rs)（21 行目）が `データベースダンプ jawiki-` を含む行がちょうど 1 行であることを検査しているため、同文字列を他の行に増やさない。
2. **[scripts/fetch-data.sh](../../scripts/fetch-data.sh)（20 行目）と [xtask/src/data/fetch.rs](../../xtask/src/data/fetch.rs)（10 行目）**:
   - コメントに旧表記「モデルカードでは Apache-2.0」が残っているため、現在の `CC BY-SA 4.0` に直す。
3. **[windows/installer/ikari.iss](../../windows/installer/ikari.iss)（118 行目）と .NET 依存ライセンス**:
   - 現在は `THIRD_PARTY_LICENSES_RUST.md` のみを `{app}` へコピーしており、`THIRD_PARTY_NOTICES.md` がインストーラに入っていない。Windows 版でモデル・辞書を同梱する（または候補窓を配布する）際は、`THIRD_PARTY_NOTICES.md` および `.NET` / Windows App SDK 関連のライセンス文面（`license.txt` 等。`THIRD_PARTY_NOTICES.md` 288 行目）を配布物に同梱する。
4. **[xtask/src/licenses.rs](../../xtask/src/licenses.rs)（25 行目）**:
   - 現在の `SHIPPED` は macOS の 3 成果物（`lm` metal、`engine-ffi`、`shell-ffi`）のみを対象としている。Windows 版を配布する段（#99 / #490）では、Windows の配布バイナリ（`windows/engine-host`、`windows/tip`、`windows/candidate`、`windows/settings`、`fetch`、`lm` vulkan 等）の Rust 依存クレートを一覧に含めるか・別ファイルにするかを決める。
5. **[ADR 0060](../adr/0060-user-selectable-lm-model.md) 末尾で列挙された関連文書**:
   - [ADR 0071](../adr/0071-settings-ui-and-fetcher.md) §3、[ADR 0076](../adr/0076-ikari-data-contract.md) §2・§7、[ADR 0072](../adr/0072-lm-cage-levels.md)、[security-design.md](../security-design.md)、[ikari-ime_spec.md](../ikari-ime_spec.md) NFR-17（同梱実装時にあわせて改訂する）。
