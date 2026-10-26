# フェーズ1 完了条件の確認（ADR 0002 §17.4）

- 辞書: `data/build/dict`（形式 v2、`dict-builder/policy/chive.tsv`、`vectors.bin` 付き、grammar_id=0xb570a1f4bdffbbfb）
- 構成: `--ranker full`（LeftContext → DocumentCache → Learning → Profile → Topic。LM なし）
- 実行日: 2026-10-11 / release ビルド

## 合否

| 条件 | 結果 | 数値 |
|---|---|---|
| `cargo test --workspace` が全件成功 | **合格** | 669 件成功、0 件失敗、3 件 ignored（実モデルを要するテスト） |
| `cargo clippy --workspace --all-targets` 警告ゼロ、`cargo fmt` | **合格** | - |
| `homophones.tsv` で a-03 / a-05 / h-07 / h-09 が 1 位 | **合格** | 4 件とも 1 位 |
| フェーズ0で 1 位だった h-01 / a-01 が 1 位のまま（`--baseline eval/reports/phase0-baseline-cases.tsv --fail-on-regression`） | **合格** | 終了コード 0、回帰 0 件 |
| a-02 の学習シナリオが通る | **合格** | `eval/scenarios/a-02.tsv`、probe 4 件すべて OK |
| `--profile-ab` で否定ケースが下がらない | **合格** | homophones.tsv 3 件、ext の dev 36 件 / test 8 件のいずれも悪化 0、否定・文法の回帰 0 |
| LM なしの既定構成で、2000 文字の文脈を付けて warm p99 ≤ 16ms | **合格** | warm p99 0.12ms、cold（文ごとのキャッシュが効かない最悪側）p99 1.5〜3.9ms（下を参照） |
| `eval/reports/phase1-topic.md` と ADR 0003 の案 | **未作成**（このタスクの範囲外。材料は `phase1-quality.md` §4 と `phase1-ajimee.md`） | - |

`homophones.tsv` 全体の 1 位正解率は 21.4%（フェーズ0）→ **85.7%（12/14）**、MRR 0.391 → 0.917。1 位でないのは h-08（プロファイル、3 位）と a-04（辞書にない複合動詞、2 位）。

## 2000 文字の文脈での遅延

`eval/scripts/make_long_context_set.py` で、評価セットの左文脈を 2000 文字に伸ばした（`docs/` の日本語の文を材料にし、材料が足りなくなる 2 周目以降は文頭にかな 1 字を足して、解析器の文ごとのキャッシュが効かないようにした）。

```
eval/scripts/make_long_context_set.py eval/sets/homophones-ext.tsv --text docs/adr/0002-phase1-architecture.md \
    --text docs/adr/0001-phase0-architecture.md --text docs/ikari-ime_spec.md --chars 2000 > long-ext.tsv
ikari-eval --set long-ext.tsv --ranker full --repeat 20
```

| セット | 対象 | サンプル数 | p50 | p99 | 最悪 (ms) |
|---|---|---|---|---|---|
| homophones.tsv（14 件） | warm | 280 | 0.074 | 0.109 | 0.112 |
| 〃 | cold（各ケースの初回） | 14 | 1.181 | 3.853 | 3.853 |
| homophones-ext.tsv（876 件） | warm | 17520 | 0.078 | 0.123 | 0.380 |
| 〃 | cold | 876 | 1.055 | 1.515 | 4.179 |

文脈が短い通常の評価（`homophones.tsv`）では warm p99 0.089ms、cold p99 1.6ms。最大 RSS は 36.7MiB（mmap のページを触った分）。

## シナリオ a-02

```
# ikari-eval scenario
- steps: 6 / probe: 4 / 失敗: 0

| 行 | step | 文脈 | 読み | 表記 | 期待 | 順位 | 1位 | 結果 |
|---|---|---|---|---|---|---|---|---|
| 6 | probe | その話は | はじめてしった | 初めて知った | top1 | 1 | 初めて知った | OK |
| 7 | probe | ゲームを | はじめてしった | 始めて知った | not-top1 | 5 | 初めて知った | OK |
| 8 | commit | ゲームを | はじめてしった | 始めて知った | - | - | - | 確定 |
| 9 | commit | ゲームを | はじめてしった | 始めて知った | - | - | - | 確定 |
| 10 | probe | ゲームを | はじめてしった | 始めて知った | top1 | 1 | 始めて知った | OK |
| 11 | probe | その話は | はじめてしった | 初めて知った | top1 | 1 | 初めて知った | OK |
```

