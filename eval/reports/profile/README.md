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

## 静かな状態の記録（2026-10-31。負荷平均 2.5〜2.8。基準の候補）

`alloc-shell-quiet-2026-10-31.md`、`cpu-shell-quiet-2026-10-31.md`（コミット 5020a52）。最初の記録との違いは、暴走していた別のプロセスを止めたあとに測ったこと。確保の回数は最初の記録と同じ（`dispatch` 1 回あたり p50 15、p99 144、最大 194）で、負荷に左右されにくい。

- **確保**: 上位は、候補・メニュー・設定の複製（`CandidateItem::clone` 3.8 回/dispatch、`KeyKind::clone` 1.4、`menu::radio` 1.4、`SettingsSnapshot::clone` 1.9、`MenuItem::clone` 1.2）。バイト数では `menu::build`（125 MB/測定）と `CandidateItem` の `to_vec`（75 MB）が大きい。診断ログの組み立て（`DiagEpisode::event`）は 0.65 + 0.44 回/dispatch で小さい。
- **CPU**: 呼び出しを含めた時間の 40.6% が `Background::spawn`（スレッドの生成。カーネルと pthread の記号なしの部分が自分の時間の 44%）。次いで `Composer::kana`／`refresh_composing` 11.3%、`is_valid_reading` 10.8%、`grapheme_count` 8.9%（`unicode_segmentation` の書記素の走査）。
- **読み方の注意**: `Background::spawn` を呼ぶのはキーごとではなく、入力の開始（エンジンへの hello）、郵便番号の問い合わせ、絵文字の表、設定の変更のとき（`controller/mod.rs` の 537、676、710、777、811、819、851 行）。測り手のシナリオはこうした場面を繰り返し含むので、本番のキー 1 回の平均より割合が大きく出ている（推測。本番の頻度は未測定）。替え玉のエンジン（`ScriptedTransport`）と、シナリオの周辺テキストの変換（`ScenarioSurrounding::to_host`）の確保も `dispatch` の中に入る。

- `alloc-shell-after-settings-menu-2026-10-31.md`: 設定とメニューの複製を減らした前後の確保（issue #161）。

## 起動と定常を分けた測り方（2026-10-31。issue #161、#149）

最初の記録の「dispatch 1 回あたり」の場所の表には、測り手が dispatch ごとに作り直す `Shell` の**起動の費用**（`Activate`、最初の `finish` のメニュー、`Background::spawn` の hello、設定の複製）が混ざっていた（確保の回数の p50、p99 は元から `Activate` を除いていたので、混ざっていたのは場所の表と CPU の割合）。測り手（`shell-core/benches/shell_perf.rs`）は、イベントを 3 つに分けて数える。呼び出しの列に名前が残る別々の関数で呼ぶ（`perf alloc`、`perf profile` はこの名前で分ける）。

| 区分 | 関数 | 中身 |
|---|---|---|
| 起動 | `startup_dispatch` | シナリオごとの最初の `Activate` と、手順の中の `Activate` |
| 最初の 1 打 | `first_dispatch` | 起動のあとの最初の `Activate` 以外のイベント。取り直しの結果（エンジンの状態など）が入ってメニューが作り直される |
| 定常 | `steady_dispatch` | それ以降の打鍵ごと |

- 測り手の JSON に `startup`、`first`、`steady`（`dispatches`、`p50_ns`、`p99_ns`、`max_ns`、alloc のときは `alloc`）を足した。従来の指標（`p50_ns` など）の名前と意味、基準値の形は変えていない。
- `perf alloc` は既定で 3 つの表を出す（`--within` を指定すると従来の 1 つの表）。`perf profile` の既定の範囲は `steady_dispatch`（起動は `--focus shell_perf::startup_dispatch`）。
- 落とし穴 2 つ（直した）: 中身が同じ関数はリンカが 1 つにまとめる（名前が区別できなくなる）。末尾呼び出しにすると枠が消える。どちらも `black_box` で避けた。
- 記録: `alloc-shell-steady-2026-10-31.md`、`cpu-shell-steady-2026-10-31.md`、結論は `steady-state-findings-2026-10-31.md`。
- `after-candidate-clones-2026-10-31.md`: 候補と変換の状態の複製を減らした前後（確保と CPU と P1。issue #161）。
- `typing-findings-2026-10-31.md`: 入力中のローマ字の 1 打の測定（`alloc-shell-typing-2026-10-31.md`、`cpu-shell-typing-2026-10-31.md`）と、直す対象の見立て（issue #161）。測り手に入力中の台本を足した（`TYPING_SCRIPTS`、`perf profile --focus shell_perf::typing_`）。
- `after-typing-2026-10-31.md`: 入力中の毎キーの無駄（`refresh_composing`、`prefetch_if_changed`）を減らした前後（issue #161）。
- `after-romaji-push-2026-10-31.md`: ローマ字の `push` を Trie にした前後と、旧実装との同値の確かめの範囲と件数（issue #161）。
