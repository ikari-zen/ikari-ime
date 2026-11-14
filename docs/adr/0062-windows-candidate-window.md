# ADR 0062: Windows の候補窓（W3）。別の小さなプロセス `ikari-candidate.exe` に置き、画面は WinUI 3（Windows App SDK、C#）で作る。判断と通信は Rust

- 状態: 提案（2026-11-14。同日、ユーザーの方針「できるだけモダンな技術」を受けて §4 をやり直した）
- 日付: 2026-11-14
- 対象: 新しい `windows/candidate/`（Rust の核: パイプの受け手、命令の状態、配置。C の ABI）と `windows/candidate-ui/`（WinUI 3 の画面。C#）、`windows/tip/src/ops.rs`・`com/edit.rs`（位置、送る、上下の報告）、`windows/engine-host/`（起動と見張り）、`shell-core/src/panel.rs`（y 軸が下向きの座標の入口）、`proto/`（候補窓の命令）、`windows/dev/`（確かめる道具）、`docs/windows-real-device-check.md`、CI の Windows のジョブ
- 前提: main = 0dd0481。ADR 0040 §2.2・§2.4・§2.5・§7（W3）、ADR 0042（判断は Rust。UI だけ別の言語でよい）、ADR 0014 §2.2（上下と予約の行）、ADR 0061 §6.2（`developer_note`）、ADR 0060 Q4（LM を別プロセスにする方向。PR #347、提案）、スパイク `spike/w3-0-candidate-window`（#319。**実機ではまだ動かしていない**）
- 判断の置き場所: Rust。上下と位置の計算は `shell-core` の `panel`、送るかどうかと送る内容は TIP の `ops`、候補窓のプロセスの中の「どれを描くか（最後の命令、持ち主の接続、切れたら隠す）」と横の位置は `windows/candidate` の Rust の核。C# は、核が渡す「描く状態」を XAML で描くだけ（UI であるため。macOS の Swift と `shell-ffi` と同じ形）
- この ADR ではコードを変えない。「分かったこと」はコードと公式の文書で確かめたもの、「推測」は確かめていないもの。推測は §8 のスパイクで確かめてから本決定にする

---

## 0. ユーザーの回答（2026-11-14）
- Q1（描き方）: 推奨（Direct2D の自前描画）ではなく、**「使用する技術はできるだけモダンなものを使ってほしい」**。→ 窓は別プロセスに置くので、TIP の DLL（アプリのプロセスの中）の制約で WebView2 や WinUI 3 を外した理由は弱まる。この前提で §4 をやり直した。
- Q2（アクセシビリティ）: 推奨どおり。TSF の UI 要素は 1.0.0 に入れる。窓そのものの UIA は 1.0.0 の後。
- Q3（目で見る確認）: 推奨どおり。VM のコンソールの画面を開いて見る。
- Q4（16ms のうち窓に使う時間）: 推奨どおり。S-2 の実測のあとに聞く。
## 1. 現状（分かったこと）

### 1.1 Windows
- TIP は `ShellAction::ShowCandidates` / `MoveSelection` / `HideCandidates` を `TsfOp::ShowCandidates(model)` / `MoveCandidates(model)` / `HideCandidates` に変える（`windows/tip/src/ops.rs`）。COM の層は**ログを出すだけ**で、窓はない（`com/edit.rs` の `TsfOp::ShowCandidates(_)` は `show candidates (position found|no position)` を書くだけ）。
- 位置は `candidate_rect()` が `ITfContextView::GetTextExt` で取る。範囲は**合成の全体**（なければ選択）。戻りの `clipped` は使っていない。
- TIP は `Shell::panel_shown(side, source)` を呼んでいない。shell-core の機械は、窓が上（`PanelSide::Above`）のとき ↑/↓ を入れ替える（`shell-core/src/machine/mod.rs` の `panel_side`）。**Windows では今、窓の上下が機械に伝わる道がない。**
- `pipe` の `Client` には片道の `post` がある（返事を待たない）。名前付きパイプの作成、ACL（自分のユーザーだけ）、`PIPE_REJECT_REMOTE_CLIENTS` は `pipe` にある。
- 実機（N150 の VM、2026-11-14）: SSH はセッション 0 で窓を出せない。セッション 1 で流すのは `run-in-session.ps1`。`virsh screenshot` はデスクトップとタスクバーは映したが、Run や Notepad の窓は映さなかった（原因は未確認。`docs/windows-real-device-check.md` §1.6）。画面の代わりに `uia-probe.ps1`、`ime-test.ps1`、`dbgcap.ps1` を使っている。

### 1.2 macOS（揃える相手）
- 窓は自前描画（`macos/Sources/IkariIME/CandidatePanel.swift`。NSPanel に NSView の `draw` で描く。WebView ではない）。行の高さは固定の関数（`CandidateView.height(rows:)`）で、幅は文字を測って決める。
- 配置は Swift が測った大きさで `ShellPanelGeometry.layout`（= `shell-core` の `panel::layout_panel`）を呼び、返った `side` を同期で機械に返す（`ShellDriver.swift` の `PanelShown`）。
- アンカーは変換文字列の先頭の文字の矩形で、閉じるまで動かさない（`PanelUI.anchor`。#86）。取れないときの順は `panel::resolve_anchor`（クライアント → アクセシビリティ → 同じアプリの最後の正常値 → マウスのある画面の中央下）。
- `panel` の座標は AppKit と同じ（左下が原点、y は上向き）。

