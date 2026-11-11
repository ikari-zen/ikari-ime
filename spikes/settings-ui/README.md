# 設定画面の方式のスパイク（W4-s0、issue #322、ADR 0040 §2.5 / S11）

**捨てるコード。main に入れない。** ブランチ `spike/w4-s0-settings-ui`。結果は ADR 0040 §2.5 に追記する（末尾の表の雛形）。

Mac では `cargo check --target x86_64-pc-windows-msvc` と `cargo clippy` が通るところまで確かめてある。**動かしたことはない（画面の見た目も含めて、すべて未確認）**。実機で最初に動かすときに、ビルドの失敗や実行時のエラーが出うる。

## 前提（ADR 0040 §2.5）

- 設定の中身（検証、整形、読み書き、変更の組み立て）は Rust の `shell-core/src/settings/` を共有する。Windows で作るのは**画面だけ**。画面の相手は `windows/engine-host`。
- 画面に出す設定の一覧は Rust が持つ（`specs.rs`。PR #321、ブランチ `feat/w4s1-settings-registry`）。画面は種類（`SettingKind`: `Bool`、`Int{min,max}`、`Enum`、`BundleIdList`、`KeyChord`）から部品を選ぶ。
- 見た目のきれいさを優先する（保守性を妥協してよいのは UI だけ。CLAUDE.md）。ほかに、設定画面の起動の速さと配布の軽さを見る。

## 候補の比較（コードを書く前の調べ）

「事実」は一次情報（crates.io、各リポジトリの README、ライセンスのファイル。2026-11-12 に確認）で見たこと。「推測」は、見ていない、または実機で確かめていないこと。

| 軸 | U-a: WebView2（wry + tao。HTML と CSS） | U-b: WinUI 3（Windows App SDK） | U-c1: Slint | U-c2: egui（eframe） | U-c3: iced | U-d: Win32 の自前（`windows` crate） |
| --- | --- | --- | --- | --- | --- | --- |
| 見た目 | CSS で自由。Windows 11 風にするのは自前（このスパイクは Segoe UI Variable、角丸、アクセント色を手書き） | Fluent そのもの（推測。最も Windows らしい） | 標準の部品に Fluent スタイルがある（推測。公式の説明どおりなら Windows 11 風。実機で確認） | 即時モードの独自の見た目。Windows らしさは弱い（推測） | 独自のテーマ。Windows らしさは弱い（推測） | 標準コントロール（古い見た目）。きれいにするには自前で描く |
| Rust との境界 | 同じ exe の中（wry の IPC。JS との文字列のやり取り） | C# の殻を別に持つ（FFI かパイプ）。WinUI 3 の XAML を Rust から直接使う公式の方法は、私の知る限りない（推測） | なし（Rust だけ。画面は `.slint` かマクロ） | なし | なし | なし |
| ビルドの道具 | Rust だけ（実行時に WebView2 ランタイムが要る） | .NET と Windows App SDK、MSIX か非パッケージのための追加の設定 | Rust だけ（`slint` crate） | Rust だけ | Rust だけ | Rust だけ |
| アクセシビリティ（ナレーター） | ブラウザの機能（ARIA）に乗る。HTML の意味づけが正しければ強い（推測） | OS 標準の UI Automation（推測。最も強い） | AccessKit（README の `accessibility` feature）。読み上げの品質は実機で確認 | AccessKit。eframe では既定で有効（事実: egui の README） | README に記載なし（推測: 未対応か弱い） | 標準コントロールなら OS が対応。自前描画は全部自分で実装 |
| 日本語入力の欄での IME | ブラウザの標準の入力欄。変換中の表示と候補窓の位置は Chromium が処理（推測: 最も安定） | 標準の `TextBox`（推測: 安定） | winit 経由。変換中の文字の表示と候補窓の位置は**未確認**（要・実機） | winit 経由。日本語の字形に CJK フォントの読み込みが要る（推測。egui は既定のフォントに CJK を持たない）。IME は**未確認** | winit 系。**未確認** | `WM_IME_*` を自分で処理するか、標準の編集コントロール |
| ライセンス | wry、tao は Apache-2.0 / MIT（推測。crates.io で要確認） | MIT 系（推測） | **GPL-3.0-only / Royalty-free 2.0 / 商用の 3 択**（事実: crates.io と README。Royalty-free は表示（attribution）が条件。本プロジェクトは Apache-2.0 で配るので、どれで使うかをユーザーが決める必要がある） | MIT / Apache-2.0（事実: README） | MIT（推測） | MIT / Apache-2.0（`windows` crate） |
| 成熟度 | wry 0.57.2 が 2026-11-10（事実: crates.io）。活発 | 成熟（推測） | 1.18.1、2026-09-21（事実）。デスクトップの製品の前例あり（推測） | 0.37.0、2026-10-15（事実） | 0.14.0、2025-12-07（事実）。README に「experimental」（事実） | — |
| 配布の大きさの見込み | exe は小さい（数 MB 程度の推測）。ランタイムは Windows 11 に入っている見込み（推測、未確認） | .NET と WinAppSDK のランタイムの同梱か、利用者に入れてもらう（大きい。推測） | exe に全部入る。数 MB から十数 MB（推測） | 同上（推測） | 同上（推測） | 最小 |
| 候補窓との共通化 | 候補窓の方式 A（#319）と土台（tao + wry）を共有できる | 共有できない | 共有できない | 共有できない | 共有できない | 候補窓の方式 B と Win32 の土台を共有できる |

