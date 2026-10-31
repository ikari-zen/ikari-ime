# プロファイラの導入（issue #149）

`cargo xtask perf profile`（CPU。samply）と `cargo xtask perf alloc`（ヒープ確保。dhat）。基準値ではなく、どこが重いかを見る道具。
このディレクトリの `cpu-*.md`、`alloc-*.md` は、道具の動作確認を兼ねた最初の記録（2026-10-31）。**基準値ではない**（負荷平均が高い中で測った）。

## スパイクの結果（macOS、Apple Silicon）

| 道具 | 結果 | 必要なもの・手間 |
|---|---|---|
| samply 0.13.1 | 動いた（sudo なし）。`samply record --save-only` で記録できる | `cargo install samply --locked`（約 1 分）。関数名と行には `debug = 1` が要る（`CARGO_PROFILE_BENCH_DEBUG=1` を xtask が付ける。`Cargo.toml` は変えない）。`--unstable-presymbolicate` で記号表（`*.syms.json`）も出るので、CLI だけで上位の関数を文字にできる。記録の形（Firefox Profiler 形式）と記号表は samply の「不安定」な出力で、`xtask` の `profile.rs` が読む。`-o x.json`（`.gz` なし）なら圧縮されない。システムのライブラリ（libsystem_*）は記号がなく、ライブラリごとに 1 つにまとめて出す。Windows: `samply record` のヘルプに Windows 用の指定（`--windows-symbol-server`）があるが、この環境では試していない |
| dhat 0.3.3 | 動いた。`ikari-shell-core` の feature `dhat-heap`（任意の依存。通常のビルドには入らない）で、`dispatch` 1 回の確保の回数とバイト数（`HeapStats`）と、確保の場所（`dhat-heap.json`）が出る | 道具のインストールは不要。関数名と行には `debug = 1`。`dhat-heap.json` の `pps`（場所）と `ftbl`（フレーム）から、呼び出しの列を文字で取り出せる。測り手が遅くなる（時間の数値は使えない） |
| cargo flamegraph 0.6.14 | **不可**。sudo は要らず（Instruments の `xctrace` で記録する）、記録は取れるが、`unable to collapse generated profile data: Unpaired tag: frame` で図にならない | 直し方は調べていない |
| Instruments（`xctrace`） | 試していない（後回し） | — |
| divan / criterion | 試していない（第 2 段） | — |

## 見方

```sh
cargo xtask perf profile --out eval/reports/profile/cpu-shell-YYYY-MM-DD.md
cargo xtask perf alloc --detail --out eval/reports/profile/alloc-shell-YYYY-MM-DD.md
samply load target/perf-tools/profile.json     # ブラウザで見る
```

- 測る前に負荷平均を見る（記録の見出しに自動で入る。4 以上なら注意が付く）。
- `perf profile` は、既定で `Shell::dispatch` を含むサンプルだけを数える（`--focus`）。測り手の準備（シナリオの読み込み、`Shell::new`）を除く。
- `perf profile -- <実行ファイルと引数>` で、ほかの対象（例: 変換の `ikari-eval`）も測れる。`debug = 1` つきでビルドしたもの（`CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release -p ikari-eval`）。
- `perf alloc` の場所の表は、確保の呼び出しの列のうち、いちばん内側の ikari の関数と行にまとめる。替え玉のエンジン（`ScriptedTransport`）の確保も `dispatch` の中から呼ばれるので入る。

## 最初の記録で分かったこと（2026-10-31。負荷平均 4〜6.6。基準値ではない）

- 確保（[alloc-shell-2026-10-31.md](alloc-shell-2026-10-31.md)）: `dispatch` 1 回あたりの確保は p50 15 回、p99 144 回、最大 214 回。多いキーは確定のキー（Enter、数字キー、Cmd+C など。138〜214 回）。
  - 上位の場所は診断ログではなく、`CandidateItem::clone`、`KeyKind::clone`、`menu::radio` / `menu::build`、`SettingsSnapshot::clone`、`MenuItem::clone`（候補とメニューと設定の複製）。
  - 診断ログ（`DiagEpisode::event` の中）は、`dispatch` 内の確保の約 3.8%、`engine::codec` を通るものは約 2.9%（重なりあり）。確定のキーだけで見た割合は、まだ出していない。
- CPU（[cpu-shell-2026-10-31.md](cpu-shell-2026-10-31.md)）: `dispatch` の時間のうち、**`Background::spawn`（スレッドの生成）が約 44%**、`libsystem_kernel`・`libsystem_malloc`・`libsystem_pthread` の記号なしの部分が自分の時間の約 66%（スレッド生成とヒープ確保）。`unicode_segmentation` の `grapheme_count`（`start_conversion` の中）が約 7%。
  - 推測: 測り手の P1 が、キーごとに背景のスレッドを作る経路（`BackgroundMode::Thread`）を通っている。実機の経路と同じかは、確かめていない。