### 1.3 スパイク（`spike/w3-0-candidate-window`。捨てるコード）
- A（WebView2、tao + wry の別 exe）、B（Win32 + GDI の別 exe）、C（`ITfCandidateListUIElement` のオブジェクトを自分のスレッドに渡すだけ）を、同じ固定の `CandidatePanelModel` 相当で出す。Mac で `cargo check --target x86_64-pc-windows-msvc` まで。**実機で動かしたことはない。** 測定の値は 1 つもない。
- B は GDI だけで、Direct2D / DirectWrite は未実装。DLL の中に置いた場合も測れない（単独の exe）。C は他のアプリが UI 要素を受け取るかを見られない（TIP として入らないと分からない）。

## 2. 問題
1. 候補窓がないので、Windows では 2 位以下の候補を選べない。1.0.0（ADR 0040 §2.3）の「macOS と同じ範囲」を満たさない。
2. 窓の上下が機械に伝わらないので、窓を作っても、上に出たとき ↑/↓ の向きが macOS と違ってしまう。
3. ADR 0040 §2.4 は方式をスパイクの実測で決めるとしたが、実測はまだない。実測を待つと W3 の契約もレーンも組めない。**この ADR は、実測の前に決められること（置き場所、契約、位置決め、レーン）を決め、実測でしか決まらない点を §8 のスパイクの合格の条件にする。**

## 3. 決定の要約（提案）
1. **画面は WinUI 3（Windows App SDK）で作る。言語は C#**（§4 の W-a）。Fluent の見た目、ライトとダーク、高 DPI、アニメーション、UIA（スクリーンリーダー）が標準で付く。配布は**パッケージなし（MSIX にしない）・自己完結**（Windows App SDK と .NET を exe と一緒に置く。利用者に別のランタイムを入れさせない。未署名の配布 #99 と両立）。
2. **判断と通信は Rust の核**（`windows/candidate`。C の ABI の DLL）。パイプの受け手、命令の状態（最後だけを描く、持ち主の接続、切れたら隠す）、横の位置の計算（`shell-core` の `panel`）を持つ。C# はこの DLL を呼び、返った「描く状態」を描くだけ（macOS の Swift と `shell-ffi` と同じ形。境界は 1 本）。
3. **窓の置き場所は、別の小さなプロセス `ikari-candidate.exe`**（ユーザーのセッションに 1 つ、常駐）。`ikari-engine.exe` が子として起動し、見張る（落ちたら起動し直す。回数の上限は engine-host の `CRASH_LIMIT` と同じ考え方）。TIP は exe を起動しない。常駐なので、起動の時間はログオンと落ちたあとにだけ効く。
4. **TIP → 候補窓は片道**（`pipe::Client::post`）。返事を待たない。キーの処理の時間（`key`）に窓の描画は入らない。
5. **上下（`side`）は TIP が決める**。行の高さを「行数と行の種類だけで決まる純粋な関数」にし（macOS と同じ）、TIP がその高さで `panel::layout_panel` を呼んで `side` を得て、すぐ `Shell::panel_shown` に渡す。候補窓は TIP の `side` に従い、横の位置だけを測った幅で決める。
6. **アンカーは合成の先頭の 1 文字**の `GetTextExt`（閉じるまで動かさない。macOS と同じ規則）。TIP がホストの窓の DPI の扱いに合わせて**物理の画面座標**に直してから送る。取れないときの順は `panel::resolve_anchor` をそのまま使う。
7. **y 軸が下向きの座標の入口を `shell-core` の `panel` に足す**（中は既存の `layout_panel` を y を反転して呼ぶだけ）。
8. **WinUI 3 が §8 の条件（フォーカスを奪わない、最前面、常駐のメモリ）を満たさなければ、W-b（Rust だけ: Windows.UI.Composition + Direct2D/DirectWrite + DWM の背景の素材）に切り替える。** W-b は Windows 11 の素材（Acrylic、角丸、影）を OS の同じ口から使えるので、見た目の差は小さい見込み（推測）。差は、UIA と部品を自前で持つこと。
9. 確かめ方: 描いた絵を PNG に書く開発用の口と、窓の状態を読む道具（`windows/dev/candidate-probe.ps1`）で、画面を見ずに確かめる。最後に VM のコンソールで目で見る（Q3 の決定）。

方式の本決定（1〜3、8）は、§8 のスパイクの結果のあとにユーザーが行う（ADR 0040 §2.4 の手順どおり。結果は ADR 0040 §2.4 にも追記する）。

## 4. 描き方（決めること 1。2026-11-14 にやり直した）

### 4.1 比較の軸
- **モダンさ**: Windows 11 の見た目（Mica / Acrylic、角丸、影）、ライトとダーク、高 DPI、アニメーション、アクセシビリティ（UIA）が**標準で**付くか。
- **技術選定**（CLAUDE.md、ADR 0042）: 判断を Rust に置けるか、UI との境界の細さ、ランタイムの配布の負担、未署名の配布（#99）、ARM64、起動の時間とメモリ（常駐）、**フォーカスを奪わない窓**（`WS_EX_NOACTIVATE` 相当）、最前面と全画面の扱い。
- **保守性**: UI だけは妥協してよい（CLAUDE.md）。ただしビルドと CI、Mac での確かめやすさは見る。