### 最小の実装を作る 2 つ

**U-a（WebView2）と、U-c1（Slint）**。理由:

1. **U-a**: 見た目の自由度が最も高く（CSS）、ナレーターと日本語の入力欄をブラウザの標準の仕組みに任せられる（推測）。候補窓の方式 A と土台を共有できる。配布は exe 1 つとランタイム（Windows 11 に入っている見込み）。保守性の弱さ（HTML と Rust の文字列のやり取り）は UI なので許容範囲。
2. **U-c1（Slint）**: Rust の GUI crate のうち、標準の部品が Fluent スタイルを持つ（見た目で最も近い見込み）、AccessKit でアクセシビリティを持つ、Rust だけで閉じる。egui は CJK フォントの手当てが要り、見た目も標準らしくない（推測）。iced は 0.x で experimental と README にあり、アクセシビリティの記載がない。
3. **U-b（WinUI 3）を作らない理由**: C# と .NET のビルド、ランタイムの配布が増え、ADR 0042（OS に依存しない判断は Rust。言語をまたぐ境界は細く）と合いにくい。見た目と ナレーターは最強の見込みなので、**U-a と U-c1 の結果が悪かった場合の備え**として残す。このスパイクでは作らない。
4. **U-d（Win32 の自前）を作らない理由**: 保守性が最も低く、見た目をきれいにするほど自前で描く量が増える。

**懸念（先に書いておく）**: Slint のライセンスが 3 択で、GPL-3.0 は Apache-2.0 の配布物と混ぜると全体が GPL になる（推測。法的な確認は未了）。Royalty-free の条件（表示）で足りるかを、採用の前にユーザーが決める。結果が良くても、egui（MIT / Apache）が代替の候補になる。

## 何を作ったか

同じ固定のデータ（`src/model.rs`）の設定 6 種類を、2 つの exe で出す。キーと範囲は `specs.rs` の実際のもの。

| 種類 | 設定（固定のデータ） | U-a: `settings-webview` | U-c1: `settings-slint` |
| --- | --- | --- | --- |
| 真偽 | `paging.wrap` | スイッチ（checkbox + CSS） | `Switch` |
| 整数と範囲 | `paging.size`（1..=9） | スライダと数の欄 | `SpinBox` |
| 列挙 | `input.method`（romaji / kana） | `select` | `ComboBox` |
| 文字列 | `spike.note`（日本語の IME の確認用） | `input type=text lang=ja` | `LineEdit` |
| キー割り当て | `keymap.convert` | 読み取り専用の欄で `keydown` を取る | `FocusScope` の `key-pressed` |
| Bundle ID の一覧 | `emoji.shortcode_exclude` | 一覧 + 追加欄 + 削除ボタン | `ListView` + 追加欄 + 削除ボタン |

