# 殻の性能の基準値（Rust 移行。ADR 0041 §6.3）

`baseline.json` は、移行の前（Swift 版の殻）の性能。Rust 版の各レーンを統合するたびに、同じ計測を流して比べる（`cargo xtask perf compare --baseline eval/reports/shell-perf/baseline.json`）。

**Swift 版の値（`swift.*`）は固定で、もう取り直せない。** Swift 版の測り手（`IkariShellBench`）は、測る相手の Swift の状態機械（`IkariShellCore`）と一緒に、移行の片付け（M5、#138）で消した。`perf compare` は、今の Rust 版をこの保存済みの値と比べる。`perf shell --impl swift` は誤りになる。

## 指標

| 指標 | 中身 | 道具 |
| --- | --- | --- |
| P1 | キー 1 回の殻の時間（Convert など返事を待つ要求を送ったキーを除く）。p50、p99、max。共通のシナリオ（`shell-core/scenarios/`）を替え玉のエンジンで再生し、全部を 100 回くり返す | `perf shell`。Rust 版は `shell-core/benches/shell_perf.rs`（Swift 版の測り手 `IkariShellBench` は M5 で消した） |
| P3 | 殻の起動（設定の解釈、キー割り当ての読み込み、ショートコードの表 2000 件、状態機械の生成、エンジンへの hello）。100 回 | 同上 |
| P4 | 最大 RSS（P1 のプロセス） | 同上 |
| P5 | エンジンとコアの性能（`check.sh --perf` の中身）。実辞書が要る | `perf engine` |
| P2 | 変換の往復（XPC 込み）。smoke を流すので、ユーザーの了解が要る | `perf smoke --allow-smoke` |

各指標は 5 回流した値の中央値と、最小・最大（ばらつき）を持つ。比べるのは中央値。

## 基準値を取り直す手順

殻の区間（`swift.*`）は取り直せない（上）。エンジンとコアの区間（P5）だけを取り直せる。

```sh
IKARI_DICT_DIR="$HOME/Library/Application Support/ikari-ime/dict" cargo xtask perf engine --out eval/reports/shell-perf/baseline.json
```

ほかの区間の指標を残して混ぜる。ほかのレーンのビルドと CPU を取り合うと、ばらつきが増える。`meta.sections.<区間>` に、測っている間の負荷平均と、見えた重いプロセス（cargo、rustc、swift など）を記録している。

## 読むときの注意

- Swift 版の P1 は、状態機械（`InputStateMachine.handle`）の時間。本番では合成器（ローマ字）が Rust の FFI だが、測り手は `FakeComposer`（テストの替え玉）を使うので、合成器の時間と FFI の符号化は入っていない。Rust 版の P1 は `Shell::dispatch` 全体なので、合成器の時間を含む。比べるときはこの差を頭に入れる（Rust のほうが多くを測る）。
- 時計は `CLOCK_UPTIME_RAW`（Apple silicon で約 41.7 ns の刻み）。p50 が数百 ns の値は数刻みでしかない。
- Rust 版は `Shell` が stub の間（`"stub": true`）は結果に入れない。
- 許す幅（`tolerance.toml`）はユーザーが基準値を見て決める。ファイルがない間、`perf compare` は表だけを出して終了コード 0。形は `xtask/src/perf/compare.rs` の先頭。

## Rust 版の P1 の内訳と、条件を揃えた比較（L-A2 のあと）

Rust 版の測り手（`shell-core/benches/shell_perf.rs`）は、替え玉のエンジン（`ScriptedTransport`: 要求の復号、台本の照合、返事の組み立てと符号化）で使った時間を記録し、JSON に `*_without_transport` として出す（`p50_ns` などは従来どおり、引かない値）。Swift 版の測り手はエンジンへの口を持たないので、比べるのは `*_without_transport`。`--detail` を付けると、キーごとの中央値の上位 25 件と分布を標準エラーに出す。

```sh
cargo bench -p ikari-shell-core --bench shell_perf -- --metric p1 --detail
```

分かったこと（`--detail`、1 回）:

- キーごとの中央値は、p10 334 ns、p50 791 ns、p90 1.4 us、p99 6.1 us。p99 の側は「確定のキー」（候補の確定、`Enter`、数字キー、Command+文字など）で、中央値 4〜6 us。うち 1.9〜2.4 us は替え玉のエンジンの側（確定と診断ログの 2 通の復号と応答）で、残りの約 4 us が殻の側（診断ログの事象の組み立て、`v1::DiagEvent` への写し、符号化、確定の要求）。
- 2,000 文字を超える文脈のキー（`surrounding text is cut to the engine limit`）は 1 回 45〜56 us。見た目の文字（書記素）で左右 2,000 文字ずつを歩くため。1 合成の最初の 3 打鍵だけで、文脈が短ければ起きない。
- 設定のメニュー操作（キー割り当て、入力方式の切り替え）は 18〜22 us。設定の再読み込みを含み、通常の打鍵では起きない。

条件を揃えた値（同じ負荷、5 回の範囲。負荷平均が 20 前後のほかの作業と同時だったため、絶対値のばらつきは大きい）: p50 は含む値 0.67〜0.75 us に対して、替え玉のエンジンを引いた値 0.50〜0.54 us。Swift 版 417 ns との差は、合成器（ローマ字）と proto の組み立てを Rust 版だけが含むため。