### 4.2 分かったこと（公式の文書。2026-11-14 に読んだ）
- WinUI 3 は「新しい Windows のデスクトップのアプリに勧める UI」で、言語は **.NET（C#）と C++**、x86・x64・ARM で動く（Microsoft Learn「WinUI 3」）。Rust は対応の言語に入っていない。`windows` crate の文書は Composition を挙げ、XAML は挙げていない（Microsoft Learn「Rust for Windows」）。
- Windows App SDK は既定で「フレームワークに依存」（ランタイムを別に入れる）。`WindowsAppSDKSelfContained` を true にすると、ランタイムの中身をビルドの出力に入れて、アプリと一緒に配れる。.NET のアプリは .NET も自己完結で発行する必要がある（Microsoft Learn「self-contained の配置」）。
- `AppWindow.Show(bool activateWindow)` があり、窓を前面にせずに出せる。`OverlappedPresenter.IsAlwaysOnTop` で最前面にできる（Windows App SDK の API 文書）。
- XAML Islands（任意の HWND に XAML を置く）は Windows App SDK 1.4 で実験の扱いが外れたが、「今は C++ のアプリでだけ試している」（1.4 のリリースノート）。
- `Windows.UI.Composition.Desktop.DesktopWindowTarget`（Win32 の窓に OS の合成を使う）は Windows 10 1803 から（API 文書）。
- DWM の背景の素材 `DWMWA_SYSTEMBACKDROP_TYPE` は Windows 11 build 22621 から。`DWMSBT_TRANSIENTWINDOW` は「一時的な窓」の素材で、Windows 11 では Desktop Acrylic（API 文書）。
- 角丸: 既定ではポップアップの窓は角丸にならないが、`DWMWA_WINDOW_CORNER_PREFERENCE`（`DWMWCP_ROUNDSMALL` など）で角丸にできる。**VM の中で動くアプリは、設計として角丸にならない**（Microsoft Learn「角丸の適用」）。→ VM では角丸を目で確かめられない。
- winit（Slint の標準の窓の層）の Windows の拡張には、`with_system_backdrop`、`with_corner_preference`、`with_skip_taskbar`、`with_owner_window` などがあるが、**フォーカスを奪わない窓（`WS_EX_NOACTIVATE`）の指定はない**（docs.rs の `WindowAttributesExtWindows`）。

### 4.3 案

| 軸 | W-a: WinUI 3（C#。自己完結。推奨） | W-b: Rust だけ（Windows.UI.Composition + Direct2D/DirectWrite + DWM の素材） | W-c: Slint（Rust の UI。Fluent の見た目のスタイル） | W-d: Direct2D の自前描画だけ（前の案） |
| --- | --- | --- | --- | --- |
| Windows 11 の見た目 | 標準（Fluent、Mica / Acrylic、角丸、影） | OS の同じ素材を DWM と Composition の口から使う。部品は自前 | Fluent に似せたスタイル（OS の素材ではない部分がある。推測）。winit で背景の素材と角丸は指定できる | 自前（角丸と背景の素材は DWM で付けられる） |
| ライトとダーク | 標準（テーマに追従） | 自前（OS の設定を読む） | スタイルが持つ（推測） | 自前 |
| 高 DPI | 標準 | 自前（Per-Monitor V2） | 持つ（推測） | 自前 |
| アニメーション | 標準 | Composition のアニメーション（OS の口） | 持つ | なし（自前） |
| UIA（読み上げ） | 標準（XAML の部品が出す） | 自前（後で） | 一部（推測。確かめる） | 自前（後で） |
| 日本語の文字の描画 | DirectWrite（OS と同じ） | DirectWrite | 自前の描画系（OS と字の見え方が違う可能性。推測） | DirectWrite |
| フォーカスを奪わない | `Show(false)` は文書で確認。`WS_EX_NOACTIVATE` を足して XAML が正しく動くかは**推測**（S-3） | 自前の窓なので確実に付けられる | winit に口がない。HWND を取ってスタイルを足す（推測） | 確実 |
| 最前面 | `IsAlwaysOnTop`（文書で確認） | `WS_EX_TOPMOST` | winit の窓の段（推測） | `WS_EX_TOPMOST` |
| 判断を Rust に | 核を Rust の DLL にし、C# から呼ぶ（境界 1 本。macOS と同じ形） | すべて Rust | すべて Rust | すべて Rust |
| ランタイムと配布 | Windows App SDK と .NET を同梱（大きさは推測で数十〜100MB 超。ADR 0059（#345）の (c) でユーザーは LM について「配る大きさは気にしない」と答えた。候補窓にも当てはめてよいかは Q1 で確かめる） | OS の標準だけ | crate だけ | OS の標準だけ |
| 未署名の配布（#99） | パッケージなし・自己完結なら MSIX の署名は要らない（文書の範囲で）。SmartScreen はどの案でも同じ | 影響なし | 影響なし | 影響なし |
| ARM64 | 対応（文書） | 対応（`windows` crate） | 対応（推測） | 対応 |
| 起動とメモリ（常駐） | 最も重い見込み（推測。S-2 で測る）。常駐なので起動はログオン時だけ | 小さい | 中くらい（推測） | 最も小さい |
| ビルドと CI | .NET SDK と Windows App SDK。**Mac ではビルドできない**（Windows の CI と VM だけ）。新しい言語（C#） | Rust だけ。Mac で `cargo check --target` | Rust だけ | Rust だけ |
| 設定画面（W4-s3）との共有 | 設定画面が U-b（WinUI 3）になれば、道具とランタイムを共有できる | — | 設定画面が U-c（Slint）なら共有できる | — |

