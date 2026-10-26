# ADR 0002: フェーズ1のアーキテクチャ（文脈・学習・ユーザーデータ・Rewriter・話題スコアの実験）

- 状態: 承認（2026-10-11。§21 の決定を参照）
- 日付: 2026-10-11
- 対象: フェーズ1（リード経由でユーザーが承認した範囲 ①〜⑨と、話題スコアの実験）
- 前提: ADR 0001（フェーズ0）、`docs/ikari-ime_spec.md` v0.1、main = da30b93、ベースラインの評価 `eval/reports/phase0-baseline.md`（1位正解率 21.4%）

---

## 0. 現状の把握（設計の根拠になった事実）

### 0.1 フェーズ0の実装（ADR 0001 どおり）
- `ikari-core`: `Converter::new(dict, grammar, ranker, opts)` と `convert(reading, &Context)`。`lattice::build(key, dict, grammar, bos_right_id)` は **BOS の右IDを引数で受け取る作りになっている**が、`converter.rs` は常に `grammar.bos_eos_id()` を渡している。
  - 候補はプールの単位で `Ranker::rank` に渡し、score の安定ソートで末尾に追加する。表示済みのページは固定される（NFR-05）。
- 辞書の形式は v1。エントリは16バイトで、`flags` の u16 は予約のまま（0）。索引は読みの fst だけで、表記の索引も正規化表記（lemma）もない。
- CLI（`ikari`）は1コマンドだけで、サブコマンドはない。`ikari-eval` には回帰判定がなく、`profile` 列も使っていない。
- 遅延は warm の p99 が 0.079ms、cold の最悪値が 9.6ms。**予算（16ms）は大きく余っている**。

### 0.2 ベースラインの誤りの原因（実辞書で確認した）
リードの見立て「カタカナの未知語経路」は、実際には**未知語ではなく辞書のエントリ**だった。SudachiDict は**解析用の辞書**なので、表記ゆれ（カタカナ書き、ローマ字書き、数字）を低いコストで持っている。変換（読みから表記を選ぶ）では、これが上位に来てしまう。

| 誤った1位 | 実際の経路（`--format tsv` で確認） | 原因になったエントリ |
| --- | --- | --- |
| コウセイ | コウ(名詞) + セイ(名詞) 5347 | `コウ` の正規化表記は `こう`（カタカナの表記ゆれ）、コスト 2924。`セイ` は人名 |
| 4ヨウ | 4(数詞, 読み シ) + ヨウ 3155 | `4` に読み シ/ヨ/ヨウ/ヨッ などがあり、コスト 4010〜5769 |
| カイ等 | カイ + 等(接尾辞) 4328 | `カイ` はコスト 3300 の名詞（正規化表記もカタカナ） |
| ナッタ | 名詞,固有名詞,人名 4354 | 人名のカタカナ |
| 始めてシッタ | シッタ（正規化表記 `しった`） | カタカナの表記ゆれ |
| カキススメる | カキ（→かき）+ ススメる（→勧める） | カタカナの表記ゆれ |

実データでの規模:
- カタカナの表記ゆれ（表記がカタカナを含み、正規化表記がカタカナを含まない）は **21,478 行**。ただし `スズメ/雀`、`マグロ/鮪` のように、カタカナ表記が普通に使われる語も含む。
- 英小文字だけの表記で読みがかなの行は **17,919 行**（例: `hennkann` / ヘンカン、コスト 4978。入力ミスを解析するためのエントリ）。
- 英数字を含む表記の行は全体で約 44k 行（`relax` / リラックス など）。
- lemma（正規化表記）は 875,365 種類、表記は 1,362,833 種類。
- **辞書形ID（CSV の13列目）は small_lex の行番号（0始まり）**（`ひんし` 134002 → 134003行目の `ひんする` で確認）。ここから活用の規則を自動で導ける。

### 0.3 左文脈の接続は大きく効く（matrix.def で確認）

| 前の語（右ID） | 次の語（左ID） | 連接コスト |
| --- | --- | --- |
| と（格助詞, 793） | しよう（為る 意志推量形, 1820） | **-2794** |
| と（格助詞, 793） | 名詞,普通名詞,一般（5145, 仕様型） | +1459 |
| に（格助詞, 796） | なっ（成る 非自立可能, 4168） | **-4422** |
| に（格助詞, 796） | 名詞,固有名詞,人名（4788, ナッタ） | +582 |
| BOS（0） | 人名（4788） | +1 |

BOS の右IDを左文脈の最後の語にするだけで、a-03（なった）、a-05（しよう）、h-07 の文法ケースは、コストの差が 4000 以上ひらく見込み。

### 0.4 外部リソース（2026-10-11 に実際に確認した。本体はまだダウンロードしていない）

| 資源 | 入手元 | サイズ | ライセンス | 備考 |
| --- | --- | --- | --- | --- |
| chiVe v1.3 mc90 テキスト | `https://sudachi.s3-ap-northeast-1.amazonaws.com/chive/chive-1.3-mc90.tar.gz` | 593,557,443 B（HEAD で確認） | Apache-2.0（README の v1.3 の項） | 語彙 410,533、300次元、Sudachi の**正規化表記**がキー、SudachiDict 20240109-core で学習。Last-Modified 2024-03-01 |
| chiVe v1.3 mc30 | 同じ場所の `chive-1.3-mc30.tar.gz` | 1,098,109,577 B | 同上 | 語彙 759,011 |
| zenz-v3.2-small GGUF | HF `Miwa-Keita/zenz-v3.2-small-gguf`、`ggml-model-Q5_K_M.gguf` | 73,871,936 B | **HF のメタデータは apache-2.0**（README はフロントマターだけ） | 2026-05-06 更新 |
| zenz-v3.2-xsmall GGUF | HF `Miwa-Keita/zenz-v3.2-xsmall-gguf` | 20,970,304 B | 同上 apache-2.0 | 2026-07-20 更新 |
| zenz-v3.1-small GGUF | HF `Miwa-Keita/zenz-v3.1-small-gguf` | 73,871,968 B | cc-by-sa-4.0 | |
| zenz-v2.5-small（safetensors） | HF `Miwa-Keita/zenz-v2.5-small` | 180,915,904 B、91M パラメータ | cc-by-sa-4.0（基盤は ku-nlp/gpt2-small-japanese-char, CC BY-SA 4.0） | GGUF 版の公開はない |
| AJIMEE-Bench | GitHub `azooKey/AJIMEE-Bench`、`JWTD_v2/v1` | 200件 | **データは CC BY-SA 3.0**、評価コードは CC0 | 左文脈あり100件、なし100件。入力はカタカナで、許容解は複数 |
| llama-cpp-2（Rust） | crates.io 0.1.159（2026-10-07） | — | MIT OR Apache-2.0 | feature `metal` あり。**ビルドに cmake が必要だが、この機械には未導入**（Xcode は導入済み） |
| sudachi.rs | GitHub `WorksApplications/sudachi.rs`（2026-10-02 更新） | — | Apache-2.0 | **crates.io には公式に公開されていない**（非公式の再配布 `suiko-sudachi` があるだけ）。別途 Sudachi のバイナリ辞書（core 72MB zip）が要る |

- zenz の v3 系のプロンプト形式は、AzooKeyKanaKanjiConverter の `ZenzPromptBuilder.swift` で確認した。
  - `[ プロフィール][ 話題][ 文体][ 好み]` + ` 左文脈（末尾40文字）` + ` 読み（カタカナ）` + `` のあとに、出力の表記が続く。
  - **話題とプロフィールの条件（、）がそのまま属性プロファイルに対応する。**
- 基準機は Apple M4、メモリ 16GB（NFR-07、NFR-08 と一致）。

---

## 1. 決定の要約

| 項目 | 決定 |
| --- | --- |
| ① コスト調整 | 辞書を**落とさずに**、エントリに `ime_penalty` を持たせる（形式 v2）。変換では `cost + penalty`、文脈解析では `cost` だけを使う。規則は「コードで決めた述語」×「値を書いたデータファイル」 |
| ② 文脈解析 | **自前の辞書と Viterbi で解析する `LatticeContextAnalyzer`**（表記の索引を追加）。sudachi.rs は使わない。BOS の右IDは、左文脈の最後の語の右ID |
| ③ 文書内キャッシュ | `DocumentCacheRanker`。解析した左文脈と確定履歴の内容語（表記と lemma）に加点する。メモリ内だけに持ち、保存しない |
| ④ 学習 | `LearningStore` と `LearningRanker`。キーは (読み, 表記, 特徴量ハッシュ)、層は `usage` と `initial`。保存は TSV で、生テキストは持たない |
| ⑤ プロファイル | `ProfileRanker`。内容語だけに加点し、上限を設け、文脈が増えるほど弱める。chiVe がなければ種語の一致だけで動く |
| ⑥ ユーザー辞書 | **Mozc 互換の4列 TSV**。品詞は簡略名から内部IDへ対応付ける。活用は grammar.bin v2 の「活用規則」で展開する。エクスポートはスキーマのバージョン付きの単一テキストで、インポートは上書き |
| ⑦ 文章の取り込み | 新しい crate `ikari-import`。LLM の出力は **JSONL**。辞書で強制アライメントし、2割の評価用は**ハッシュで決めて複製しない** |
| ⑧ Rewriter | `Rewriter` trait。最初のプールに `base_cost = 辞書の1位 + 1` で入れて Ranker を通す。学習は書式IDで行う |
| 話題スコア | `TopicRanker`（chiVe）と `ZenzRanker`（zenz, llama.cpp）を両方作り、eval の dev/test 分割で比較する。採用は ADR 0003 で決める |
| 形式 | `FORMAT_VERSION = 2`（system.dict と grammar.bin）。`vectors.bin` を新設する |
| 新しい crate | `ikari-import`（取り込み）と `ikari-lm`（zenz。default-members の外に置く） |

---

## 2. crate 構成と依存の向き

```
ikari-dict-format ── fst
   ^        ^
   |        |
ikari-core  ikari-dict-builder (lib+bin; vectors サブコマンドを追加)
 ^  ^  ^ ^
 |  |  | └─ ikari-lm (lib; llama-cpp-2。feature "metal")   ← default-members の外
 |  |  └── ikari-import (lib; serde, serde_json)
 |  └───── ikari-eval (lib+bin; ikari-import と、feature "lm" で ikari-lm)
 └──────── ikari-cli (bin "ikari"; ikari-import と、feature "lm" で ikari-lm)
```
- **判断: zenz は `ikari-lm` という別 crate に隔離する。** llama.cpp のネイティブビルド（cmake と Metal）は重く、壊れやすい。core と CI に持ち込まない。`[workspace] default-members` から外し、cli と eval は `--features lm` のときだけ依存する。
- **判断: 取り込みは `ikari-import` に置く。** JSONL のために serde と serde_json が必要になるが、core は依存を軽く保ちたい。取り込みは core の辞書、解析器、学習に依存し、core は取り込みに依存しない。
- 循環依存はない。