- 変更は標準出力に `CHANGE <key><TAB><value>` で出る（値は `settings.tsv` の書き方に寄せた）。
- `src/html.rs` は U-a の HTML、`src/bin/webview.rs` は U-a の窓、`src/bin/slint.rs` は U-c1（画面の定義は `slint!` マクロの中）、`src/metrics.rs` は時間とメモリの計測（候補窓のスパイクと同じもの）。
- **入れていないもの**: `shell-core` との接続（パイプ、`SettingsIntent`）、ユーザー辞書の表、学習、「このアプリについて」、文言の鍵、多言語化。方式の比較には要らないため。
- 両 exe は別の bin にしてある（配布の大きさを方式ごとに測るため）。リリースの設定（`opt-level = "s"`、LTO、strip）は両方で同じ。
- `.cargo/config.toml` で `SLINT_STYLE=fluent` にしている（U-c1 のスタイル。ライト・ダークはシステムに従う見込み。未確認）。

## 実機の準備

1. Windows 11 x64 の VM（miniPC 上。ADR 0040 §2.6）。スナップショットを取っておく。
2. Rust（`rustup`、`stable-x86_64-pc-windows-msvc`）と Visual Studio Build Tools（「C++ によるデスクトップ開発」）。
3. WebView2 ランタイム（Evergreen）。Windows 11 には通常入っている（**未確認**。VM のイメージによる）。
4. 日本語の入力: Microsoft IME（標準）。**ikari 自身を入れた状態での確認は、TIP ができてから別に行う**（ADR 0040 §2.5 の「ikari 自身を使って日本語が打てるか」）。今回は Microsoft IME で、欄が IME を受け付けるかを見る。
5. ナレーター（Win+Ctrl+Enter）。
6. このブランチを clone する。

## ビルドと実行

```powershell
cd spikes\settings-ui
cargo build --release
Get-Item target\release\settings-webview.exe, target\release\settings-slint.exe | Select-Object Name, Length

# 画面を出して操作する（閉じるまで残る。閉じるときに at-close のメモリが出る）
.\target\release\settings-webview.exe
.\target\release\settings-slint.exe

# 自動で測る（最初の描画のあと 3 秒待ってメモリを測り、終わる）
.\target\release\settings-webview.exe --settle-ms 3000 --exit-after-settle
.\target\release\settings-slint.exe --settle-ms 3000 --exit-after-settle
```

出力の読み方（`RESULT` の行を集める）:

| 出力 | 意味 |
| --- | --- |
| `RESULT A first_paint_from_main` / `RESULT B(rendered) first_paint_from_main` | `main` の先頭から最初の描画まで（QPC）。A は JS で `requestAnimationFrame` を 2 回待った時点（実際の画面への反映より少し早い近似）。B は Slint のレンダリング通知の `AfterRendering`（通知が使えないときは `B(loop-start)` で、イベントループの開始時の近似） |
| `RESULT A/B... cold_start_total` | プロセスの作成から最初の描画まで。**ユーザーが体感する起動時間に最も近い** |
| `RESULT A webview_created`、`RESULT B window_created` | 窓（A は WebView）を作るまで |
| `RESULT MEM <tag> total` | 自分と子孫のプロセス（A は `msedgewebview2.exe` 群を含む）の合計。`private` を常駐のメモリの目安にする。`first-paint`、`settled`（3 秒後）、`at-close` |
| `CHANGE <key>\t<value>` | 画面で変えた設定 |

## 測るもの

各方式について。