### 4.4 推奨: W-a（WinUI 3、C#、パッケージなし・自己完結）
- ユーザーの方針（できるだけモダン）に最も合う。Microsoft が新しいデスクトップのアプリに勧める UI で、Windows 11 の見た目、テーマ、高 DPI、アニメーション、UIA が**標準で**付く。自前で作る部分が最も少ない（UI の保守は妥協してよいが、作る量が減るのは利点）。
- 窓は別プロセスで常駐なので、W-a の弱み（起動の時間、ランタイムの重さ）は、打鍵ごとには効かない。効くのは常駐のメモリだけ（S-2 で測る）。
- 判断は Rust の核に置き、C# は描くだけにする。境界は「C の ABI の DLL を 1 つ」で、macOS（Swift → `shell-ffi`）と同じ形なので、新しい作法を増やさない。
- C# にする理由（C++/WinRT ではなく）: UI は保守性を妥協してよい範囲で、C# の方が文書と例が多く、書く量が少ない（推測）。XAML Islands（C++ でだけ試されている）は使わず、`AppWindow` の窓を使う。
- 弱み（受け入れるもの）: 新しい言語とビルド（.NET SDK）、Mac で確かめられない（CI の Windows のジョブと VM だけ）、配る大きさ、UIA が付く代わりに窓の振る舞い（フォーカス）は XAML の作法に従う必要がある。

### 4.5 W-b・W-c・W-d の扱い
- **W-b は次の案**（W-a が S-3 か S-2 で条件を満たさないとき）。Rust だけで閉じ、OS の同じ素材を使える。UIA は Q2 の決定どおり 1.0.0 の後でよいので、1.0.0 の範囲では W-a との差は小さい。
- **W-c（Slint）を採らない理由**: フォーカスを奪わない窓の口が winit にない（文書）。文字の描画が OS と違う可能性がある（推測）。Fluent に「似せた」スタイルで、OS の部品そのものではない。Rust だけで閉じたいなら W-b の方が OS に近い。ただし設定画面（W4-s3）が Slint に決まったら、もう一度比べる。
- **W-d（前の案）** は W-b の部分集合（Composition と素材を足さない形）。W-b に含める。
- **WebView2（ADR 0040 の C-a）**: 別プロセスなら使えるが、常駐のメモリ（WebView2 のプロセス群）、大きさを測るのが非同期（JS の往復）、見た目が Windows の部品ではなく CSS で似せるもの、の 3 点で W-a に劣る。比べる材料として、今のスパイクの A は流す。

### 4.6 `ITfCandidateListUIElement`（W3-c。窓の後。Q2 で 1.0.0 に入れると決定）
- 窓を出す前に `ITfUIElementMgr::BeginUIElement` で UI 要素を登録し、`show` が `false` のとき（アプリが自分で候補を描く。全画面のゲームなど）は窓に送らない。候補の一覧は UI 要素から読めるようにする。これは TIP（Rust）の仕事で、描き方の案に関係しない。
- 推測: スクリーンリーダー（NVDA、ナレーター）は、この UI 要素から候補を読むことがある。S-4 で確かめる。W-a なら窓そのものの UIA も標準で出るが、フォーカスを持たない窓をスクリーンリーダーが読むかは別（推測）。

## 5. 置き場所（決めること 2）

### 5.1 案

| 軸 | P-a: TIP の DLL の中（アプリのプロセス） | P-b: `ikari-engine.exe` の UI のスレッド | P-c: 別の小さなプロセス `ikari-candidate.exe`（採る） |
| --- | --- | --- | --- |
| 使える描き方 | W-b・W-d だけ（WinUI 3 と .NET をアプリのプロセスに入れるのは現実的でない。推測） | どれでも（W-a ならエンジンに .NET が入る） | どれでも |
| 遅延 | 往復なし | パイプ 1 回（片道） | パイプ 1 回（片道） |
| 窓が落ちたとき | **ユーザーのアプリが落ちる**（保存していない文書ごと）。Rust の panic は `catch_unwind` で止まるが、描画の下のドライバの中の失敗やデッドロックは止まらない | エンジンも落ちる（変換がかなだけになる） | 窓だけが消える。入力と変換は続く |
| 窓の不具合を直したとき | **DLL を読み込んでいるアプリをすべて開き直す**まで古いまま | エンジンの再起動だけ | exe の再起動だけ。見た目を何度も直す UI に向く |
| 高 DPI | ホストのアプリの DPI の扱いに引きずられる（推測） | 1 つの扱い | 1 つの扱い |
| 上下の決定 | 同期で測れる | TIP で決める（§6.2） | TIP で決める（§6.2） |
| 前例 | Microsoft IME（推測） | なし | Mozc（renderer）、azooKey-Windows（ui の exe） |
| 増えるもの | なし | エンジンに Windows の UI が入る | exe 1 つ、パイプ 1 本、見張り |

- **P-c を採る理由**: アプリのプロセスに入るコード（TIP の DLL）を小さく保つ（ADR 0040 §2.2）。窓の失敗がアプリにもエンジンにも波及しない。見た目を直す回し方が速い。片道なので遅延の不利はほぼない（推測。S-2 で測る）。モダンな UI（W-a）を選べるのは、この置き場所だけ。
- **P-b を却下する理由**: 打鍵ごとに描く窓と、変換と LM を持つエンジンが同じプロセスで、片方の失敗がもう片方を止める。W-a ならエンジンに .NET が入る。
- **P-a は、別プロセスの窓が S-3 で条件を満たさないときの最後の案**（そのときは描き方も W-b に替わる）。

### 5.2 ADR 0060 Q4（LM を別プロセス）との関係
- **同じ考え方を使う**: 失敗しやすい部分を主たる処理と別のプロセスに分け、落ちても主たる処理は続く。見張り（起動し直し、回数の上限）は engine-host が持ち、`ikari-lm-host` と `ikari-candidate.exe` で同じ仕組みを使う（共通の部品にするかは、Q4 の本決定のあとに実装で決める。先に共通化しない）。
- **違う点**: LM は要求と返事（エンジンが待つ）。候補窓は片道（TIP は待たない）。P-a の巻き込み先は**ユーザーのアプリ**で、LM（巻き込み先はエンジン）より分ける理由が強い。