---

## 3. 辞書の形式 v2（dict-format）

### 3.1 共通
- `FORMAT_VERSION = 2`。v1 のファイルは読み込まない（ADR 0001 の方針どおりビルドし直す）。リトルエンディアン、8バイト境界、決定的に出力する点は v1 と同じ。

### 3.2 `system.dict` v2（ヘッダ 176 バイト）

| off | 型 | フィールド |
| --- | --- | --- |
| 0 | [u8;8] | `IKARIDIC` |
| 8 | u32 | format_version = 2 |
| 12 | u32 | header_len = 176 |
| 16 | u64 | grammar_id |
| 24 | u32 | entry_count |
| 28 | u32 | kind |
| 32 | u64 | **dict_id**（FNV-1a 64 を index、entries、strings、lemma_ids、lemma_table の順にかけた値。vectors.bin の照合に使う） |
| 40 | u32 | **lemma_count** |
| 44 | u32 | 予約 0 |
| 48 / 56 | u64 | index（読みの fst）off / len |
| 64 / 72 | u64 | entries off / len |
| 80 / 88 | u64 | strings off / len |
| 96 / 104 | u64 | meta off / len（`policy=<ポリシーのFNV>`、`chive_vocab=<版>` を追加） |
| 112 / 120 | u64 | **surface_index**（表記の fst。値は postings の範囲 `start<<32 \| count`） |
| 128 / 136 | u64 | **postings**（u32 のエントリ番号。表記ごとに `(cost, index)` の昇順） |
| 144 / 152 | u64 | **lemma_ids**（u32 × entry_count。エントリ → lemma 番号） |
| 160 / 168 | u64 | **lemma_table**（`u32 offsets[lemma_count+1]` + UTF-8 の連結。lemma は辞書順で、番号は添字） |

**エントリ（16バイト。v1 の flags を置き換える）**

| off | 型 | フィールド |
| --- | --- | --- |
| 0 | u32 | surface_offset |
| 4 | u16 | surface_len |
| 6 | u16 | left_id |
| 8 | u16 | right_id |
| 10 | i16 | cost（Sudachi の値そのまま。**解析に使う**） |
| 12 | u16 | pos_id |
| 14 | i16 | **ime_penalty**（変換時に cost へ足す。`i16::MAX` = `IME_EXCLUDED`、変換では使わない） |

- lemma は CSV の12列目（正規化表記、`\uXXXX` を戻したもの）。`*` や空なら表記を使う。
- 表記の索引は、`MAX_READING_CHARS` と同じ32文字以下の表記だけを入れる（解析ラティスの最大語長）。
- 規模の見積もり（v1 の 60MB からの増分）: 表記の fst 約15MB、postings 約6MB、lemma_ids 約6MB、lemma_table 約12MB。合計で約100MB（mmap）。

### 3.3 `grammar.bin` v2（ヘッダ 96 バイト）
- v1 の 0〜80 はそのまま。80 / 88 に **conj（活用規則）off / len** を追加する。
- **品詞レコード v2**: `u16 rep_left_id, u16 rep_right_id, i16 user_cost, u16 rep_share_permille, UTF-8 名前`（固定部 8 バイト）。
  - `user_cost`: その品詞のシステム辞書のコストの10パーセンタイル。ユーザー辞書の語のコストに使う。
  - `rep_share_permille`: その品詞のエントリのうち、代表の左右IDと一致するものの割合（‰）。⑥の検証に使う。
- **活用規則の節**: `u32 rule_count`、続けて規則を `(base_pos, target_pos)` の順に並べる。

  | 型 | フィールド |
  | --- | --- |
  | u16 | base_pos（例: `動詞,一般,*,*,下一段-マ行,終止形-一般`） |
  | u16 | target_pos（例: `…,下一段-マ行,連用形-一般`） |
  | u8 | strip_chars（終止形の末尾から削る文字数） |
  | u8 | add_surface_len |
  | u8 | add_reading_len |
  | u8 | 予約 |
  | bytes | add_surface（UTF-8）、add_reading（ひらがな UTF-8） |

- **規則の導き方**（builder）: 品詞の5列目（活用型）が `*` でない各行について、辞書形ID の行を基準形とし、表記と読みそれぞれで共通接頭辞を求めて (削る文字数, 足す文字列) を出す。`(base_pos, target_pos)` ごとに**最も多い規則**を採用する。支持数が3未満なら捨てる。
- grammar_id は活用規則の節と品詞レコードも含めて計算する。

### 3.4 `vectors.bin`（新設。spec「単語ベクトル：語IDで引く別ファイル」）
ヘッダ 96 バイト:

| off | 型 | フィールド |
| --- | --- | --- |
| 0 | [u8;8] | `IKARIVEC` |
| 8 | u32 | format_version = 1 |
| 12 | u32 | header_len = 96 |
| 16 | u64 | dict_id（system.dict の dict_id と一致しなければ**読み込まない**） |
| 24 | u16 | dims（300） |
| 26 | u16 | cluster_count（64） |
| 28 | u32 | row_count |
| 32 / 40 | u64 | lemma_rows（u32 × lemma_count。行番号、なければ u32::MAX）off / len |
| 48 / 56 | u64 | rows（i8 × dims × row_count）off / len |
| 64 / 72 | u64 | centroids（i8 × dims × cluster_count）off / len |
| 80 / 88 | u64 | meta（`source=chive-1.3-mc90` など）off / len |

- 各行は L2 正規化してから ×127 で丸めた i8。コサイン類似度は i8 の内積 ÷ 127² で近似する。
- **行の並びは chiVe のテキストの順番**（word2vec のテキストは頻度の降順）にする。**行番号がそのまま頻度順位**になり、§4 のコスト事前分布の実験にも使える。
- lemma の対応付けは、**辞書の lemma（Sudachi の正規化表記）と chiVe のキーの完全一致**だけにする。chiVe は Sudachi の正規化表記で学習されているので、表記ゆれ（`ユーザ` と `ユーザー`、活用形）は自然に同じベクトルに集まる。一致率は builder がレポートする。
- 規模の見積もり: 一致する lemma を最大で約30万件とすると、300次元の i8 で約90MB（mmap なので、常駐するのは触れた行だけ）。大きすぎれば、実験の後に PCA で128次元に落とす（形式は dims を持っているので変えずに済む）。
- クラスタ: builder が k-means（k=64、反復20回、乱数の種は固定）で求める。学習の「話題」特徴量（§8）に使う。

---

## 4. ① 不要な表記とコストの調整（ポリシー）

### 4.1 しくみ
- **判断: 行は落とさずに `ime_penalty` を付ける。** 解析（§5）では表記ゆれも数字も必要になる（左文脈に `4つ`、`スズメ` と書かれることがある）。変換で出したくないだけなので、ペナルティで表す。
- **判断: 述語はコードに固定し、値はデータファイルに書く。** 汎用の規則言語は作らない（YAGNI）。ファイルは `dict-builder/policy/default.tsv` で、列は `述語<TAB>値`。値は整数（加算）か `exclude`。

| 述語 | 判定（CSV の行に対して） | 初期値 |
| --- | --- | --- |
| `ascii_lower` | 表記が英小文字、数字、空白、`.'&-` だけで構成され、英小文字を含む | `exclude` |
| `ascii_other` | 表記に ASCII の英字を含む（上の規則に当たらないもの） | +3000 |
| `numeric_surface` | 品詞2 = 数詞で、表記に数字（ASCII または全角）を含む | `exclude`（数字は NumberRewriter で出す） |
| `katakana_variant` | 表記がカタカナを含み、lemma がカタカナを含まない | +3000 |
| `symbol_kana` | 品詞1 = 記号 | +3000 |
| `proper_noun` | 品詞2 = 固有名詞 | +1000 |
| `person_name` | 品詞3 = 人名（`proper_noun` に加算される） | +1000 |
| `place_name` | 品詞2 = 固有名詞、かつ品詞3 = 地名（`proper_noun` に加算される。#205） | -750（地名の合計は +250） |
| `official_ka_counter` | 品詞 = 名詞,普通名詞,助数詞可能、表記が「か」+ 月・国・所、lemma が「箇」+ 同じ漢字（か月・か国・か所。ADR 0019 §17） | -6000 |
| `katakana_place_peer` | 品詞 = 名詞,固有名詞（地名、一般、人名）、表記がカタカナ（長音を含む）だけで、同じ読みに chiVe の頻度が 5 万位より上の漢字の地名の行があり、その表記がカタカナより頻度が高い（シブヤ に 渋谷、カワサキ に 川崎。#223。`chive.tsv` のみ）。値は余白 M で、総コストを「同じ読みの最安の頻度が高い漢字の名詞の、1 語だけを変換するときの総コスト + M」にそろえる（上げるだけ） | 600（加算ではなく余白） |
| `rare_lemma` | `--chive-vocab` を指定したときだけ: lemma が chiVe の語彙にない | +1500（実験の対象） |

- 当たった規則の値はすべて加算する。`exclude` が1つでも当たれば `IME_EXCLUDED` にする。加算した結果は i16 に飽和させる。
- 未知語のコスト（10000）は据え置く。ベースラインでは未知語の経路は誤りの原因になっていなかった（§0.2）。
- BuildReport に「述語ごとに当たった件数」を出す。
- 値は §15 の eval の **dev 分割**だけで調整する（test 分割を見て調整しない）。

### 4.1.1 地名のペナルティ（#205。2026-10-22）
- **決定**: 地名は誰でもどんな場面でも使うので、人名（合計 +2000）と分けて軽くする。述語 `place_name`（品詞3 = 地名）を足し、`proper_noun` +1000 に加算して **地名の合計を +250** にする（`place_name` = -750）。人名と、地名・人名以外の固有名詞（+1000）は変えない。同音の普通の語と同音の地名（みなと、にしき、さくら など）の除外リストは作らず、一律で軽くする（ユーザー決定）。
- **掃引**（地名の合計。辞書は値ごとに作り直した。`eval/scripts/snapshot.sh` の全セット 4403 件 + 新セット `eval/sets/place-names.tsv` 153 件。1位の正解数。LM なし / あり）:

| 地名の合計 | place-names LM なし | place-names LM あり | 既存の全セット（LM なし・あり共通の差） |
| --- | ---: | ---: | --- |
| +1000（変更前） | 105 | 137 | 基準 |
| +750 | 106 | 137 | 差なし |
| +500 | 107 | 138 | 差なし |
| **+250（採用）** | 110 | 138 | 差なし |
| 0 | 111 | 138 | 差なし |
| -500 | 113 | 140 | counters の 三+とう が 頭 → 島 に悪化（1件） |
| -1000 | 115 | 140 | 同上（1件） |

