# 殻の性能の基準値（Rust 移行。ADR 0041 §6.3）

`baseline.json` は、移行の前（Swift 版の殻）の性能。Rust 版の各レーンを統合するたびに、同じ計測を流して比べる（`cargo xtask perf compare --baseline eval/reports/shell-perf/baseline.json`）。

## 指標

| 指標 | 中身 | 道具 |
| --- | --- | --- |
| P1 | キー 1 回の殻の時間（Convert など返事を待つ要求を送ったキーを除く）。p50、p99、max。共通のシナリオ（`shell-core/scenarios/`）を替え玉のエンジンで再生し、全部を 100 回くり返す | `perf shell`。Swift 版は `macos/Sources/IkariShellBench/`、Rust 版は `shell-core/benches/shell_perf.rs` |
| P3 | 殻の起動（設定の解釈、キー割り当ての読み込み、ショートコードの表 2000 件、状態機械の生成、エンジンへの hello）。100 回 | 同上 |
| P4 | 最大 RSS（P1 のプロセス） | 同上 |
| P5 | エンジンとコアの性能（`check.sh --perf` の中身）。実辞書が要る | `perf engine` |
| P2 | 変換の往復（XPC 込み）。smoke を流すので、ユーザーの了解が要る | `perf smoke --allow-smoke` |

各指標は 5 回流した値の中央値と、最小・最大（ばらつき）を持つ。比べるのは中央値。

## 基準値を取り直す手順

```sh
cargo xtask perf shell --impl swift --out eval/reports/shell-perf/baseline.json
IKARI_DICT_DIR="$HOME/Library/Application Support/ikari-ime/dict" cargo xtask perf engine --out eval/reports/shell-perf/baseline.json
```

どちらも、ほかの区間の指標を残して混ぜる。ほかのレーンのビルドと CPU を取り合うと、ばらつきが増える。`meta.sections.<区間>` に、測っている間の負荷平均と、見えた重いプロセス（cargo、rustc、swift など）を記録している。

## 読むときの注意

- Swift 版の P1 は、状態機械（`InputStateMachine.handle`）の時間。本番では合成器（ローマ字）が Rust の FFI だが、測り手は `FakeComposer`（テストの替え玉）を使うので、合成器の時間と FFI の符号化は入っていない。Rust 版の P1 は `Shell::dispatch` 全体なので、合成器の時間を含む。比べるときはこの差を頭に入れる（Rust のほうが多くを測る）。
- 時計は `CLOCK_UPTIME_RAW`（Apple silicon で約 41.7 ns の刻み）。p50 が数百 ns の値は数刻みでしかない。
- Rust 版は `Shell` が stub の間（`"stub": true`）は結果に入れない。
- 許す幅（`tolerance.toml`）はユーザーが基準値を見て決める。ファイルがない間、`perf compare` は表だけを出して終了コード 0。形は `xtask/src/perf/compare.rs` の先頭。