### 5.3 通り道
- TIP は `ikari-candidate.exe` のパイプ（`ikari-ime-candidate-v1-s<セッション>`。名前の作り方は `pipe::pipe_name` と同じ）に直接つなぐ。engine-host を中継しない。パイプの受け手は Rust の核（`pipe::Server`）で、C# はパイプに触れない。
- つながらない・切れたときは、窓を出さずに入力を続ける（今と同じ状態）。エンジンの再接続と同じ間隔の上限でつなぎ直す。
- 窓は 1 つ。最後に `Show` を送った接続が持ち主。持ち主の接続が切れたら（アプリが落ちた、TIP が外れた）、核が窓を隠させる。
- パイプの ACL はエンジンのパイプと同じ（自分のユーザーだけ）。ストアのアプリ（app container）からの接続は、エンジンのパイプの決定に従う（1.0.0 の範囲外。ADR 0040 §2.3）。

## 6. 位置決め（決めること 3）

### 6.1 アンカー
- **合成の先頭の 1 文字の範囲**（長さ 0 で答えないアプリには長さ 1 も試す）で `GetTextExt`。今の `candidate_rect()` は合成の全体を使っているので、折り返した合成で矩形が 2 行分になる。直す（L1）。
- 一度決めたアンカーは `HideCandidates` まで動かさない（#86。macOS の `PanelUI.anchor` と同じ）。
- 取れないとき: `panel::resolve_anchor(client, None, last_known, cursor_monitor_work_area)`。2 番目（macOS のアクセシビリティ）に当たるものは、Windows ではまず空にする。`GetGUIThreadInfo` のキャレットを入れるかは、S2（ADR 0040 §6）の結果で決める（`AnchorSource` を増やすので、その時に）。`last_known` は TIP の DLL のインスタンス（= アプリのプロセス）ごとに持つ。
- `clipped` が真（文字が見えている範囲の外）のときの扱いは、S2 の結果で決める。まずは使う（ログに残す）。

### 6.2 上下と大きさ
- **行の高さは、DIP（96 dpi を 1 とした単位）の固定の関数**: 候補の行数（`reserved_rows` と実際の数）、注釈の行の有無、開発者の行の有無だけで決まる。文字を測らない。数値は `windows/candidate` の Rust に置き、TIP と候補窓が同じ関数を使う（macOS の `CandidateView.height(rows:)` と同じ考え方）。XAML の側は、この高さを行の高さとして使う（XAML に任せて高さを決めさせない）。
- TIP は、アンカーのある画面の倍率（DPI / 96）と作業領域（タスクバーを除く）を取り、高さ × 倍率で `layout_panel` を呼んで `side` を決める。すぐ `Shell::panel_shown(side, source)` を呼ぶ（§1.1 の欠けを埋める）。
- 候補窓では、核が「描く状態」（モデル、`side`、アンカー、作業領域、倍率）を C# に渡す。C# は XAML で幅を測り（`Measure`）、核の横の位置の関数（アンカーの左端に揃え、右端からはみ出すなら左へ、作業領域の左端より左へは出さない。`layout_panel` と同じ規則）を呼んで位置を得て、窓を置く。`side == Above` のときは、候補を上下逆に並べ、1 位をアンカーに近い側に置く（ADR 0014 §2.2）。並べる順も核が決めて渡す。
- 測った高さが TIP の高さとずれたら（不具合）、核がログに残し、TIP の `side` に従う（上下が機械とずれる方が害が大きい）。

### 6.3 座標
- Windows の画面座標は左上が原点、y は下向き、単位は物理のピクセル（Per-Monitor V2 のプロセスから見て）。`panel` は y 上向き。**`panel` に y 下向きの入口を足す**（`layout_panel` を、y を反転した矩形で呼び、結果を戻すだけ。新しい規則は作らない。Mac で単体テストする）。
- TIP はホストの窓（`ITfContextView::GetWnd`）の DPI の扱いを読み、DPI に対応しない・システムの DPI だけのアプリなら、`GetTextExt` の矩形を物理の座標に直す（`LogicalToPhysicalPointForPerMonitorDPI` などの OS の API。どれを使うかは S-1 で決める）。**推測**: 対応しないアプリでは `GetTextExt` が論理の座標を返す。
- 倍率と作業領域は、アンカーの中心を含む画面（`MonitorFromRect`）のもの。2 枚の画面をまたぐときもアンカーの画面に従う。WinUI 3 の窓の位置は物理のピクセルで置く（`AppWindow.Move` などは物理の座標。推測。S-1 で確かめる）。

## 7. 契約（決めること 4）

### 7.1 共通と OS 固有の線

| 部分 | 共通（Rust、両 OS） | Windows 固有 |
| --- | --- | --- |
| 内容 | `CandidatePanelModel`（`developer_note` を含む）。shell-core が作る。変えない | なし |
| 上下と横の規則 | `panel::layout_panel`、`resolve_anchor`、`is_usable_anchor`（y 下向きの入口を足す） | なし |
| 行の高さの値 | — | `windows/candidate`（Rust。macOS は Swift が持つ。値は見た目なので OS ごと） |
| アンカー | — | TIP の COM の層（`GetTextExt`、DPI の変換、画面と作業領域） |
| 送るかどうか、何を送るか | — | TIP の `ops`（純粋。Mac でテスト） |
| 候補窓のプロセスの状態（最後の命令、持ち主、切れたら隠す、並べる順） | — | `windows/candidate` の核（Rust。純粋な部分は Mac でテスト） |
| 描く | — | `windows/candidate-ui`（C#、WinUI 3） |

