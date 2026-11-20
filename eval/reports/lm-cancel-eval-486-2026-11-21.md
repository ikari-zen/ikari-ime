# LM 推論の取り消し（ADR 0074）の LM ありの評価（#486、§9.4・§9.5。§9.3 は未実施）

LM なしの記録は [lm-cancel-snapshot-486-2026-11-20.md](lm-cancel-snapshot-486-2026-11-20.md)。

## 1. LM ありの snapshot（§9.4）

- 基準: `873865b`（取り消しの前）。今回: `66072dc`（main の先頭）。
- 構成: `--lm <zenz-v3.2-small-Q5_K_M.gguf>`（実機の models を読むだけ）、`--ajimee` なし、`--smoke` なし、`--no-ref`。学習なし（実機の学習データは使っていない）。辞書は同じ `data/build/dict`。LM の締め切りは snapshot の既定（予算 2000ms、締め切り 2000ms、時間切れ 0 になるまで最大 3 回）。
- コマンド: 基準のコミットで `eval/scripts/snapshot.sh <出力> --lm <gguf> --no-ref`、先頭で `eval/scripts/snapshot.sh --compare <基準の出力> <出力>`。評価は 1 本ずつ順番に流した。
- 結果: **差分 0（一致）**。全構成の `--out-cases` 列 1〜7（1 位と順位）、segments、シナリオ 16 本（LM あり・なし各 8）、CLI golden 48 入力がすべて一致。LM の構成はすべて時間切れ 0、1 回目で有効。1 位の悪化は 0。

| LM ありの構成 | ケース数 | 1 位正解（基準 = 今回） | warm p99 ms（基準 → 今回） |
| --- | ---: | ---: | ---: |
| homophones-full-lm | 14 | 13 | 35.2 → 34.4 |
| ext-dev-topic-lm | 1004 | 892 | 34.0 → 31.8 |
| ext-test-topic-lm | 262 | 228 | 34.2 → 31.4 |
| hiragana-topic-lm | 140 | 136 | 33.9 → 31.1 |
| verbs-full-lm | 120 | 98 | 39.3 → 35.9 |

- 速さの警告はなし（基準の 1.1 倍超の悪化なし）。LM なし構成も同じ実行で一致。

## 2. §9.3 再生の評価: 実施できなかった

ADR 0065 §8 の表の「1a 検証の口」（遅延の包み `DelayedScorer`、`zenz.debug_infer_scale` / `debug_infer_extra_ms`、`eval/src/replay.rs`、`eval/data/` の分位数の表）が、リポジトリにまだ無い（`debug_infer`、`DelayedScorer`、再生のモードを grep しても ADR 以外に出ない。`eval/reports/lm-x-sweep-*.md` も無い）。
`eval/scripts/prefetch_s1/` は文脈の許容の評価（ADR 0038）の前例で、打鍵の間隔と Space までの間を再生する道具ではない。このため、曲線のデータは取っていない。X と Q2 には触れていない。
これは新しい機能の実装（`lm` の包み、`ikari-eval` の新しいモード、製品の `Engine` の先行計算・待ち行列・30ms の待ちを時刻つきで通す再生）で、ADR 0065 §8 では 1a のレーン（時刻の組み立ては Opus 可）。

## 3. `scripts/check.sh --real --perf`（§9.5）

- `scripts/check.sh --real --perf --no-swift`（`66072dc`）: **CHECK_EXIT=0**。実データのテスト（`--ignored`）は全部通った（失敗 0）。性能のテストも通った（perf_smoke p50 0.33ms / p99 0.44ms、segment_perf の first / resize / reconvert は p99 0.13 / 0.33 / 0.12ms）。
- `--no-swift` なので swift test は含まない。swift test はフルの `scripts/check.sh`（PR の確認）で流す。
- モデルは `data/raw/zenz/`・`data/raw/kanakanji-edge/` の既定の場所（メインのリポジトリの `data/` へのシンボリックリンク）。#491（単体の `cargo test -p ikari-engine` で落ちるテスト）は、単体では流していない（未確認）。この流し方（workspace、LM つき）では落ちなかった。

## 4. 分かったこと／推測／ユーザーの決定

- 分かったこと: LM ありでも、取り消しの前後で 1 位・順位・シナリオ・CLI は同じ。`--real --perf` は全部通る。
- 推測: 速さの揺れは、負荷（流している間の load average は 3〜8）による。
- ユーザーの決定が要る点: §9.3 の再生の評価の道具（ADR 0065 の 1a）を先に作るか（作るなら、リードが Opus のレーンを組む）。X と Q2 はその曲線のあと。