- **選んだ値の幅**: 合計 0〜+1000 の間は、place-names と既存の全セット（LM なし・あり）に悪化がない。0 と +250 の差は いしかわ（イシカワ → 石川）だけで、カタカナ表記に負ける件（#223）の領域なので、中ほどの +250 にした。-500 以下は、日常の助数詞（三頭）が地名（三島）に負け始めるので避ける。
- **直る例（LM なし、+250）**: きょうと（今日と → 京都）、おおたく（おお宅 → 大田区）、新幹線で+きょうと、奈良県+なら（なら → 奈良）。LM ありでは、まるのうち（○のうち → 丸の内）が直る。うめだ（梅だ）、かまくら（カマ・ク・ラ）は、地名の合計が -500 まで下げても LM なしでは直らない（辞書のコストが高い）。
- **残る誤り（この変更の対象外）**: 有名な地名の漢字がカタカナ表記に負ける（しぶや → シブヤ、よこはま → ヨコハマ など）のは #223。
- **反映**: ペナルティは辞書のビルドで入るので、辞書を作り直さないと効かない。実機は `scripts/install-data.sh` が要る。

### 4.2 効果の見込み（ベースラインのケース）
- コウ+セイ、シッタ、カキ、ススメる は +3000 以上、4 は除外、ナッタ は +2000 になる。
- こうせい は「更生 5398 / 構成 5404」の差が 6 しかないので、ベースのコストだけでは決まらない。文脈（③と話題スコア）の出番になる。

---

## 5. ② ContextAnalyzer

### 5.1 判断: 自前の辞書と Viterbi で解析する（sudachi.rs は使わない）

| 比較軸 | 自前（`LatticeContextAnalyzer`） | sudachi.rs を trait で包む |
| --- | --- | --- |
| ID空間 | 変換と同じ辞書と grammar なので、**解析結果の右IDをそのまま BOS に使える** | Sudachi の辞書のバージョンがずれると、左右IDが合わない。対応付けが要る |
| 追加の資源 | 表記の索引（約20MB、同じ mmap） | Sudachi のバイナリ辞書（core の zip で 72MB、展開後はさらに大きい）を別途持つ |
| 供給 | 自分で管理する | crates.io に公式版がなく、git 依存になる |
| 解析の質 | 単語コストと連接コストは Sudachi と同じ。未知語の処理（char.def）だけが簡略 | Sudachi そのもの |
| 実装コスト | ラティスを流用。表記の索引と未知語のまとめ処理で数百行 | ラッパーは小さいが、ID の対応付けとバージョン管理が増える |

Sudachi のコストは解析用に作られているので、**解析に使うと本来の用途どおりになる**（変換では §4 のペナルティを足すが、解析では足さない）。sudachi.rs は、解析の誤りが原因で評価が頭打ちになったときに、同じ trait の別実装として足せばよい。spec の未決事項「解析器」はこれで決定とする。

### 5.2 契約（`core/src/analysis/mod.rs`）
```rust
pub trait ContextAnalyzer: Send + Sync {
    /// 左文脈を解析する。結果は Arc で共有する（Converter と Ranker が同じものを見る）。
    fn analyze(&self, left_text: &str, history: &[Commit]) -> Arc<AnalyzedContext>;
}

#[derive(Clone, Debug, Default)]
pub struct AnalyzedContext {
    /// 解析窓の中のトークン（古い順）。窓は末尾の CONTEXT_WINDOW_CHARS 文字
    pub tokens: Vec<ContextToken>,
    /// 変換ラティスの BOS に使う右ID
    pub bos_right_id: u16,
    /// tokens のうち内容語の数（プロファイルの減衰に使う）
    pub content_count: usize,
}

#[derive(Clone, Debug)]
pub struct ContextToken {
    pub surface: String,
    pub lemma: String,          // 未知語なら表記
    pub lemma_id: Option<u32>,  // システム辞書の lemma 番号（ベクトルを引く用）。それ以外は None
    pub word: Option<WordId>,
    pub left_id: u16,
    pub right_id: u16,
    pub pos_id: u16,
    pub is_content: bool,       // crate::pos::is_content_word
    pub char_end: u32,          // 左文脈の末尾からの距離（文字）。キャッシュの減衰に使う
}

pub const CONTEXT_WINDOW_CHARS: usize = 2000;
pub struct NullAnalyzer;   // BOS を固定し、トークンを返さない（フェーズ0と同じ挙動。テストと比較実験用）
```
```rust
// core/src/pos.rs（契約。中身も契約コミットで実装する）
/// 品詞1 が 名詞/動詞/形容詞/形状詞/副詞 のどれかで、品詞2 が 非自立可能 でも 数詞 でもない
pub fn is_content_word(pos_name: &str) -> bool;
pub fn pos1(pos_name: &str) -> &str;
```

### 5.3 `LatticeContextAnalyzer` の仕様（レーンB）
1. `left_text` の末尾 `CONTEXT_WINDOW_CHARS` 文字を対象にする。確定履歴（`history`）は left_text に含まれている前提（ADR 0001 の Context の定義どおり）。
2. 表記のラティスを作る。各位置で `Dictionary::common_prefix_search_surface` を引き、コストは `cost` だけを使う（penalty は足さない）。
3. **未知語**: 各位置で、同じ文字種（カタカナ、英字、数字、漢字、ひらがな、その他）の**最長の連続**を1ノードにする。左右IDは grammar の未知語の値、コストは `unk_cost` を1回だけ（文字数を掛けない）。さらに1文字の未知語ノードも置き、どこからでも次へ進めるようにする。
4. 前向き Viterbi で最良パスを1本だけ求める（N-best は不要）。
5. **BOS の規則**: left_text が空、または末尾が `。！？!?\n` なら `bos_eos_id`。それ以外は最後のトークンの右ID。
6. **増分のキャッシュ**: 直前に解析したテキストを `Mutex` で1件だけ持つ。新しい left_text がその延長なら、**前回の最後の文区切りより前の結果を再利用**し、そこから後ろだけを解析し直す（こまめに確定する入力で、毎回 2000 文字を解析しないため）。
7. 確定履歴にある (読み, 表記) は、left_text の解析とは別に、「確定済みの語」として DocumentCache に渡す（§7）。

### 5.4 Dictionary trait への追加（契約。既定の実装付きで、壊れない変更）
```rust
pub trait Dictionary: Send + Sync {
    // 既存の4つのメソッドはそのまま
    /// 表記が text の接頭辞と一致するエントリを列挙する。f(一致した表記のバイト長, entry)
    fn common_prefix_search_surface(&self, text: &str, f: &mut dyn FnMut(usize, WordEntry<'_>)) {}
    /// lemma（正規化表記）。id は「この辞書の lemma 番号」で、DictionarySet では slot 0 のものだけ Some
    fn lemma(&self, id: WordId) -> Option<Lemma<'_>> { None }
    /// system.dict の dict_id（vectors.bin の照合用）。なければ None
    fn dict_id(&self) -> Option<u64> { None }
}
#[derive(Clone, Copy, Debug)] pub struct Lemma<'a> { pub text: &'a str, pub id: Option<u32> }

pub struct WordEntry<'a> { /* 既存のフィールド */ pub penalty: i16 }   // フィールドを追加（構築は辞書の実装の中だけ）
pub const IME_EXCLUDED: i16 = i16::MAX;
```

---

## 6. Converter の変更（契約と レーンB）

### 6.1 組み立て
```rust
impl Converter {
    /// 既存の new は残す（builder に委ねる）
    pub fn builder(dict: Arc<dyn Dictionary>, grammar: Arc<dyn Grammar>) -> ConverterBuilder;
}
pub struct ConverterBuilder { /* … */ }
impl ConverterBuilder {
    pub fn ranker(self, r: Arc<dyn Ranker>) -> Self;                 // 既定は CostRanker
    pub fn analyzer(self, a: Arc<dyn ContextAnalyzer>) -> Self;      // 既定は NullAnalyzer
    pub fn rewriters(self, r: Vec<Arc<dyn Rewriter>>) -> Self;       // 既定は空
    pub fn clock(self, c: Arc<dyn Clock>) -> Self;                   // 既定は SystemClock
    pub fn options(self, o: ConvertOptions) -> Self;
    pub fn build(self) -> Result<Converter, ConvertError>;
}
pub struct ConvertOptions { /* 既存 */ pub rewriter_anchor: usize /* 既定 1 */ }
```

### 6.2 `convert` の流れ（フェーズ1）
1. `ctx` を sanitize する（secure なら空。既存の処理）。
2. `analysis = analyzer.analyze(&ctx.left_text, &ctx.history)`。secure なら解析せず、`AnalyzedContext::default()`（BOS は `bos_eos_id`）を使う。
3. `lattice::build(key, dict, grammar, analysis.bos_right_id)`。ノードのコストは `cost.saturating_add(penalty)` にし、`penalty == IME_EXCLUDED` のエントリはノードにしない。
4. 辞書の候補でプールを作る（既存の処理）。
5. **Rewriter**: 各 Rewriter の候補を集め、表記が既に出ているものは捨てる。`base_cost = (プール内の辞書候補の base_cost を昇順に並べたときの rewriter_anchor 番目) + 1` で、`origin = Rewriter { name, format }`、セグメントは未知語のIDで1つとして、プールに加える（spec「辞書の上位候補より後ろ」）。
6. かなの候補を加え、Ranker を通し、安定ソートする（既存の処理）。

`RankInput` に `analysis: &'a AnalyzedContext` と `dict: &'a dyn Dictionary` を追加する（契約）。

### 6.3 確定（学習への入口）
```rust
impl Conversion {
    /// generated()[index] を確定したことを表すイベント。secure なら None（学習しない。NFR-18）
    pub fn commit(&self, index: usize) -> Option<CommitEvent>;
}
pub struct CommitEvent {
    pub reading: String,
    pub candidate: Candidate,
    pub analysis: Arc<AnalyzedContext>,   // 特徴量は学習側（§8）で作る
}
```

### 6.4 CandidateOrigin（契約）
`Rewriter { name: &'static str, format: u16 }` を追加する（`#[non_exhaustive]` なので、外部の match は既に `_` を持っている）。

---

## 7. ③ DocumentCacheRanker と Ranker の合成（レーンB）

### 7.1 合成
- `CompositeRanker::new(Vec<Arc<dyn Ranker>>)` を作り、順に `rank` を呼ぶ。各 Ranker は score に**差分を足すだけ**にする（ADR 0001 の方針）。
- **単位**: すべてコスト単位で、負の値が加点。各 Ranker の重みは `*Config` 構造体（`Default` 付き）で持ち、`from_kv(&BTreeMap<String, String>)` で上書きできる。CLI と eval の `--ranker-config <file>`（`key=value` の行）で渡す。
- 既定の並び: `DocumentCache → Learning → Profile → Topic → Zenz`（存在するものだけ）。