## homophones.tsv（full、`--baseline … --fail-on-regression --profile-ab`）

# ikari-eval report
- set: eval/sets/homophones.tsv (14 cases) / dict: sudachi-core-20260723 grammar_id=0xb570a1f4bdffbbfb / ranker: composite
- 構成: ranker=full(left_context,doc_cache,learning,profile,topic) analyzer=lattice rewriters=5 vectors=yes lm=none profile=per-case
- split: all / 除外（unreachable）: 0 件
- build: release / repeat: 20 / max_rank: 50 / 最大RSS: 36.7 MiB

## 正解率
| 区分 | 件数 | 1位正解 | 1位正解率 | MRR |
|---|---|---|---|---|
| 全体 | 14 | 12 | 85.7% | 0.917 |
| kind:プロファイル | 1 | 0 | 0.0% | 0.333 |
| kind:分割 | 1 | 0 | 0.0% | 0.500 |
| kind:否定 | 2 | 2 | 100.0% | 1.000 |
| kind:学習 | 1 | 1 | 100.0% | 1.000 |
| kind:文法 | 1 | 1 | 100.0% | 1.000 |
| kind:機能語 | 1 | 1 | 100.0% | 1.000 |
| kind:話題 | 7 | 7 | 100.0% | 1.000 |
| tag:怒り | 5 | 4 | 80.0% | 0.900 |

## 応答時間（convert〜1ページ目, ms）
| 対象 | サンプル数 | p50 | p99 | 最悪 |
|---|---|---|---|---|
| warm | 280 | 0.049 | 0.089 | 0.093 |
| cold（各ケースの初回） | 14 | 0.149 | 1.556 | 1.556 |

## ケース別
| id | kind | 文脈 | 読み | 期待 | 1位 | 順位 | warm p99 |
|---|---|---|---|---|---|---|---|
| h-01 | 話題 | ユーザー辞書に単語を登録するときは | ひんし | 品詞 | 品詞 | 1 | 0.054 |
| h-02 | 話題 | 敵の攻撃を受けて兵士は | ひんし | 瀕死 | 瀕死 | 1 | 0.040 |
| h-03 | 話題 | システムの | こうせい | 構成 | 構成 | 1 | 0.076 |
| h-04 | 話題 | 原稿の誤字を | こうせい | 校正 | 校正 | 1 | 0.084 |
| h-05 | 話題 | 質問への | かいとう | 回答 | 回答 | 1 | 0.093 |
| h-06 | 話題 | 冷凍した肉を | かいとう | 解凍 | 解凍 | 1 | 0.091 |
| h-07 | 否定 | 要件をまとめてドキュメント化 | しよう | しよう | しよう | 1 | 0.051 |
| h-08 | プロファイル | 外部APIの | しよう | 仕様 | 使用 | 3 | 0.056 |
| h-09 | 機能語 | この | の | の | の | 1 | 0.010 |
| a-01 | 話題 | IMEの変換では単語の | ひんし | 品詞 | 品詞 | 1 | 0.039 |
| a-02 | 学習 | その話は | はじめてしった | 初めて知った | 初めて知った | 1 | 0.072 |
| a-03 | 文法 | やっと春に | なった | なった | なった | 1 | 0.034 |
| a-04 | 分割 | 一語ずつ確定しながら | かきすすめる | 書き進める | 書きすすめる | 2 | 0.087 |
| a-05 | 否定 | 今日はここまでと | しよう | しよう | しよう | 1 | 0.052 |

## プロファイルあり・なしの差分（--profile-ab）
- 対象 3 件 / 改善 0 / 変化なし 3 / 悪化 0 / **否定・文法ケースの回帰 0 件**

| id | kind | profile | 期待 | なし | あり | 変化 |
|---|---|---|---|---|---|---|
| h-07 | 否定 | it | しよう | 1 | 1 | - |
| h-08 | プロファイル | it | 仕様 | 3 | 3 | - |
| a-05 | 否定 | it | しよう | 1 | 1 | - |

## 回帰（基準で1位だったケース）
- 基準で1位: 3 件 / 回帰: 0 件 / 基準にあって今回未実行: 0 件 / 基準にない新規: 0 件
- 回帰なし