### 7.2 命令（`proto/ikari/candidate/v1/candidate.proto`。新しい）
`ikari.shell.v1` の `CandidatePanelModel`、`Rect`、`PanelSide` を取り込む（同じ形を二重に作らない）。

```proto
message CandidateCommand {
  uint64 seq = 1;                 // 接続ごとに増える。古いものは捨てる
  oneof op {
    Hello hello = 2;              // 版と TIP の pid（ログ用）
    Show show = 3;
    Update update = 4;
    Hide hide = 5;
  }
}
message Show {
  ikari.shell.v1.CandidatePanelModel model = 1;
  ikari.shell.v1.Rect anchor = 2;     // 物理の画面座標、y 下向き
  ikari.shell.v1.Rect work_area = 3;  // 同上。アンカーの画面の作業領域
  ikari.shell.v1.PanelSide side = 4;  // TIP が決めた上下
  double scale = 5;                   // DPI / 96
}
message Update { ikari.shell.v1.CandidatePanelModel model = 1; ikari.shell.v1.PanelSide side = 2; }
message Hide {}
```
- 枠は `pipe` の枠（`post`）。返事はない。核は届いたものの最後だけを C# に渡す（間に合わないものは飛ばす）。
- フィールドの名前と番号は契約コミットで確定する（この ADR は形だけを決める）。

### 7.3 核と C# の境界（C の ABI。1 本）
- 核の DLL（`ikari_candidate.dll`）は、起動（パイプの受け手を始める）、止める、「描く状態が変わった」の知らせ（C# が渡した関数を、C# の UI のスレッドに戻して呼ぶ。戻し方は C# の `DispatcherQueue`）、横の位置の関数、行の高さの関数、を出す。描く状態は proto のバイト列で渡す（`shell-ffi` と同じ作法）。
- C# はこの DLL だけを呼ぶ（`LibraryImport`）。C# の proto の型は、同じ `.proto` から作る。
- W-b に切り替えたときは、同じ核の上に Rust の画面を載せる（核はそのまま使える）。

### 7.4 TsfOp と ops
- `TsfOp::ShowCandidates(model)` / `MoveCandidates(model)` / `HideCandidates` は**変えない**。COM の層がアンカーを取り、`ops` の純粋な関数（新しい候補の部分）に「アンカー、作業領域、倍率、モデル」を渡すと、「送る命令」と「`panel_shown` に渡す `side` と `source`」が返る。COM の層は送ることと `panel_shown` を呼ぶことだけをする。
- `MoveCandidates` のときも、行数が変わって `side` が変わる場合に備えて計算し直す（予約の行があるので、ふつうは変わらない。ADR 0014 §2.2）。
- 文書のフォーカスが外れたとき、TIP が外れるとき（`Deactivate`）は `Hide` を送る（shell-core が `HideCandidates` を出さない経路の取りこぼしを防ぐ）。
- `developer_note`（ADR 0061、#363）は、モデルに入って運ばれ、C# が 1 行を描くだけ。Windows で別に作るものはない。

### 7.5 依存の向き
`windows/tip → windows/candidate`（rlib の純粋な部分: 命令の型、行の高さ）、`windows/tip → shell-core`、`windows/candidate → shell-core`（型と `panel`）、`windows/candidate → pipe`、`windows/candidate-ui（C#）→ ikari_candidate.dll`、`windows/engine-host`（exe を起動するだけ）。循環はない。

## 8. 確かめるスパイク（推測を確かめる。捨てるコード、ログつき）
ブランチ `spike/w3-0b-candidate-modern`（main に入れない）。今のスパイク（`spike/w3-0-candidate-window`）の A（WebView2）と B（GDI）は、比べる基準としてそのまま流す。足すのは次の 2 つの exe と、TIP の開発用のログ。

- **exe-a**: WinUI 3（C#、パッケージなし・自己完結）。`AppWindow` の窓に、固定の `CandidatePanelModel` を XAML で出す。
- **exe-b**: Rust（`windows` crate）。Win32 の窓に Windows.UI.Composition と Direct2D/DirectWrite で描き、`DWMSBT_TRANSIENTWINDOW` と `DWMWCP_ROUNDSMALL` を付ける。
- どちらも、TIP からの `post` を受ける最小のパイプの受け手を持つ（S-2 の遅延を測るため）。