### 7.2 DocumentCacheRanker
- 入力は `RankInput.analysis.tokens` の内容語と `context.history` の表記。**メモリ内だけで持ち、保存しない**（NFR-19）。
- 候補の内容語セグメントごとに、次の加点をする。
  - 同じ表記が窓の中にある: −1500
  - 表記は違うが同じ lemma がある: −800
  - 末尾から 500 文字より遠い出現は半分にする。
- 1候補あたりの合計は −3000 までにする。機能語のセグメントには加点しない。
- 加点したセグメントの数は、eval のデバッグ用に `Candidate` へは残さない（Ranker 内部のログだけに出す。Candidate の契約は変えない）。

---

## 8. ④ 文脈付きの学習（レーンU）

### 8.1 特徴量（`core/src/user/features.rs`）
`extract(analysis: &AnalyzedContext, vectors: Option<&dyn WordVectors>, salt: u64) -> ContextFeatures`

| 種類 | 中身（ハッシュの対象） | 重み（初期値） |
| --- | --- | --- |
| `Unconditional` | なし（固定値 0） | 0.2 |
| `Prev` | `"p:" + 直前のトークンの lemma + "/" + 品詞1` | 1.0 |
| `PrevContent` | `"pc:" + 直近10トークン内で最後の内容語の lemma` | 0.7 |
| `Bag` | `"b:" + 直近20個の内容語の lemma`（重複を除く。1つずつ別の特徴量） | 合計 0.8 |
| `Topic` | `"t:" + 文脈ベクトルに最も近いクラスタの番号`（vectors.bin があるときだけ） | 0.8 |

- ハッシュは FNV-1a 64 で、利用者ごとのソルトを前置する。ソルトは学習ファイルのヘッダに保存し、エクスポートにも含める。
- **プライバシーの評価（隠さずに書く）**: 生テキスト、語順、文は保存しない（NFR-19 を満たす）。ただし、ソルトと辞書の語彙が分かれば、Bag と Prev の特徴量は**総当たりで単語には戻せる**。文章は復元できないが、「近くに出た語の集合」は推測されうる。気になるなら Bag を設定で無効にできるようにする（§20 の判断事項）。

### 8.2 保存形式（`<user_dir>/learning.tsv`）
```
#ikari-learning	schema=1	salt=<16桁の16進>
layer	source	unit	reading	surface	segments	feature	count	last_day
usage	-	phrase	はじめてしった	初めて知った	初めて/はじめて/副詞,*,*,*,*,* + 知っ/しっ/動詞,… + た/た/助動詞,…	9f3a…	3	20372
initial	2	word	ひんし	品詞	品詞/ひんし/名詞,普通名詞,一般,*,*,*	1b07…	5	20371
```
- 列: `layer` は `usage` / `initial`。`source` は取り込み元の番号（usage は `-`）。`unit` は `phrase`（確定した候補全体）/ `word`（セグメント）。`segments` は spec どおり表記、読み、品詞名の**文字列**（語IDは保存しない）。`feature` は16進、`last_day` は UNIX 日。
- Rewriter の候補は、surface に `@rw:<name>:<format>` を入れる（文字列ではなく書式を学習する。spec）。
- 書き込みは一時ファイルを作ってから rename する。上限は20万行で、超えたら `last_day` の古いものから捨てる。

### 8.3 API
```rust
pub struct LearningStore { /* RwLock の中に HashMap<(reading, surface_key, feature, layer, unit), Stat> */ }
impl LearningStore {
    pub fn open(path: &Path) -> Result<Self, UserDataError>;   // ファイルがなければ空で作る（ソルトを生成）
    pub fn record(&self, ev: &CommitEvent, vectors: Option<&dyn WordVectors>, today: u32);
    pub fn add_initial(&self, source: u32, reading: &str, segs: &[SegmentKey], feats: &ContextFeatures, today: u32);
    pub fn reset(&self, layer: LayerSel) ;              // usage / initial / all
    pub fn remove_source(&self, source: u32);
    pub fn save(&self) -> Result<(), UserDataError>;
    pub fn salt(&self) -> u64;
}
pub struct LearningRanker { store: Arc<LearningStore>, vectors: Option<Arc<dyn WordVectors>>, cfg: LearningConfig }
```

### 8.4 スコア
- reading が一致する記録について、現在の文脈から作った特徴量ごとに、その特徴量の下での選択の割合 `share = n(r,f,c) / N(r,f)` を求め、`Σ_f w_f × share × B` を加点する（B = 3000）。
- `phrase` の一致は ×1.0、`word` の一致はセグメントごとに ×0.5。
- 層の重みは usage 1.0、initial 0.4。経過日数で減衰させる（半減期90日）。合計の加点は −6000 までにする。
- **「最後に選んだものを1位にする」学習にはしない**。Unconditional の重みを低くしているので、技術文書で「品詞」を選んだ記録は、小説の文脈（Bag、Topic、Prev が一致しない）ではほとんど効かない（spec の学習の節）。
- secure のときは `commit` が None を返すので記録しない。Ranker 側も、secure なら加点しない。

---

## 9. ⑤ 属性プロファイル（レーンB が Ranker、レーンU が保存）

### 9.1 データ
- プリセットの分野は `core/assets/profiles/<domain>.tsv`。1行目が `#ikari-domain<TAB>id=it<TAB>name=IT・ソフトウェア`、以降は種語の lemma（1行1語、50〜200語）。
- フェーズ1のプリセットは `it`、`infra`、`novel`、`business`（一覧は §20 の判断事項）。
- 利用者の設定は `<user_dir>/profile.tsv`（`domain<TAB>weight`。weight は `often` = 1.0、`sometimes` = 0.5）。
- `Context` に `profile: Option<Arc<Profile>>` と `with_profile` を追加する（契約）。アプリごとの切り替え（spec の候補）と、eval のケースごとの指定を同じ口で扱える。

### 9.2 ProfileRanker の規則（spec の制約を実装に落とす）
1. **内容語だけ**: `is_content_word` が真のセグメントだけを見る。
2. **加点の量**:
   - vectors.bin があれば、プロファイルのベクトル（種語のベクトルを重み付きで平均したもの）とセグメントのベクトルのコサイン類似度 `cos` を使い、`−K × max(0, cos − 0.15)` を加点する（K = 3000）。
   - vectors.bin がなければ、種語に lemma が一致したら −800。
3. **上限**: 1候補あたり −1200 まで。BOS の連接で文法的に決まる差（例: と→しよう の 4000 以上）は覆らない。
4. **文脈で減衰**: `× 1 / (1 + analysis.content_count / 20)`。
5. **否定ケースを必須にする**: h-07（ドキュメント化 + しよう）と a-05 が、プロファイルの有無で変わらないことを eval で確かめる（`--profile-ab`、§15）。これが崩れたら、上限を下げるか、「品詞1の並びが違う候補同士では順位を入れ替えない」という強い規則を足す（第二案として保留）。

---

## 10. ⑥ ユーザー辞書とエクスポート／インポート（レーンU）

### 10.1 形式（spec の未決事項「Mozc/MS-IME 互換 TSV」に対する判断: **Mozc 互換の4列**にする）
`<user_dir>/user_dict.tsv`
```
#ikari-user-dict	schema=1
かきすすめる	書き進める	動詞:下一段-マ行	
いかり	ikari-ime	短縮よみ	自作IME
```
- 列は 読み、表記、品詞、コメント。品詞は簡略名で、動詞は `動詞:<活用型>` と書く。
- Mozc の書き出したファイルは `ikari user-dict import --format mozc` で読み込む。Mozc の品詞名（`動詞カ行五段`、`動詞一段` など）は `core/assets/user_pos_mozc.tsv` で対応付ける。
  - 「動詞一段」の上一段／下一段と行は、`る` の直前の文字の段と行から推定する。

### 10.2 簡略品詞の対応表（`core/assets/user_pos.tsv`）

| 簡略名 | 内部の品詞名 | 活用 |
| --- | --- | --- |
| 名詞 | 名詞,普通名詞,一般,*,*,* | なし |
| サ変名詞 | 名詞,普通名詞,サ変可能,*,*,* | なし |
| 形状詞 | 形状詞,一般,*,*,*,* | なし |
| 固有名詞、組織 | 名詞,固有名詞,一般,*,*,*（Sudachi には「組織」の細分類がない） | なし |
| 人名 / 姓 / 名 | 名詞,固有名詞,人名,{一般/姓/名},*,* | なし |
| 地名 | 名詞,固有名詞,地名,一般,*,* | なし |
| 副詞 | 副詞,*,*,*,*,* | なし |
| 感動詞 | 感動詞,一般,*,*,*,* | なし |
| 短縮よみ | 名詞,普通名詞,一般,*,*,*（表記が読みと無関係でもよい） | なし |
| 動詞:<活用型> | 動詞,一般,*,*,<活用型>,終止形-一般 | 必須（活用規則で展開） |
| 形容詞 | 形容詞,一般,*,*,形容詞,終止形-一般 | 自動 |

### 10.3 登録の処理
1. 品詞を引く。grammar に品詞がなければエラー。
2. 活用する品詞なら、grammar.bin v2 の活用規則で、すべての活用形の (表記, 読み, 品詞) を展開する。語尾が規則の削る文字列と一致しなければエラー（例: 下一段なのに `る` で終わらない）。
3. 各形を、その品詞の代表左右ID と `user_cost` で `InMemoryDictionary` に入れる。ペナルティは 0。
4. ユーザー辞書は `DictionarySet` の slot 1 に push する。表記の索引も持つので、文脈解析でもユーザーの語が使える。

### 10.4 代表左右IDの検証（`ikari user-dict check` と builder のレポート）
- 対応表の全品詞について、grammar に存在すること、左右IDが連接表の範囲内にあること、`rep_share_permille ≥ 950` であることを確かめる。
- 950 未満なら警告を出す（代表IDで足りない品詞がある、という ADR 0001 の懸念7への答え）。
- 活用する品詞では、活用規則が `終止形-一般` から少なくとも `連用形-一般` と `未然形-一般` まで導けていることを確かめる。

### 10.5 エクスポート／インポート
`ikari data export <file>` は、次の形の単一の UTF-8 テキストを書き出す。
```
#ikari-export	schema=1
[profile]
…profile.tsv の中身…
[user_dict]
…
[learning]
…（ソルトのヘッダを含む）…
[ingest_sources]
…
```
- セクションの見出しは、行全体が `[名前]` の行だけ。各セクションの中身は、それぞれのファイルのスキーマ（バージョン付き）に従う。
- **インポート**: 最初に全体を検証する（スキーマのバージョン、各行）。1つでも不正があれば何も変えない。正しければ、全ファイルを `.tmp` に書いてから rename し、**上書き**する（spec: マージしない）。
- スキーマのバージョンが古ければ、変換関数を通す（schema=1 しかないうちは「一致しなければエラー」）。