1. **起動時間**: `cold_start_total` を、再起動の直後の 1 回目（コールド）と、2 回目以降の 10 回（ウォーム）で。
2. **メモリ**: `RESULT MEM` の `first-paint` と `settled`。A は子プロセスの合計と、`private` の内訳（`MEM` の行）。
3. **配布の大きさ**: `target\release\*.exe` の大きさ。A はこれに加えて WebView2 ランタイムが要る（入っているか。ない場合に入れる手順の重さ）。
4. **6 種類の操作**: 各部品を操作して、`CHANGE` の行が正しく出るか。整数は範囲の外（0、10）を入れたときの扱い、列挙の選択、Bundle ID の追加（空欄、重複）と削除。
5. **日本語入力の欄**（`spike.note`、Bundle ID の追加欄）: Microsoft IME で日本語（例: 「にほんご」→ 変換 → 確定）を打つ。見る: (a) 変換中の下線つきの表示が出るか、(b) 候補窓が入力欄の近くに出るか（左上や画面の端に出ないか）、(c) 確定した文字が欄に入るか、(d) 変換中に `CHANGE` が途中の文字で出ないか（A は `compositionend` まで待つ作りにしてある。B は**未確認**）、(e) 半角/全角キーで切り替えられるか、(f) フォントの字形（欠けや豆腐がないか）。
6. **キー割り当ての欄**: `Ctrl+J`、`Shift+←`、`Alt+Space`、`Backspace`（none）、IME がオンの状態での `変換`・`無変換`キー（JIS）が取れるか。**Slint は Ctrl+文字のとき `event.text` が制御文字になる可能性がある（推測）。取れなければ、方式の弱点として記録する**（キー割り当ては ikari の設定画面の中心の機能）。
7. **ダーク・ライト**: Windows の設定でライト → ダークに切り替え、**起動したまま**反映されるか、起動時に従うか、従わないか。
8. **高 DPI**: 100% / 150% / 200%（サインアウトが要る）。字がにじまないか、レイアウトが崩れないか。2 枚のモニタで DPI が違うときに窓を動かして崩れないか。
9. **スクリーンリーダー（ナレーター）**: Tab で各部品に移ったとき、ラベル（「候補をぐるっと回す」など）と種類（スイッチ、スライダ、コンボボックス）と値が読まれるか。スイッチのオン・オフが読まれるか。Bundle ID の一覧の項目と削除ボタン（「〜 を削除」）が読まれるか。
10. **キーボードだけの操作**: マウスなしで Tab / Shift+Tab ですべての部品に行けるか、フォーカスの枠が見えるか、スイッチが Space で切り替わるか、SpinBox と select が矢印で動くか。キー割り当ての欄から Tab で出られるか（キーを奪い続けて出られなくならないか）。
11. **窓の基本**: リサイズで崩れないか、スクロールが効くか、最小化・最大化、タイトルバーの見た目（A も B も標準のタイトルバー）。

## 判断の目安

数値の基準は決めない（実測のあとにユーザーが決める。CLAUDE.md の「値は計測で決める」）。次の順で比べる見込み。

1. **要件を満たすか**: 6 種類が操作できる、日本語の入力欄で IME が使える（5）、キー割り当てが取れる（6）、ナレーターで読める（9）、キーボードだけで操作できる（10）。1 つでも満たさない方式は、直せるかを見て、直せなければ外す。
2. **見た目**（ユーザーが画面を見て決める。スクリーンショットを撮る。ライトとダークの両方）。
3. **起動の速さと配布の軽さ**（`cold_start_total`、`private`、exe の大きさ、ランタイムの要否）。
4. **保守性と法務**: A は HTML と Rust の文字列のやり取り、B は Slint の言語を覚える手間とライセンスの 3 択。どちらも UI の側なので妥協してよい範囲か。
5. 両方が悪い場合は、U-b（WinUI 3）、egui のスパイクを足す。

## 結果の雛形（ADR 0040 §2.5 に追記する）

測った環境: Windows ___（ビルド番号）、VM、CPU/メモリ、日付、WebView2 ランタイムのバージョン、Slint のバージョン。

| 項目 | U-a: WebView2 | U-c1: Slint |
| --- | --- | --- |
| 起動 cold_start_total（コールド / ウォームの中央値） | ___ / ___ ms | ___ / ___ ms |
| メモリ private（first-paint / settled） | ___ / ___ MB | ___ / ___ MB |
| exe の大きさ（ランタイムの要否） | ___ MB（WebView2 要） | ___ MB |
| 6 種類の操作 | ___ | ___ |
| 日本語入力の欄（変換中の表示、候補窓の位置、確定、途中の `CHANGE`） | ___ | ___ |
| キー割り当て（Ctrl+J、変換キー、Tab で出られるか） | ___ | ___ |
| ダーク・ライト（起動中の切り替え） | ___ | ___ |
| 高 DPI（150%、200%、2 枚のモニタ） | ___ | ___ |
| ナレーター | ___ | ___ |
| キーボードだけの操作 | ___ | ___ |
| 見た目（スクリーンショットのパス） | ___ | ___ |
| ビルドの所要時間（初回） | ___ | ___ |
| ライセンス上の問題 | なし（推測） | Slint の 3 択のどれか |
| 所見 | ___ | ___ |

ユーザーの判断: ___（方式の決定は、結果を見てユーザーが行う）。