| 番号 | 確かめる推測 | 対象 | ログに出すもの | 合格の目安 |
| --- | --- | --- | --- | --- |
| S-1 | DPI に対応しないアプリで、`GetTextExt` を物理の座標に直せる。窓を物理の座標で置ける | TIP（共通）、exe-a、exe-b | `anchor raw=… awareness=… physical=… monitor=… dpi=…`、窓の実際の矩形（`GetWindowRect`） | Notepad と、DPI に対応しない小さな試験の exe で、100% と 150%、2 枚の画面で、窓の左上がアンカーの左下とほぼ一致（差を記録する。許す差はユーザーが実測を見て決める） |
| S-3 | フォーカスを奪わず最前面に出る | exe-a（`Show(false)`、`IsAlwaysOnTop`、`WS_EX_NOACTIVATE` を足した場合と足さない場合）、exe-b | 表示の前後の前面の窓、窓の拡張スタイル、Z 順で自分より上の窓 | Notepad、Edge、Chrome、VS Code で、前面が変わらない、打鍵が続く、Alt+Tab とタスクバーに出ない、最前面のアプリの上にも出る。**exe-a がここで落ちたら W-b にする** |
| S-2 | 片道の送信と描画が速い。常駐のメモリ | exe-a、exe-b | `post_us`（TIP）、`recv→present_us`（exe。QPC はプロセスをまたいで比べられる）、選択を動かしたときの更新の中央値と p99、プロセスの private のメモリ（表示の前後と 10 分後）、起動の時間（冷えたときと 2 回目） | 数値の目安は決めない。実測をユーザーに出して、Q4（窓に使える時間）と、常駐のメモリを許すかを聞く |
| S-4 | UI 要素とスクリーンリーダー | TIP（共通。`ITfCandidateListUIElement` の最小）、exe-a の UIA | アプリごとの `show`、ナレーターの読み上げ（耳で確かめる。ユーザー） | アプリごとの `show` の一覧が取れる。ナレーターが UI 要素から読むか、exe-a の UIA から読むかが分かる |
| S-5 | Windows 11 の素材（Acrylic）、角丸、影、ダーク | exe-a、exe-b | DWM の呼び出しの戻り値、`DwmGetWindowAttribute` で読み直した値、テーマの値 | **VM の中では角丸にならない**（文書）。VM では戻り値と読み直しの値で確かめ、目で見るのは素材とダークだけ。角丸を目で見るには実機が要る（§12） |
| S-6 | 画面なしで窓を確かめられる | exe-a（XAML の `RenderTargetBitmap` で PNG）、exe-b（Direct2D の絵を PNG）、セッション 1 の `CopyFromScreen` | PNG の場所、`CopyFromScreen` に窓が映るか | PNG が描いたとおり。`CopyFromScreen` が窓を映すかを記録する |

- 順は S-1 → S-3 → S-2 → S-5 → S-6 → S-4（S-1 と S-3 が置き場所と描き方の成否を決める）。
- 結果は ADR 0040 §2.4 の表と、この ADR に追記する。

## 9. 実装の進め方（決めること 5）

### 9.1 レーンと順序
0. **スパイク**（§8）。ユーザーの方式の決定（§10 Q1）のあとに 1 へ。
1. **契約コミット**（リード）: `proto/ikari/candidate/v1/candidate.proto`、`windows/candidate` の骨組み（命令の型、行の高さと横の位置の関数のシグネチャ、C の ABI のヘッダ）、`panel` の y 下向きの入口のシグネチャ、`ops` の候補の部分のシグネチャとスタブ、`windows/candidate-ui` の空のプロジェクトと CI のビルドだけのジョブ。
2. **並行**（`isolation: worktree`。持つファイルの境界を決める）:
   - **L1 TIP**（`windows/tip/`）: アンカー（先頭の 1 文字、閉じるまで固定、DPI の変換、作業領域）、`ops` の候補の部分、送信（`post`、つなぎ直し）、`panel_shown`、フォーカスが外れたときの `Hide`。`ops` は Mac でテスト。
   - **L2 核**（`windows/candidate/`、`shell-core/src/panel.rs` の y 下向きの入口）: パイプの受け手、命令の状態（最後だけ、持ち主、切れたら隠す、並べる順）、行の高さ、横の位置、C の ABI。純粋な部分は Mac でテスト。
   - **L3 画面**（`windows/candidate-ui/`。C#）: `AppWindow` の窓（前面にしない、最前面、タスクバーと Alt+Tab に出さない）、XAML の候補の並び、ライトとダーク、Acrylic、上下逆の並び、開発者の行、PNG の口。Mac ではビルドできないので、確かめは CI の Windows のジョブと VM。
   - **L4 起動と道具**（`windows/engine-host/`、`windows/dev/`、手順書、配布の一覧）: 子の起動と見張り、`build.ps1` と `start-engine.ps1` への追加（.NET SDK の確認を含む）、`candidate-probe.ps1`、手順書の新しい項目。
3. **統合**（リード）: `scripts/check.sh`、CI の Windows のジョブ（C# のビルドを含む）、実機の手順書の新しい項目（§9.2）。
4. **W3-c**（UI 要素。§4.6）: 窓の後の別の issue（1.0.0 に入れる。Q2 の決定）。
5. 1.0.0 の後: 窓の UIA を確かめて整える（W-a なら標準で出る分を確かめるだけ）、ストアのアプリ。

### 9.2 実機で確かめる方法（画面が映らない前提）
- `candidate-probe.ps1`（`run-in-session.ps1` でセッション 1 で流す）: 窓を探し、見えているか、矩形、拡張スタイル、前面の窓がホストのままか、画面と DPI を書く。キーは今までどおり `virsh send-key`。
- PNG の口: 開発者向けの設定（`developer.*`）か、開発用の環境変数があるときだけ、描いた絵を開発用の場所に書く。**候補の文字が絵に入るので、既定ではオフ。ログには文字を出さない**（今のログの約束と同じ）。
- 合わせて読むもの: TIP の `dbgcap.ps1`（`show candidates` に、アンカー、`side`、送った `seq`）、核のログ（受けた `seq`、描いた矩形、かかった時間）。`seq` で突き合わせる。
- 目で見る確認（ユーザー、5 分程度）: VM のコンソールの画面で（Q3 の決定）。見た目（字、色、素材、ダーク）、ちらつき。角丸は VM では出ない（§4.2）。

## 10. ユーザーの判断が要る点（推奨つき）