---

## 11. ⑦ 文章の取り込み（`ikari-import`、レーンU）

### 11.1 流れ
1. `ikari ingest prompt` で、同梱したプロンプト `import/prompts/ingest-v1.md` を表示する。
   - プロンプトのヘッダにはバージョン `ingest-prompt-v1` があり、LLM に出力ファイルへこのバージョンを書かせる（NFR-22）。
   - プロンプトの冒頭と CLI の両方で、「業務文書をクラウドの LLM に送らない」注意を出す（NFR-28）。
   - プロンプトでは、コードブロック、インラインコード、URL を除くように指示する。
2. ユーザーが LLM に入力し、出力を保存する（NFR-21）。
3. `ikari ingest run <file> [--dry-run]` で、検証、アライメント、学習の順に処理する。
4. `ikari ingest list | remove <id> | reset | candidates | accept <n>…` で管理する。

### 11.2 LLM 出力のスキーマ（JSONL）
**判断: TSV ではなく JSONL にする。** チャット画面からコピーすると、タブが空白に化けることがあるため。
```
{"type":"header","format":"ikari-ingest","version":1,"prompt_version":"ingest-prompt-v1"}
{"type":"sentence","surface":"ユーザー辞書に単語を登録するときは品詞を指定する。","reading":"ゆーざーじしょにたんごをとうろくするときはひんしをしていする。"}
{"type":"doc_break"}
```
- **検証**（NFR-24。信用しない入力として扱う）:
  - 1行は 4KB まで、表記は400文字まで、ファイルは 50MB まで。
  - `reading` は `normalize_key` を通したあと、ひらがな、ー、および表記に現れる記号、英数字だけで構成されること。
  - `prompt_version` は既知のものであること。
  - 不正な行は捨てて、理由ごとに件数を報告する。

### 11.3 強制アライメント（辞書で読みを再現する）
- 状態を (表記の位置 i, 読みの位置 j) とする DP。辺は次の3種類。
  - **辞書の辺**: `common_prefix_search(reading[j..])` で引いたエントリのうち、表記が `surface[i..]` の接頭辞と一致するもの。コストは `cost + 連接コスト`。
  - **文字そのままの辺**: 表記の1文字が、かなとして読みの1文字と一致する。または記号、英数字が両方に同じ文字で現れる。
  - **未知語の辺**: 表記の漢字、カタカナ、英字の連続を1つ以上の読みの文字に当てる。コストは高くし、1文につき1回まで。
- 最小コストのパスを採用する。パスがなければ文を捨てる（件数を報告する）。
- **未知語の辺を使った文は、学習には使わない。** その (表記, 推定した読み) を登録候補として `<user_dir>/ingest_candidates.tsv` に件数付きで貯める（spec: ユーザーが確認したものだけをユーザー辞書に入れる）。

### 11.4 評価用の2割と初期学習
- **評価用**: `FNV64(表記) % 5 == 0` の文を評価用にする。**決定的に選ぶので複製しない**（NFR-19、NFR-26: 原文は取り込みファイルにだけ残る）。eval は取り込み元のパスを `ingest_sources.tsv` から読み、同じ規則で取り出す。
- **学習用（残りの8割）**: アライメントしたトークン列を `AnalyzedContext` と同じ形に組み、各内容語について次の場合に `initial` 層の `word` 記録を作る。特徴量は §8.1 を同じ関数で作る。
  - その読みに辞書上で2つ以上の表記がある（同音異義語）。
  - 同じ lemma に複数の表記がある（表記の好み。`ユーザ` と `ユーザー` など）。
- **分野の推定**: 文書ごとの内容語ベクトルの平均と、各プリセットの重心とのコサイン類似度を表示し、`--apply-profile` を付けたときだけ `profile.tsv` に書く。
- **取り込み元の台帳** `<user_dir>/ingest_sources.tsv`: `id, path, imported_at, fnv64, prompt_version, sentences_ok, sentences_dropped`。
  - 削除（`remove <id>`）では、その source の initial 記録を消す（NFR-26）。
  - 再学習の時にパスが存在しなければ通知だけにする（NFR-27）。

---

## 12. ⑧ Rewriter（レーンR）

### 12.1 契約（`core/src/rewriter/mod.rs`）
```rust
pub trait Rewriter: Send + Sync {
    fn name(&self) -> &'static str;
    fn rewrite(&self, input: &RewriteInput<'_>, out: &mut Vec<RewriteCandidate>);
}
pub struct RewriteInput<'a> { pub reading: &'a str /* normalize_key 済み */, pub context: &'a Context, pub now: LocalDateTime }
pub struct RewriteCandidate { pub surface: String, pub format: u16 /* Rewriter ごとに固定の書式ID。変えない */ }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalDateTime { pub year: i32, pub month: u8, pub day: u8, pub hour: u8, pub minute: u8, pub weekday: u8 /* 0=月 */ }
pub trait Clock: Send + Sync { fn now(&self) -> LocalDateTime; }
pub struct SystemClock;   // jiff でローカルのタイムゾーンを使う（jiff の型は契約に出さない）
pub struct FixedClock(pub LocalDateTime);   // テスト用
```

### 12.2 実装

| Rewriter | 発火の条件 | 書式ID（固定） |
| --- | --- | --- |
| `DateRewriter` | きょう、ほんじつ、あした、あす、きのう、あさって、おととい、ことし、らいねん、きょねん、いま | 1 `2026/10/11`、2 `2026年10月11日`、3 `10月11日`、4 `令和8年10月11日`、5 `2026-10-11`、6 `10/11(土)`。年: 11 `2026年`、12 `令和8年`。時刻: 21 `14:05`、22 `14時05分`、23 `午後2時5分` |
| `NumberRewriter` | 読みが数字（ASCII、全角）だけか、小数点を1つ含む | 1 `1,234`、2 `千二百三十四`、3 `１２３４`、4 `一二三四` |
| `CalculatorRewriter` | `=` で終わり、`0-9 . + - * / ( )` だけの式（ASCII。ローマ字表はこれらの文字をそのまま通す） | 1 結果だけ、2 `式=結果` |
| `SymbolRewriter` | `core/assets/symbols.tsv`（`読み<TAB>表記<TAB>書式ID`。やじるし、ほし、おなじ、まる、しかく、さんかく、こめ、ゆうびん など） | 行ごとに固定 |
| `EmoticonRewriter` | `core/assets/emoticons.tsv`（かお、かおもじ、えもじ、わらい など。手作業で作る） | 行ごとに固定 |

- z＋キーの記号は既存のローマ字表で扱う（変えない）。
- 令和の計算は 2019-05-01 を元年の始まりとする。0除算や桁あふれ（i64 を超える）の式では、候補を出さない。
- データファイルは自作する。Mozc や他の IME のデータは写さない（ライセンスを増やさないため）。

---

## 13. 話題スコアの実験（chiVe と zenz）

### 13.1 TopicRanker（chiVe。レーンB が Ranker、レーンA が vectors.bin）
```rust
// 契約（core/src/vectors/mod.rs）
pub trait WordVectors: Send + Sync {
    fn dims(&self) -> usize;
    fn vector(&self, lemma_id: u32) -> Option<&[i8]>;
    fn nearest_cluster(&self, v: &[f32]) -> Option<u16>;
    fn rank(&self, lemma_id: u32) -> Option<u32>;   // 頻度順位（行番号）
}
pub fn open_vectors(path: &Path, dict_id: u64) -> Result<Arc<dyn WordVectors>, DictError>;   // 実装はレーンA
```
- **文脈ベクトル**: 解析窓の内容語のうち最新50個のベクトルを、新しいものほど重く（`0.97^距離`）平均する。内容語が3個未満なら加点しない。
- **候補の加点**: 内容語セグメントごとに `−W × max(0, cos(seg, ctx) − τ)`（W = 4000、τ = 0.10）。1候補あたり −2500 まで。
- **ビルド**: 次の手順で作る。tar.gz の展開はユーザーが行い、gzip の依存は増やさない。
  ```
  ikari-dict-builder vectors --chive data/raw/chive-1.3-mc90/chive-1.3-mc90.txt --dict data/build/dict --out data/build/dict/vectors.bin
  ```

### 13.2 ZenzRanker（zenz。`ikari-lm`、レーンR）
- **版**:
  - 第一候補は **zenz-v3.2-xsmall（21MB）と zenz-v3.2-small（74MB）の GGUF**。v3 系は文脈と話題、プロフィールの条件を持てる。サイズは NFR-10（0.5〜1B 級まで）を大きく下回る。
  - ライセンスの表記が v3.2（apache-2.0）と v3.1 / v2.5（CC BY-SA 4.0）で異なり、基盤モデルは CC BY-SA 4.0 なので、**確定するまでは CC BY-SA 4.0 として扱う**（§20）。
- **推論**:
  - **llama-cpp-2（feature `metal`）** を使う。Zenzai 自身が llama.cpp で GGUF を動かしており、実績がある。
  - candle（純 Rust）も検討したが、GGUF の GPT-2 と zenz の特殊トークンの扱いを自分で検証する必要があり、却下した。
  - ビルド要件: cmake（`brew install cmake`）と Xcode のコマンドラインツール。
- **プロンプト（v3）**: `[ プロファイルの表示名][ 話題（解析窓の上位の内容語を3つまで）] +  + left_text の末尾40文字 +  + 読み（カタカナ） + `
- **スコア**: 候補ごとに `NLL = −Σ log P(表記のトークン + EOS | プロンプト)`。
  - プロンプトは1回だけ評価して KV キャッシュに載せ、候補はシーケンスを分けて並列に評価する（KV のコピー）。
  - プールが30候補、各数トークンなら、1回の decode は100トークン強で済む。
  - 加点は `α × (NLL − min NLL) × 1000`（α は dev 分割で調整する）。
- **時間の予算（NFR-04）**:
  - モデルは専用のワーカースレッドが持つ（llama の context は Sync ではないため）。
  - `rank()` は要求を送り、`recv_timeout(budget = 30ms)` で待つ。間に合わなければ何もせず戻る（ベースと n-gram 相当の順位のまま）。遅れて届いた結果は (読み, 文脈の末尾) をキーに1件だけ保持し、同じ変換が来たら使う。
  - フェーズ2で入力中の先行計算（NFR-03）を入れるときは、この保持の口を使う。
- **解放（NFR-11）**: 最後の要求から `idle_unload`（既定300秒）経ったら、ワーカーがモデルを解放する。次の要求で読み直し、読み直している間は時間切れとして扱う。
- **テスト**: `trait LmScorer` を内部に切り、モックでワーカー、時間切れ、解放をテストする。実モデルのテストは `#[ignore]` を付け、`IKARI_ZENZ_MODEL` が設定されているときだけ動かす。

