# ADR 0043: ビルドと検証のスクリプトを `cargo xtask` に寄せる

- 状態: 承認（実装済み。`scripts/check.sh` は `cargo xtask check` を呼ぶ包みで、`dict`・`data`・`eval`・`perf`・`cov` も `xtask` にある）
- 日付: 2026-10-30
- 対象: 方針、段、`check` の記録との関係（コードは変えない）。採ったときに触るのは、新しい crate `xtask`、`.cargo/config.toml`（別名 `xtask`）、`scripts/check.sh`・`scripts/build-dict.sh`・`scripts/fetch-data.sh`・`scripts/install-data.sh`・`eval/scripts/snapshot.sh`（中身を xtask に移し、薄い呼び出しにする）、`scripts/check-record.sh` と `.claude/hooks/guard-pr-merge.sh`（記録の書式の扱い）
- 前提: main = a43ccb1。ADR 0042（技術選定の方針。PR #137。main にはまだない）§1-5。ADR 0041（殻の Rust への移行）§6.3（性能の計測）
- タスク: #138

---

## 0. 現状の把握（researcher の棚卸しと、`scripts/check-record.sh` を読んだ範囲）
- Windows でも要るシェルのスクリプト: `scripts/check.sh`（274 行）、`scripts/check-record.sh`（40）、`scripts/build-dict.sh`（219）、`scripts/fetch-data.sh`（249）、`scripts/install-data.sh`（96）、`eval/scripts/snapshot.sh`（479）ほか。
- macOS 専用: `macos/scripts/*`（`build.sh` 340、install、smoke、gen-*）、`scripts/setup.sh`。
- `check.sh` は終わりに `CHECK_EXIT=<コード>` を出し、作業ツリーがきれいなら、git の共有ディレクトリの `ikari-check/<SHA>` に `CHECK_EXIT=<コード>` と `SCOPE=<full|no-swift>` の 2 行を書く（`check-record.sh` の `check_record_write`）。`--docs-only` は記録しない。
- `.claude/hooks/guard-pr-merge.sh` は、`gh pr merge` の前にこの記録を読んで止める。`.githooks/` は記録に依存しない。
- 分析の Python（`scripts/diag-report.py`、`eval/scripts/*.py`）は、殻とエンジンの実行に関わらない道具。

## 1. 問題の定義
- Windows の開発環境（ADR 0040。x64 の VM）では、bash のスクリプトがそのまま動かない。Windows でも `check` と辞書のビルドが要る（#138 の完了の条件「Windows でも `cargo xtask check` が通る」）。
- 一度に全部を書き換えると、毎日使う `check.sh` と、マージを止める記録の仕組み（#107）が同時に壊れうる。

## 2. 決定（提案）
1. workspace に `xtask` の crate（bin）を足し、`.cargo/config.toml` に `xtask = "run --package xtask --"` の別名を置く。依存は workspace にあるもの（`clap`、`serde`、`serde_json` など）に限る。
2. **段階的に包む**。各段で、シェルのスクリプトは `cargo xtask <下位コマンド>` を呼ぶだけの数行にして残す（手の覚えと、フック・文書の参照を壊さないため）。全部を移し終えた段で、シェルの包みを消すかは別に決める。
3. **`check` の記録の書式を保つ**。`cargo xtask check` も、同じ置き場所（`git rev-parse --git-common-dir` の下の `ikari-check/<SHA>`）に、同じ 2 行（`CHECK_EXIT=`、`SCOPE=`）を書き、最後に `CHECK_EXIT=<コード>` を標準出力に出す。`IKARI_CHECK_RECORD_DIR` も同じく効かせる。こうすれば `guard-pr-merge.sh` は変えずに済む。記録を書く側を xtask に移した段で、`check-record.sh` の書く関数を消し、読む関数（guard が使う）だけを残す。
4. macOS 専用のもの（`macos/scripts/*`、`scripts/setup.sh`）はシェルのまま（ADR 0042 §1-5）。`cargo xtask check` の中の swift test は、macOS のときだけ `swift test` を呼び、ほかの OS では飛ばして `SCOPE=no-swift` にする（今の `--no-swift` と同じ意味）。
5. 例外: `scripts/diag-report.py` と `eval/scripts/*.py` は移さない（ADR 0042 §3。分析の道具）。