### 決定済み（2026-11-14）
- Q2: TSF の UI 要素は 1.0.0 に入れる。窓そのものの UIA は 1.0.0 の後。
- Q3: VM のコンソールの画面で目で見る。
- Q4: 窓に使える時間は、S-2 の実測のあとに聞く。
- Q1 の方針: できるだけモダンな技術を使う（§0）。
- **Q5: 角丸は、API の戻り値と PNG でだけ確かめる（ユーザー、2026-11-14）。** VM では角丸にならず、手元に Windows の実機がない。作者以外の利用者から報告があれば直す。
- **Q1': W-a（WinUI 3、C#、パッケージなし・自己完結）にする（ユーザー、2026-11-14）。** 理由の補足: 候補窓は描画に関わる Windows 独自の部分で、Rust である必要がない（判断は Rust の核、描画は OS ごとの UI。macOS の Swift と同じ考え方）。本決定は S-1・S-3・S-2 の結果のあと（S-3 が通らなければ W-b）。

### Q1'. 描き方を W-a（WinUI 3、C#、パッケージなし・自己完結）にするか
- 選択肢: W-a（WinUI 3）、W-b（Rust だけ: Composition + Direct2D/DirectWrite + DWM の素材）、W-c（Slint）。
- 推奨: **W-a。ただし本決定は S-1・S-3・S-2 の結果を見てから**。S-3（フォーカスを奪わない、最前面）で W-a が通らなければ W-b にする。
- W-a で受け入れることになる点: リポジトリに C# と .NET SDK が入る（UI だけ。判断は Rust の核）、Mac ではビルドできない（CI の Windows のジョブと VM で確かめる）、配る大きさが増える（推測で数十〜100MB 超。LM と同じく「気にしない」でよいか）、常駐のメモリが W-b より大きい（S-2 で測る）。

### Q5. 角丸を目で確かめる手段
- VM の中のアプリは角丸にならない（文書）。手元に Windows の実機がない（#345 の (e) の回答）ので、角丸は API の戻り値と PNG でだけ確かめる。
- 推奨: **それでよい**（角丸は OS が描くもので、付ける呼び出しが成功していれば十分。作者以外の利用者からの報告で直す）。

## 11. 却下した案（まとめ）
- Slint（W-c）: §4.5。
- WebView2: §4.5。
- 前の案の Direct2D の自前描画だけ（W-d）: W-b に含める（§4.5）。
- engine-host に窓を置く（P-b）: §5.1。
- TIP が exe を起動する: ストアのアプリや管理者のアプリから起動すると、権限と寿命がアプリに引きずられる。起動は常駐の engine-host だけ。
- 候補窓が上下を決めて返事で返す: 返事を待つと打鍵の道に往復が乗り、待たないと次のキー（↑/↓）に間に合わないことがある。行の高さを固定の関数にして TIP が決める（§6.2）。
- engine-host を中継する: 往復が増え、エンジンが落ちると窓も止まる（§5.3）。
- C# にパイプと状態を持たせる: 判断が C# に入り、W-b に切り替えたときに作り直しになる。核を Rust に置く（§7.3）。
- XAML Islands（C++）: Windows App SDK 1.4 の時点で C++ でだけ試されている。`AppWindow` の窓で足りる見込み（推測）。

## 12. 懸念と前提
- **すべての「推測」は実機で 1 回も確かめていない。** とくに S-1（DPI の変換と窓の座標）と S-3（WinUI 3 の窓がフォーカスを奪わず最前面に出る）が崩れると、描き方（§4）か置き場所（§5）が変わる。
- W-a は Mac で確かめられない部分が増える（C# の画面）。今の Windows の作業も CI と VM が頼りなので、作法は変わらないが、ビルドの道具（.NET SDK、Windows App SDK）を CI と VM に足す。
- 1 つのプロセスに 1 つの窓なので、2 つのアプリで同時に候補を出すことはない。持ち主の切り替えは「最後に Show を送った接続」で足りる見込み（推測）。
- `virsh screenshot` が窓を映さない原因は分かっていない（GPU を渡したあとの 2 つの画面のどちらかを映している可能性。推測）。確かめ方は PNG と道具で組み、画面に頼らない。
- exe と DLL と同梱のランタイムが増えるので、配布（W5）とセキュリティレビュー（W6）の対象が増える（パイプ 1 本、子の起動、同梱のランタイムの更新）。
- 行の高さを固定にするので、OS の「テキストのサイズ」（文字の拡大）に追従するかは、L2 と L3 で決める（追従するなら、倍率と同じく TIP と候補窓に同じ値を渡す）。
- 設定画面（W4-s3）の方式が決まったら、道具の共有（WinUI 3 なら同じ .NET と Windows App SDK）を見直す。候補窓の決定は設定画面に縛られない。
- ADR 0060 Q4 の本決定（LM の別プロセス）が変わっても、この ADR の決定は変わらない（見張りの共通化だけが変わる）。

## 参考
- ADR 0040 §2.2・§2.4・§2.5・§6（S2、S10）・§7（W3）、ADR 0042、ADR 0014 §2.2、ADR 0061 §6.2、ADR 0060（PR #347）、ADR 0059（PR #345）
- issue #319（W3-0）、#363（Windows の候補窓に開発者の行）、#86（アンカーを動かさない）、#99（未署名の配布）
- [`docs/windows-real-device-check.md`](../windows-real-device-check.md) §1.6、項目 33
- スパイク `spike/w3-0-candidate-window` の `spikes/candidate-window/README.md`
- Microsoft Learn（2026-11-14 に読んだ）: WinUI 3 の概要、Windows App SDK の self-contained の配置、`AppWindow.Show(Boolean)`、`OverlappedPresenter.IsAlwaysOnTop`、Windows App SDK 1.4 のリリースノート（XAML Islands）、`DesktopWindowTarget`、`DWM_SYSTEMBACKDROP_TYPE`、角丸の適用、Rust for Windows。docs.rs: winit の `WindowAttributesExtWindows`