### 13.3 実験の計画
- **構成**:
  - R0 = cost（フェーズ0）
  - R1 = R0 + ポリシー（§4）+ 解析器（BOS）
  - R2 = R1 + DocumentCache
  - R3 = R2 + Topic（chiVe mc90）
  - R4 = R2 + Zenz（xsmall）
  - R5 = R2 + Zenz（small）
  - R6 = R3 + R5
  - 補助: R1 に `rare_lemma` を足したもの、chiVe を mc30 にしたもの
- **データ**: §15.2 の拡充セット（dev / test 分割）、AJIMEE-Bench（長い入力、外部ベンチ）、取り込んだ文章の評価用2割（あれば）。
- **指標**: 1位正解率（全体、kind 別）、MRR、warm と cold の p99、Zenz の時間切れ率、常駐メモリ（`getrusage` の最大 RSS。eval の bin で測る）。
- **採用の基準**（test 分割で判断する）:
  1. 否定ケースと文法ケースの正解率が R1 より下がらない（必須）。
  2. 「話題」kind の1位正解率で、R2 に対して +5 ポイント以上。
  3. 遅延: LM なしで warm p99 ≤ 16ms。LM ありで p99 ≤ 30ms、かつ時間切れ率 ≤ 5%。
  4. 1〜3 を満たす中で最も正解率が高いものを既定にする。差が2ポイント以内なら、軽い方（Topic を優先）。
- 結果は `eval/reports/phase1-topic.md` に残し、決定を **ADR 0003** に書く。

---

## 14. CLI（`ikari`、レーンU）

```
ikari [convert] [OPTIONS] <READING>        既存の引数はすべて互換。以下を追加
    --user-dir <DIR>         ユーザーデータ（$IKARI_USER_DIR。既定は macOS が ~/Library/Application Support/ikari-ime、他は $XDG_DATA_HOME/ikari-ime）
    --ranker <SET>           cost | full（既定 full = 解析器と、使えるすべての Ranker）
    --no-analyzer            BOS を固定する（比較用）
    --vectors <PATH>         vectors.bin（既定 <dict>/vectors.bin があれば使う）
    --profile <NAME|none>    profile.tsv の代わりに一時的に指定する
    --ranker-config <PATH>   重みの上書き（key=value）
    --lm <GGUF> [--lm-budget-ms 30]   feature "lm" のときだけ
    --commit <RANK>          その候補を確定し、学習する（learning.tsv を保存する）
ikari session [--script <TSV>]    1行ずつ読みを入力し、番号で確定する。確定履歴を左文脈に積んでいく。--script は「読み<TAB>選ぶ表記」で非対話
ikari user-dict add <読み> <表記> <品詞> [--comment] | remove <読み> <表記> | list | import [--format ikari|mozc] <file> | check
ikari learn show [--reading <r>] | reset --layer usage|initial|all
ikari profile presets | show | set <domain>=often|sometimes ... | clear
ikari ingest prompt | run <file> [--dry-run] [--apply-profile] | list | remove <id> | reset | candidates | accept <n>...
ikari data export <file> | import <file>
```
- clap の `args_conflicts_with_subcommands` を使い、`ikari <READING>`（フェーズ0の形）もそのまま動くようにする。フェーズ0の統合テストは変えない。

---

## 15. ⑨ eval（レーンR）

### 15.1 機能の追加
- `--min-top1 <0..1>`: 全体の1位正解率が下回れば終了コード 1。
- `--baseline <cases.tsv> --fail-on-regression`: 基準で1位だったケースが1位でなくなれば終了コード 1。その一覧を出す。
- `--split dev|test|all`: `split` 列で絞る（列がなければ all）。
- `--profile-ab`: profile 列のあるケースを、プロファイルの有無の両方で実行して差分を出す。否定ケースが下がればレポートで目立たせ、終了コード 1。
- **期待値の別解**: `expected` を `|` で区切って複数書ける（表記に `|` は使えない。検証する）。
- `--set-format ajimee <json>`: AJIMEE-Bench の JSON を読む（入力をカタカナからひらがなにし、`expected_output` を別解にする）。
- `--scenario <tsv>`: 学習のシナリオ。列は `step`（commit / probe）、context、reading、surface。一時的なユーザーディレクトリで、commit を実行してから probe を評価する。
- `--ingest-holdout <user_dir>`: 取り込みの評価用2割を読み出し、アライメントしたトークンを「内容語＋後続の機能語」の単位にまとめて、(左文脈, 読み, 期待) のケースを作る。
- 構成を変えるフラグ: `--ranker`、`--no-analyzer`、`--vectors`、`--profile`、`--ranker-config`、`--lm`（CLI と同じ意味）。
- `ikari-eval compare <a.tsv> <b.tsv>`: ケースごとの順位の差分表を出す。
- レポートに構成の行、最大 RSS、LM の時間切れ率を追加する。

### 15.2 評価セットを拡充する方針
1. **同音異義語の拡充** `eval/sets/homophones-ext.tsv`（列: `split`、`source=llm-v1`）
   - 対象は30グループ程度（ひんし、こうせい、かいとう、しよう、いし、きかい、いじょう、かんしん、たいしょう、ほしょう、きこう、せいさく、じき、ついきゅう など）。
   - 表記ごとに約10文脈を作り、合計で約1000件にする。生成のプロンプトは `eval/prompts/homophones-v1.md` に置く。
   - 文脈の長さは「短い（1〜2語）」「中くらい」「長い（2文）」を混ぜる（手がかりの強い文脈ばかりにしないため）。
   - 機械的な検証: 期待値の読みが辞書で再現できること、期待値が候補に出ること。出なければ `unreachable` の印を付けて除外する。人手で1割を抜き取って確認する。
   - **dev と test を `FNV(id) % 5 == 0 → test` で分ける。調整は dev だけで行う。**
2. **否定ケースと文法ケース**: プロファイルと話題で文法が覆らないことを見る組を、各20件以上入れる（〜としよう、〜になった、〜にする/〜に擦る など）。
3. **怒りの記録**: 見つけ次第、`homophones.tsv` に足す（運用は spec どおり）。
4. **AJIMEE-Bench**: `eval/scripts/fetch-ajimee.sh` で `eval/external/` にダウンロードする（gitignore する。CC BY-SA 3.0 のデータはリポジトリに入れない）。
5. **自分の文章**: 取り込みの評価用2割（§11.4）。リポジトリには入れない。

---

## 16. 依存クレート（追加分）

| crate | 使う crate | 用途 | 理由 |
| --- | --- | --- | --- |
| `jiff = "0.2"` | core | ローカルの日時（Rewriter の SystemClock） | std はタイムゾーンを扱えない。chrono より API が安全で、tzdb をシステムから読む。jiff の型は契約に出さない |
| `serde = { version = "1", features = ["derive"] }`、`serde_json = "1"` | ikari-import | 取り込みの JSONL | 手書きの JSON パーサは脆い。core には入れない |
| `llama-cpp-2 = { version = "0.1.159", features = ["metal"] }` | ikari-lm（default-members の外） | zenz の推論 | §13.2 |

- **追加しないもの**:
  - flate2 と tar: chiVe の展開はユーザーが行う。
  - getrandom: ソルトは時刻、PID、アドレスから作る。秘密である必要はない（ファイルに保存する値なので）。
  - ndarray: i8 の内積は手で書く。
  - toml: 設定は key=value と TSV で足りる。
  - sha2: 取り込み元のハッシュは変更の検出用なので、FNV で足りる。

---

## 17. 並行実装のレーン分割

### 17.1 契約コミット（リードが入れる）
1. workspace に `import`（ikari-import）と `lm`（ikari-lm）を追加し、`default-members` から `lm` を外す。§16 の依存をすべて書き、Cargo.lock を確定させる（llama-cpp-2 は lock に入るが、default のビルドでは compile されない）。
2. **形式の定数**: `FORMAT_VERSION = 2`、`DICT_HEADER_LEN = 176`、`GRAMMAR_HEADER_LEN = 96`、`IME_EXCLUDED`、`VECTORS_MAGIC`、`VECTORS_FILE_NAME = "vectors.bin"`、`VECTORS_HEADER_LEN = 96`。
3. **core の契約ファイル**（シグネチャと型。本体は todo! にする）:
   - `dict/mod.rs`（§5.4 の追加）
   - `analysis/mod.rs`（§5.2。`NullAnalyzer` は中身まで実装する）
   - `pos.rs`（**中身まで実装する**）
   - `ranker/mod.rs`（RankInput の追加、`CompositeRanker` のシグネチャ）
   - `rewriter/mod.rs`（§12.1。`FixedClock` は中身まで実装する）
   - `vectors/mod.rs`（§13.1 の trait）
   - `candidate.rs`（`Rewriter` の variant）
   - `context.rs`（`profile`、`with_profile`）
   - `converter.rs` のうち `ConverterBuilder`、`CommitEvent`、`ConvertOptions` の型
   - `user/mod.rs`: `UserDir`（ファイル名の定数と既定パスの解決）、`UserDataError`、`Profile`、`LearningStore`、`LearningRanker`、`ContextFeatures`、`UserDictionary` のシグネチャ
4. `lib.rs` の mod 宣言と re-export。
5. 既存の呼び出し元（cli、eval、テスト）がコンパイルできる状態にする。WordEntry に `penalty: 0` を入れる箇所を直す。

**ルール**: ADR 0001 と同じ（契約ファイルや依存を変えるときはリードに相談する）。

### 17.2 レーンと持ち物