## 3. 下位コマンドと段
| 段 | 下位コマンド | 中身 | 包むシェル |
| --- | --- | --- | --- |
| X0（ADR 0041 の M1-a の契約） | 骨組み | `check`、`dict`、`data`、`eval`、`perf` の入口だけ（`main.rs` と空のモジュール）。L-D と ADR 0041 の L-P が別のモジュールを持つため | — |
| X1 | `check` | fmt、clippy、cargo test、swift test（macOS のみ）、文書のリンクと ADR の索引の検査、`--docs-only`・`--real`・`--perf`・`--no-swift`。記録（§2-3） | `scripts/check.sh`、`scripts/check-record.sh` の書く側 |
| X2 | `dict` | 辞書のビルド | `scripts/build-dict.sh` |
| X3 | `data fetch`、`data install` | データの取得と配置 | `scripts/fetch-data.sh`、`scripts/install-data.sh` |
| X4 | `eval snapshot` | 全セットの 1 位の比較 | `eval/scripts/snapshot.sh` |
| L-P（ADR 0041 §6.3） | `perf shell`、`perf engine`、`perf smoke`、`perf compare` | 殻の性能の計測と基準値との比較 | `check.sh --perf` の `run_perf` は `perf engine` が包む |
| prof（#149） | `perf profile`、`perf alloc` | CPU プロファイル（samply。`cargo install samply --locked`。無ければ案内して止まる）と、ヒープ確保の回数とバイト数（dhat。`ikari-shell-core` の feature `dhat-heap`。道具のインストールは要らない）。どちらも上位の関数・確保の場所を文字の表にし、`--out` で `eval/reports/` に残せる。基準値の仕組み（`perf compare`）には載せない（値ではなく、どこが重いかを見る道具）。対象は殻のキー 1 回。`perf profile -- <コマンド>` で差し替えられる | — |
| cov（#158） | `cov` | テストの行カバレッジ（cargo-llvm-cov）。crate ごとの表、低いファイル、変換のロジックの小計。`--lm`（llama.cpp）、`--real`（実辞書。`check --real` と同じテスト）、`--out`（`eval/reports/` に記録）。目標値とゲートは持たない（`--fail-under` は任意）。`check` には入れない（遅い） | — |

- 最初は X1（`check`）と X2（辞書のビルド）。X1 は記録の書式を保つことを、`scripts/test-hooks.sh` の既存の検査で確かめる。
- 各段の完了の条件: 包んだシェルとの出力の同等（`check` は同じ SHA で `CHECK_EXIT` が一致、`dict` は出力の辞書のバイト列が一致）、macOS で `scripts/check.sh` が `CHECK_EXIT=0`、Windows の CI（ADR 0040 の W0）で `cargo xtask check --no-swift` が通る（W0 が入った後）。

## 4. 検討した案
| 案 | 判断 |
| --- | --- |
| **xtask（採る）** | Rust だけで Windows・macOS・Linux に通る。新しい道具を入れない。ADR 0042 §1-5 |
| `just` や `make` | 別の道具の導入が要り、中身はシェルのまま残るので Windows の問題が解けない |
| PowerShell と bash の二本立て | 同じ処理を 2 回書く。ADR 0042 の「境界を減らす」に反する |
| 一度に全部を xtask に置き換える | `check` の記録と guard が同時に壊れうる。段ごとに包む |

## 5. ユーザーに聞くこと
- なし（ADR 0042 §1-5 の決定の範囲。シェルの包みをいつ消すかは、全段が終わった後に聞く）。

## 参考
- [ADR 0041](0041-shell-state-machine-in-rust.md) §6.3、§7
- ADR 0042（PR #137）、ADR 0040（PR #126）。どちらも main にはまだない
- タスク: #138、#107（check の記録と guard）