| レーン | 持つファイル | 最初に置くもの | 完了条件 |
| --- | --- | --- | --- |
| **A: 辞書 v2 とベクトル** | `dict-format/**`、`dict-builder/**`（policy/ を含む）、`core/src/dict/{system,grammar_file,set,memory}.rs`、`core/src/vectors/mmap.rs`、`testdata/**` | fixture の更新: 表記ゆれ、ASCII、数字、人名、活用（書き進める一族、なる/なっ）の行と、`testdata/chive-mini/vectors.txt`（20語、8次元の人工のベクトル） | 形式 v2 の往復テストと、壊れた入力でパニックしないテスト。表記の検索が全数探索と一致する。活用規則が fixture から導ける。vectors の往復と dict_id の照合。実辞書をビルドして `--verify` が通る。ポリシーのレポートを出す。**R1 構成（cost + ポリシー）の eval レポート** |
| **B: 文脈と順位付け** | `core/src/analysis/lattice_analyzer.rs`、`core/src/{converter,lattice,nbest}.rs`、`core/src/ranker/{composite,doc_cache,profile,topic}.rs` | InMemory の表記検索と、テスト用の `FakeVectors` を使う | 解析器: fixture で「やっと春に」の BOS が格助詞の右IDになる。文末で BOS に戻る。増分キャッシュが全体解析と同じ結果を返す。Converter: penalty と除外、Rewriter の挿入位置、commit、secure の扱い。各 Ranker の単体テスト（内容語だけ、上限、減衰）。release で、2000文字の文脈の解析が warm で 3ms 未満 |
| **U: ユーザーデータと取り込み、CLI** | `core/src/user/**`、`core/src/ranker/learning.rs`、`core/assets/{user_pos,user_pos_mozc}.tsv`、`core/assets/profiles/**`、`import/**`、`cli/**` | 活用規則をモックした grammar で、ユーザー辞書の展開をテストする | ユーザー辞書の add / import(mozc) / check。学習の往復、層ごとのリセット、source ごとの削除、a-02 のシナリオを単体で再現。エクスポート→インポートの往復と、不正なファイルで何も変わらないこと。ingest の検証、アライメント、評価用の選別、登録候補。CLI のサブコマンドの統合テスト。フェーズ0の統合テストがそのまま通る |
| **R: Rewriter、eval、LM** | `core/src/rewriter/{date,number,calc,symbol,emoticon}.rs`、`core/assets/{symbols,emoticons}.tsv`、`eval/**`、`lm/**` | `FixedClock` と、モックの `LmScorer` | 各 Rewriter のテスト（日付と年号の境界、桁区切り、漢数字、0除算）。eval の各フラグのテスト、AJIMEE の読み込み（サンプルの JSON で）。`ikari-lm` のワーカー、時間切れ、解放をモックでテスト。実モデルの `#[ignore]` テスト。cmake の手順を README に書く |

### 17.3 マージの順序
1. **A**（形式 v2。マージ後にリードが実辞書を作り直す）
2. **B**（A の表記索引を使った実辞書での確認を含む）
3. **U**（CLI の配線に B の builder と Ranker が要る）
4. **R**（eval の配線にすべてが要る）
5. マージ後の実験（§13.3）。担当は R のエージェントかリード。結果を ADR 0003 に書く。

R の Rewriter と LM、U のユーザーデータは、契約さえあれば A や B を待たずに進められる。待つのは配線と統合テストだけ。

### 17.4 フェーズ1全体の完了条件
- `cargo test --workspace` が通る（lm は除く。`cargo test -p ikari-lm --features metal` は手動）。
- 実辞書の `homophones.tsv` で、**a-03、a-05、h-07、h-09 が1位**（BOS と文法の効果）。フェーズ0で1位だった h-01、a-01 が1位のまま（`--fail-on-regression`）。
- a-02 の学習シナリオが通る。`--profile-ab` で否定ケースが下がらない。
- LM なしの既定構成で warm p99 ≤ 16ms（2000文字の文脈を付けて）。
- `eval/reports/phase1-topic.md` があり、ADR 0003 の案が出ている。

---

## 18. テスト方針（ADR 0001 に追加するもの）
- **fixture**: `testdata/sudachi-mini` に、§17.2 の A の行を足す。拡張後の fixture は、ポリシー、解析、活用、ユーザー辞書、取り込みのテストで共有する。
- **決定性**: 形式 v2、vectors（k-means の種は固定）、取り込み（評価用の選別）は、同じ入力から同じ出力になることをテストする。
- **ファジングに近いテスト**: 形式 v2 と vectors.bin で、1バイト反転と切り詰めを網羅する（v1 のテストを流用する）。取り込みの JSONL とエクスポートのファイルでは、壊れた行、巨大な行、未知のバージョンを入れる。
- **プライバシーのテスト**: `learning.tsv` とエクスポートのファイルに、取り込んだ文の表記が文として含まれないこと（3文字以上の部分文字列の一致を検査する）。secure のときに学習ファイルが変わらないこと。
- **時刻に依存するテスト**は `FixedClock` を使う。

---

## 19. 性能
- 解析器: 増分キャッシュで、こまめに確定する場合は再解析が末尾の1文程度で済む。最悪（2000文字を全部解析）でも数 ms の見込み。B の完了条件で測る。
- Ranker: どれもプール（30件）× セグメント数のオーダー。vectors の内積は i8 の300次元で、1候補あたりマイクロ秒未満。
- mmap の総量は system.dict が約100MB、grammar.bin が 72MB、vectors.bin が最大約90MB。常駐するのは触れたページだけ。
- zenz: small の Q5_K_M は 74MB に KV キャッシュが加わる程度で、NFR-09 の予算に十分収まる。時間の予算は §13.2。

---

## 20. 却下した案

| 案 | 却下した理由 |
| --- | --- |
| 表記ゆれ、ASCII、数字の行を builder で削除する | 文脈の解析に必要。変換だけで除外できれば足りる |
| 汎用の規則言語でコストを調整する | 今の述語は8つで足りる。データファイルで値だけ変えられれば調整には十分 |
| sudachi.rs で解析する | §5.1 |
| Mozc の辞書をコストの参考にする（spec の未決事項） | Mozc は品詞ID の体系が違い、コストを直接混ぜられない。ライセンスの確認も増える。まずポリシーと chiVe の頻度で詰め、それでも足りなければ判断し直す |
| 学習を「最後に選んだものを1位」にする | spec が明示的に禁じている |
| 学習を SQLite に保存する | 依存が重い。行数（20万まで）なら TSV と全件のメモリ読み込みで足り、エクスポートも同じ形にできる |
| 取り込みの出力を TSV にする | チャット画面からのコピーでタブが崩れる |
| 評価用の2割を内部にコピーする | NFR-19 に反する。決定的に選べば複製は要らない |
| zenz を core に直接入れる | ネイティブビルドが core と CI を重くし、壊しやすくする |
| candle（純 Rust）で zenz を動かす | GGUF と特殊トークンの扱いを自分で検証する必要がある。llama.cpp は Zenzai で実績がある |
| chiVe を mc5 / mc15 で使う | 3〜4GB と大きい。mc90 で効果を確かめてから、mc30 を比べる |

---

## 21. ユーザーかリードの判断が必要な点

1. **zenz のライセンス**: v3.2（xsmall / small）の HF メタデータは apache-2.0 だが、README には本文がなく、v3.1 / v2.5 と基盤モデルは CC BY-SA 4.0。作者に確認するまでは CC BY-SA 4.0 として扱い、THIRD_PARTY_NOTICES に書く方針でよいか。
2. **ダウンロードと置き場所**: chiVe v1.3 mc90（594MB の tar.gz）と zenz-v3.2 の xsmall / small（21MB と 74MB）を、`data/raw/` に置いてよいか（gitignore 済み）。展開は手動で行う。
3. **cmake の導入**（`brew install cmake`）。ikari-lm のビルドに必要。
4. **AJIMEE-Bench**（CC BY-SA 3.0）を使ってよいか。データはリポジトリに入れず、スクリプトで取得する。
5. **LLM で作る評価セット**（§15.2）: 生成に使う LLM と、1割の抜き取り確認を誰がするか。
6. **学習のプライバシーの残留リスク**（§8.1）: Bag と Prev の特徴量は、ソルトと語彙があれば総当たりで単語に戻せる。このまま既定で有効にするか、Bag を既定で無効にするか。
7. **ユーザー辞書の形式を Mozc 互換の4列 TSV にする**（spec の未決事項を、この ADR で決めてよいか）。
8. **ユーザーデータの既定の置き場所**（macOS は `~/Library/Application Support/ikari-ime`）。
9. **プロファイルのプリセット一覧**（spec の未決事項）: フェーズ1の候補は it / infra / novel / business。種語の作成は LLM で下書きし、人が確認する案。
10. **ポリシーの初期値**（§4.1）と、**固有名詞のペナルティ**の大きさ: dev 分割で調整するが、「人名の候補が上に出にくくなる」副作用を受け入れるか。
11. **Rewriter の挿入位置**: `rewriter_anchor = 1`（辞書の1位の直後）。Mozc に近づけるなら 2〜3。
12. **フェーズ1の完了条件**（§17.4）の数値。特に「話題」kind の目標値を、実験の前に置くか後に置くか（案: 後に ADR 0003 で置く）。
13. `docs/adr/README.md` の索引に 0002 を足す作業（このレーンでは ADR 本体だけを保存した）。

### 21.1 決定（2026-10-11）
| # | 決定 | 決めた人 |
| --- | --- | --- |
| 1 | zenz は CC BY-SA 4.0 として扱い、THIRD_PARTY_NOTICES に書く | ユーザー |
| 2 | chiVe v1.3 mc90 と zenz-v3.2 small/xsmall（Q5_K_M）を `data/raw/{chive,zenz}/` に置いた（展開済み）。sha256: small `29c223d4…b673`、xsmall `00c64b3d…fa58` | リード |
| 3 | cmake を導入した（4.4.4） | ユーザー承認、リードが実施 |
| 4 | AJIMEE-Bench を使う（データはリポジトリに入れず、スクリプトで取得する） | ユーザー |
| 5 | LLM で作る評価セットはリード（Claude）が生成し、1割の抜き取り確認はユーザーが行う | ユーザー |
| 6 | Bag の特徴量は既定で有効。残るリスクをドキュメントに書く | ユーザー |
| 7 | ユーザー辞書は Mozc 互換の4列 TSV | ユーザー |
| 8 | 既定の置き場所は `~/Library/Application Support/ikari-ime` | リード |
| 9 | プリセットは it / infra / novel / business。種語は Claude が下書きし、フェーズ1の終わりにユーザーが確認する | ユーザー |
| 10 | 固有名詞のペナルティは dev 分割で調整する。副作用は評価で監視する | リード |
| 11 | `rewriter_anchor = 1` | リード |
| 12 | 「話題」の目標値は、実験のあと ADR 0003 で決める | リード |
| 13 | 索引はリードが更新する | リード |

---

## 22. 実装後の更新（統合担当、2026-10-11）

> **[ADR 0029 で改訂]** `LeftContextRanker` の連接の増幅は、名詞で始まらない候補のうち `接尾辞,名詞的`（助数詞を除く）も対象にしなくなった（#137。最適 + かい → 解）。以下の本文は書き換えない。

実辞書での確認で見つかった問題に対する、ADR の決定の範囲内の調整。値は eval の dev 分割（`homophones-ext.tsv`）で決め、test 分割は最後の確認だけに使った。詳細は `eval/reports/phase1-quality.md`。

- **§4 の述語を3つ足した**（`dict-builder/src/policy.rs`）:
  - `wave_dash`（+3000）: 2文字以上で波ダッシュを含む表記（`なった〜`、`の〜`）。
  - `variant_surface`（+1500）: 活用しない語で、漢字を含む表記が lemma と違うもの（`浜死`/瀕死、`き会`/機会）。lemma が同じなので chiVe の頻度では区別できない。
  - `lemma_rank`（chiVe が要る。桁数あたり +2000）: lemma の chiVe 頻度順位の桁数（`log10(順位+1) - 3`）に比例して加算する。`rare_lemma`（有無の2値）より dev の1位正解率が高かった。`rare_lemma` は `lemma_rank` に含まれるので使わない。
  - `numeric_surface` はローマ数字（Ⅲ、ⅰ）も数字として扱う。
  - chiVe を使う述語は `policy/chive.tsv`（`default.tsv` + `lemma_rank`）に書く。`default.tsv` は chiVe なしでビルドできる。
- **`LeftContextRanker` を足した**（`core/src/ranker/left_context.rs`。§7.1 の並びの先頭）: ADR §0.3 の BOS の連接（と → しよう で -2794）だけでは、`しよう` が `使用` に勝てなかった。原因は §0.3 の比較相手が仕様（左ID 5145、+1459）だったことで、実際の競合は 使用（サ変可能、左ID 5133）の 単語コスト 4672・連接 -214・EOS 連接 -1334（合計 3124）に対し、しよう（意志推量形）が 単語コスト 7517・連接 -2794・EOS 連接 -96（合計 4627）だったこと。Sudachi の単語コストは解析用で、同音異義語どうしを比べる値ではない。そこで、左文脈の最後の語から最初の語への連接コストのうち、絶対値が 1000 を超えた分を 4 倍に増幅する（`left_context.weight=4`、`threshold=1000`）。増幅するのは「名詞で始まらない、1つの辞書の語でできた候補」だけ。名詞どうしに適用すると希少な名詞（聖核、製革）が偶然有利になり、複数の語に分かれた候補に適用すると拘束形（き、来）の分割（き会）が上がった。いずれも dev 分割で確かめた。
- **`TopicRanker` の既定値を変えた**（§13.1 の値は設定で上書きできる範囲）: `min_content` 3 → 1、`weight` 4000 → 10000、`max_total` 2500 → 5000。評価セットの文脈は 1〜2 語が多く、内容語 3 個未満で加点しない既定では話題スコアがほとんど働かなかった（dev の1位正解率 36.7% → 63.6%）。**§13.1 の「内容語が3個未満なら加点しない」を覆す変更**なので、ADR 0003 で再確認する。

### 22.4 和語・漢語の途中のハイフンの固定の規則（2026-10-13。タスク B12、ユーザーが承認）

「すぐ」の 2 位に Sudachi の解析用の表記 `す‐ぐ`（U+2010、読み スグ）が出た。§22.2 と同じ理由（ユーザーが入力していない文字を足さない）で、`ExclusionRule` に規則3を足した。評価は `eval/reports/phase3-hyphen.md`。

- 規則3 `hyphen_inside_native_word`: 2文字以上で、ハイフンかダッシュ（‐ ‑ ‒ – — ― − - －）の両隣がひらがなか漢字で、読みに「ー」がない（3 件: す‐ぐ、げん‐ぷ、脱‐存）。
- 対象外: カタカナや英字に隣るハイフン（セブン‐イレブン、Wi-Fi、e-文書法。外来語や綴りの区切りで、中点や ＝ と同じく名前の一部）、読みの長音に当たるダッシュ（アンサ−/アンサー）、記号そのものの項目、1文字の表記。波ダッシュの規則と違い「読みに『ー』がない」だけでは決めない（Wi-Fi/ワイファイ を落とすため）。
- 調べたが規則にしなかったもの: 両隣がひらがなか漢字の中点（145 行。小・中学 と そ・し・て が混ざる）、読点（25 行。一、二塁 と しょん、ベん）、括弧（22 行）、かぎ括弧（5 行。「夕」方）。扱いはリードの判断待ち（レポートの §1）。

### 22.3 `frequent_compound` の順位の上限と、接尾の「化」・副詞の分割（2026-10-13。B3〜B5 のレーン）

評価は `eval/reports/phase3-suffix.md`。

- **`frequent_compound` の対象の順位の上限を 3 万から 4 万にした**（`COMPOUND_MAX_RANK`。規則と値 -8000 は同じ）。構造化（31404 位）、拡張性、規格外が漏れていた。dev で 3.5 万〜5 万が同じ結果、6 万で 伊富魚 が 解凍 に勝つ。katakana-suffix 128 → 131 / 145、他のセットと CLI golden は変化なし。
- **採らなかった述語**（評価セットでは悪化がないが、セットにない自然な入力で1位が崩れた）:
  - `sahen_suffix`（接尾辞,名詞的,サ変可能 に加点）: Sudachi のサ変可能の接尾辞は EOS への連接が正（化 +364、終助詞 か -2538）で、文字列化 が 文字列か に負ける。ただし加点すると 本当化、無理化、駄目化 も勝つ。か と 化 は前の名詞の意味で決まり、品詞の規則では分けられない。
  - `short_kana_adverb`（ひらがな2文字以下の副詞にペナルティ）: こう増加、そう時期 の原因は「副詞の直後に名詞」という並びで、語ごとの加算では表せない。ペナルティを付けると すぐ → す‐ぐ、どうも → 道も などが崩れる。
- **ポリシーの範囲外として残したもの**（コードの設計の変更が要る。レポートの §6）: 文脈のあとの 器/機、板/版、次/時 は LeftContextRanker が 接尾辞 だけを増幅し、名詞,助数詞可能 を増幅しないことで決まっている（最大 6000 の差）。左文脈の語と候補が辞書の1語になるときの加点（自動 + 化 = 自動化）、副詞から名詞への連接の補正も、ポリシー（語ごとの加算）では表せない。

### 22.2 表記と読みの整合の固定の規則と、機能語・複合語のポリシー（統合担当、2026-10-13。ユーザーの指摘を受けた）

実機と CLI で見つかった問題（「ことだよね」の候補に「ことだよね〜」「ことだYOね」、「ぐたいれい」が「具体〇」）への対応。評価は `eval/reports/phase3-compound.md`。

- **固定の規則 `ExclusionRule`（`dict-builder/src/policy.rs`）を足した**。ポリシーの値ではなく、辞書を作るときの規則としてコードに固定する。該当する行は `IME_EXCLUDED` にして変換に使わない（文脈の解析には残る）。
  - 理由: 基本原則の1つ目（ユーザーの入力を正とする）。読みに対応しない記号を表記に足した項目（`ね〜`/ネ、`あの〜`/アノ）が候補に出ると、ユーザーが入力していない文字を付け足すことになる。ペナルティで後ろに回すだけでは足りない。
  - 規則1 `wave_dash_without_long_vowel`: 2文字以上で波ダッシュ（〜 U+301C、～ U+FF5E）を含み、読みに「ー」がない（178 件）。
  - 規則2 `decoration_mark`: 2文字以上で飾りの記号（！？!?。…‥♪♫♬）を含む。固有名詞（`yahoo!`、`ミニモニ。`）は名前の一部なので対象外（3 件）。語の一部になりうる記号（中点、ハイフン、☆印の☆）は対象にしない。
  - 対象外: 読みの長音に当たる「ー」「〜」（ユーザー、データ、あま〜い/アマーイ）、記号そのものの項目（`〜` カラ、補助記号）、1文字の表記。
  - **P3 で削除した汎用の `exclude`（ポリシーから除外を指定する仕組み）は復活させていない**。ポリシーの述語は今までどおり加算だけ。
- **述語 `function_word_notation`（+10000）を足した**: 品詞1が助詞・助動詞・接尾辞で、2文字以上の表記に ASCII の英字か波ダッシュを含む（`yo`/よ、`ね〜`/ない）。`ascii_other` と `wave_dash` に加算して後ろに回す（除外はしない）。名詞の英字表記（PC、API）は対象にしない。
- **`numeric_surface` に漢数字のゼロ「〇」（U+3007、読み れい／ぜろ の数詞）を足した**。一、二 などは対象にしない。
- **述語 `frequent_compound`（-8000。chiVe が要る）を足した**: 品詞が名詞,普通名詞で、表記が lemma と同じ、漢字だけの3文字以上、lemma の chiVe の順位が3万位より上。Sudachi は解析のために複合語の項目のコストを高くしていて（具体例 15422 > 具体 6762 + 礼 4397）、分割した経路に負ける。負の値（加点）で公平に比べる。

### 22.1 ひらがな／漢字の表記の既定（統合担当、2026-10-11。リードが追認し、ユーザーが hiragana.tsv の抜き取りと kana_style.tsv を確認した）

ユーザーの決定: 表記の既定は文化庁「公用文作成の考え方」（2022）に従う（補助動詞、形式名詞、多くの接続詞、一部の副詞はひらがな。本動詞と実質名詞は漢字）。評価セットは `eval/sets/hiragana.tsv`、結果は `eval/reports/phase1-hiragana.md`。

- **§5.3-5（BOS の規則）の変更**: 左文脈の最後の文が文末記号で終わっていないとき、**EOS への連接を使わずに**解析し、最良の終わり方（右ID）を `bos_right_id` とする。最良から 1000 以内の終わり方は `AnalyzedContext::bos_alternatives`（追加コスト付き）として残し、変換のラティスと LeftContextRanker は候補のうち最小の連接を使う。
  - 原因: 文が続くのに EOS を含めて終わり方を選ぶと、「若い」「困った」の末尾が連体形ではなく文末の形（終止形）に偏り、次の名詞への連接が悪くなって、助詞 + 1文字名詞の不自然な分割（`と気`、`と頃`、`は図`）が1位になった。
  - ADR 0002 §5.3-5 は「最後のトークンの右ID」だった。契約（`AnalyzedContext`）にフィールドを足した。
- **`KanaStyleRanker` を足した**（`core/src/ranker/kana_style.rs`、語のリストは `core/assets/kana_style.tsv`。並びは LeftContext の次）: 辞書のコストは解析用で、ひらがなの表記（あえて、ください、とき）が漢字の表記や不自然な分割より高い。候補の**語ごと**に、直前の語（先頭は左文脈の最後のトークン）との関係で加点する。
  - `phrase`（接続詞、副詞、複合助詞）: 読みと同じ全ひらがなの候補に、文脈によらず加点。
  - `formal_noun`（形式名詞）: 直前が連体修飾（終止形・連体形の動詞、形容詞、助動詞、連体詞、「の」、数詞）ならひらがなに加点。文頭や助詞の直後は実質名詞の漢字（時、物、所、訳）に加点。
  - `aux_verb`（補助動詞）: 直前が「て／で」ならひらがなに加点（連用形は、助動詞か「て／で」が続くときだけ）。それ以外の本動詞は漢字に加点。
- **ADR 0001 §4.6-7（かな候補をプールの末尾に置く）は変更していない。** かな候補は Ranker を通るので、`KanaStyleRanker` が score で調整できる。FR-05（機能語のひらがな）は、同じ仕組みで扱える。
- **既知の限界**: 語のリストは人が書いた表で、載っていない語には効かない。個人の好みは、学習と文章の取り込みで後から合わせる設計（未実装の部分は残課題）。

