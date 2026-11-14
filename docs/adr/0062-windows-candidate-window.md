# ADR 0062: Windows の候補窓（W3）。Direct2D の自前描画を、別の小さなプロセス `ikari-candidate.exe` に置く

- 状態: 提案（2026-11-14）
- 日付: 2026-11-14
- 対象: 新しい `windows/candidate/`（lib の純粋な部分と、Windows の exe）、`windows/tip/src/ops.rs`・`com/edit.rs`（位置、送る、上下の報告）、`windows/engine-host/`（起動と見張り）、`shell-core/src/panel.rs`（y 軸が下向きの座標の入口）、`proto/`（候補窓の命令）、`windows/dev/`（確かめる道具）、`docs/windows-real-device-check.md`
- 前提: main = 0dd0481。ADR 0040 §2.2・§2.4・§7（W3）、ADR 0042（判断は Rust）、ADR 0014 §2.2（上下と予約の行）、ADR 0061 §6.2（`developer_note`）、ADR 0060 Q4（LM を別プロセスにする方向。PR #347、提案）、スパイク `spike/w3-0-candidate-window`（#319。**実機ではまだ動かしていない**）
- 判断の置き場所: Rust。上下と位置の計算は `shell-core` の `panel`、候補窓に送るかどうかと送る内容は TIP の `ops`（純粋な部分）。exe は描くだけ（Direct2D の窓。UI であるため）。TIP の COM の層は、位置を取って送るだけ
- この ADR ではコードを変えない。「分かったこと」はコードと文書で確かめたもの、「推測」は確かめていないもの。推測は §8 のスパイクで確かめてから本決定にする

---

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
1. **描き方は Direct2D + DirectWrite の自前描画**（ADR 0040 の C-b）。WebView2（C-a）は採らない。`ITfCandidateListUIElement`（C-c）は**組み合わせる口**として、窓の後に足す（W3-c）。
2. **窓の置き場所は、別の小さなプロセス `ikari-candidate.exe`**（Per-Monitor V2 の DPI 対応。ユーザーのセッションに 1 つ）。`ikari-engine.exe` が子として起動し、見張る（落ちたら起動し直す。回数の上限は engine-host の `CRASH_LIMIT` と同じ考え方）。TIP は exe を起動しない。
3. **TIP → 候補窓は片道**（`pipe::Client::post`）。返事を待たない。キーの処理の時間（`key`）に窓の描画は入らない。
4. **上下（`side`）は TIP が決める**。行の高さを「行数と行の種類だけで決まる純粋な関数」にし（macOS と同じ）、TIP がその高さで `panel::layout_panel` を呼んで `side` を得て、すぐ `Shell::panel_shown` に渡す。exe は TIP が決めた `side` に従い、横の位置だけを自分が測った幅で決める。
5. **アンカーは合成の先頭の 1 文字**の `GetTextExt`（閉じるまで動かさない。macOS と同じ規則）。TIP がホストの窓の DPI の扱いに合わせて**物理の画面座標**に直してから送る。取れないときの順は `panel::resolve_anchor` をそのまま使う（クライアント → 同じプロセスの最後の正常値 → マウスのある画面）。
6. **y 軸が下向きの座標の入口を `shell-core` の `panel` に足す**（中は既存の `layout_panel` を y を反転して呼ぶだけ。規則は 1 つ）。
7. 確かめ方: exe が**描いた絵を PNG に書く開発用の口**と、**窓の状態を読む道具**（`windows/dev/candidate-probe.ps1`）で、画面を見ずに確かめる。最後に目で見るのはユーザーの 5 分程度（§10 Q3）。

方式の本決定（1 と 2）は、§8 のスパイクの合格の条件を満たしたあとにユーザーが行う（ADR 0040 §2.4 の手順どおり。結果は ADR 0040 §2.4 にも追記する）。

## 4. 描き方（決めること 1）

### 4.1 比較の軸
(a) NFR-02（Space から 1 ページ目まで p99 16ms）に窓を足して遅れないか、(b) 見た目（macOS と同じ構成、きれいさ優先）、(c) 置けるプロセス（DLL の中でも別プロセスでも使えるか）、(d) 依存と配布、(e) 大きさを同期で測れるか（上下の決定が同期で要る。§1.1、§1.2）、(f) 高 DPI、(g) アクセシビリティ。

### 4.2 案

| 軸 | D-a: Direct2D + DirectWrite（採る） | D-b: WebView2（tao + wry） | D-c: WinUI 3 / XAML |
| --- | --- | --- | --- |
| (a) 遅延 | 小さい見込み（推測。§8 で測る） | WebView の起動と JS の往復が乗る（推測） | XAML の起動が乗る（推測） |
| (b) 見た目 | 自前で作る。macOS も自前描画なので、同じ構成をそのまま写せる。文字は DirectWrite（色つきの絵文字も描ける） | CSS で作りやすい | 標準の部品できれい |
| (c) 置き場所 | DLL の中でも別 exe でも | 別 exe だけ（アプリのプロセスに WebView を入れられない） | 別 exe だけ |
| (d) 依存 | OS の標準だけ | WebView2 ランタイム（Windows 11 は通常あり。推測）。無いときの扱いが要る | Windows App SDK のランタイム。C# か C++ のビルドが増える |
| (e) 大きさの同期 | 測れる（DirectWrite の `IDWriteTextLayout`） | JS で測って返すので非同期。最初の 1 枚が仮の大きさになる | 非同期の配置 |
| (f) 高 DPI | Per-Monitor V2 で自分で倍率を掛ける | WebView が持つ | XAML が持つ |
| (g) スクリーンリーダー | 自分で UIA を出す（後で）。TSF の UI 要素は別に足す | WebView の UIA が出る | XAML の UIA が出る |

- **D-a を採る理由**: 窓の中身は、縦 1 列の候補、番号、右の薄いラベル、注釈の行、開発者の 1 行（`developer_note`）で、macOS では 239 行の自前描画で足りている。部品を持つ枠組みの利点が小さい。依存が増えず、ランタイムが無いときの扱いが要らない。上下を決める高さを固定の関数にでき（§6.2）、TIP と exe で同じ値になる。
- **D-b を却下する理由**: 常駐のメモリ（WebView2 のプロセス群）、ランタイムの有無、大きさが非同期になることが、見た目の作りやすさより重い。設定画面（W4-s3）が WebView2（U-a）に決まったときも、候補窓は D-a のままにする（設定画面は開くときだけ動き、候補窓は打鍵ごとに動く。求めるものが違う）。
- **D-c を却下する理由**: 言語とランタイムが増える（ADR 0042「OS に依存しないものは Rust」、境界を細く）。
- GDI（スパイクの B）は比べる基準としてだけ使う。製品は Direct2D にする（拡大縮小、アンチエイリアス、色つきの絵文字、角丸）。

### 4.3 `ITfCandidateListUIElement`（W3-c。窓の後）
- 窓を出す前に `ITfUIElementMgr::BeginUIElement` で UI 要素を登録し、`show` が `false` のとき（アプリが自分で候補を描く。全画面のゲームなど）は窓に送らない。候補の一覧は UI 要素から読めるようにする。
- 推測: スクリーンリーダー（NVDA、ナレーター）は、この UI 要素から候補を読むことがある。§8 の S-4 で確かめる。
- 窓そのものの UIA（`WM_GETOBJECT`）は 1.0.0 の後にする案（§10 Q2）。

## 5. 置き場所（決めること 2）

### 5.1 案

| 軸 | P-a: TIP の DLL の中（アプリのプロセス） | P-b: `ikari-engine.exe` の UI のスレッド | P-c: 別の小さなプロセス `ikari-candidate.exe`（採る） |
| --- | --- | --- | --- |
| 遅延 | 往復なし | パイプ 1 回（片道）。エンジンの要求とは別の流れが要る | パイプ 1 回（片道） |
| 窓が落ちたとき | **ユーザーのアプリが落ちる**（保存していない文書ごと）。Rust の panic は `catch_unwind` で止まるが、ドライバ（Direct2D の下の GPU）の中の失敗やデッドロックは止まらない | エンジンも落ちる（変換がかなだけになる） | 窓だけが消える。入力と変換は続く |
| 窓の不具合を直したとき | **DLL を読み込んでいるアプリをすべて開き直す**まで古いまま | エンジンの再起動だけ | exe の再起動だけ。見た目を何度も直す UI に向く |
| 高 DPI | ホストのアプリの DPI の扱いに引きずられる（DPI に対応しないアプリでは、スレッドの DPI の文脈を切り替えて窓を作る必要がある。推測） | 1 つの扱い（Per-Monitor V2） | 1 つの扱い（Per-Monitor V2） |
| アプリごとの重さ | Direct2D の工場と字形のキャッシュがアプリごとに載る | なし | なし |
| 上下の決定 | 同期で測れる | TIP で決める（§6.2） | TIP で決める（§6.2） |
| 前例 | Microsoft IME（推測） | なし | Mozc（renderer）、azooKey-Windows（ui の exe） |
| 増えるもの | なし | エンジンに Windows の UI が入る（エンジンの OS に依存しない作りが崩れる） | exe 1 つ、パイプ 1 本、見張り |

- **P-c を採る理由**: アプリのプロセスに入るコード（TIP の DLL）を小さく保つ（ADR 0040 §2.2）。窓の失敗がアプリにもエンジンにも波及しない。見た目を直す回し方が速い（exe の再起動だけ）。片道なので遅延の不利はほぼない（推測。§8 S-2 で測る）。
- **P-b を却下する理由**: 打鍵ごとに描く窓と、変換と LM を持つエンジンが同じプロセスで、片方の失敗がもう片方を止める。engine-host は「`Engine::handle` に渡す」薄い殻で、UI を入れると責任が 2 つになる。
- **P-a は、P-c がスパイクで条件を満たさないときの次の案**として残す（§8 の S-3 で、別プロセスの窓が最前面に出ない、フォーカスを奪うなどの問題が出たとき）。

### 5.2 ADR 0060 Q4（LM を別プロセス）との関係
- **同じ考え方を使う**: 失敗しやすい部分（GPU のドライバに触る部分）を、主たる処理と別のプロセスに分け、落ちても主たる処理は続く。見張り（起動し直し、回数の上限）は engine-host が持ち、`ikari-lm-host` と `ikari-candidate.exe` で同じ仕組みを使う（共通の部品にするかは、Q4 の本決定のあとに実装で決める。先に共通化しない）。
- **違う点**: LM は要求と返事（エンジンが結果を待つ）。候補窓は片道（TIP は待たない）。候補窓の失敗の巻き込み先は、P-a なら**ユーザーのアプリ**で、LM（巻き込み先はエンジン）より分ける理由が強い。

### 5.3 通り道
- TIP は `ikari-candidate.exe` のパイプ（`ikari-ime-candidate-v1-s<セッション>`。名前の作り方は `pipe::pipe_name` と同じ）に、直接つなぐ。engine-host を中継しない（1 回の余分な往復と、エンジンへの依存を避ける）。
- つながらない・切れたときは、窓を出さずに入力を続ける（今と同じ状態）。エンジンの再接続と同じ間隔の上限でつなぎ直す。
- 窓は 1 つ。最後に `Show` を送った接続が持ち主。持ち主の接続が切れたら（アプリが落ちた、TIP が外れた）、exe は窓を隠す。
- パイプの ACL はエンジンのパイプと同じ（自分のユーザーだけ）。ストアのアプリ（app container）からつなぐときの ACL は、エンジンのパイプの決定に従う（1.0.0 の範囲外。ADR 0040 §2.3）。

## 6. 位置決め（決めること 3）

### 6.1 アンカー
- **合成の先頭の 1 文字の範囲**（長さ 0 で答えないアプリには長さ 1 も試す）で `GetTextExt`。今の `candidate_rect()` は合成の全体を使っているので、折り返した合成で矩形が 2 行分になる。直す（L1）。
- 一度決めたアンカーは `HideCandidates` まで動かさない（#86。macOS の `PanelUI.anchor` と同じ）。
- 取れないとき: `panel::resolve_anchor(client, None, last_known, cursor_monitor_work_area)`。2 番目（macOS のアクセシビリティ）に当たるものは、Windows ではまず空にする。`GetGUIThreadInfo` のキャレットを入れるかは、S2（ADR 0040 §6）の結果で決める（`AnchorSource` を増やすので、その時に）。`last_known` は TIP の DLL のインスタンス（= アプリのプロセス）ごとに持つ。
- `clipped` が真（文字が見えている範囲の外）のときの扱いは、S2 の結果で決める。まずは使う（ログに残す）。

### 6.2 上下と大きさ
- **行の高さは、DIP（96 dpi を 1 とした単位）の固定の関数**: 候補の行数（`reserved_rows` と実際の数）、注釈の行の有無、開発者の行の有無だけで決まる。文字を測らない。数値は `windows/candidate` の lib に置き、TIP と exe が同じ関数を使う（macOS の `CandidateView.height(rows:)` と同じ考え方）。
- TIP は、アンカーのある画面の倍率（DPI / 96）と作業領域（タスクバーを除く）を取り、高さ × 倍率で `layout_panel` を呼んで `side` を決める。すぐ `Shell::panel_shown(side, source)` を呼ぶ（§1.1 の欠けを埋める）。
- exe は、DirectWrite で幅を測り、TIP から来た `side` と、横の規則（アンカーの左端に揃え、右端からはみ出すなら左へ、作業領域の左端より左へは出さない。`layout_panel` と同じ）で位置を決める。`side == Above` のときは、候補を上下逆に描き、1 位をアンカーに近い側に置く（ADR 0014 §2.2）。
- exe が測った高さと TIP の高さがずれたら（不具合）、exe はログに残し、TIP の `side` に従う（上下が機械とずれる方が害が大きい）。

### 6.3 座標
- Windows の画面座標は左上が原点、y は下向き、単位は物理のピクセル（Per-Monitor V2 のプロセスから見て）。`panel` は y 上向き。**`panel` に y 下向きの入口を足す**（`layout_panel` を、y を反転した矩形で呼び、結果を戻すだけ。新しい規則は作らない。Mac で単体テストする）。
- TIP はホストの窓（`ITfContextView::GetWnd`）の DPI の扱いを読み、DPI に対応しない・システムの DPI だけのアプリなら、`GetTextExt` の矩形を物理の座標に直す（`LogicalToPhysicalPointForPerMonitorDPI` などの OS の API。どれを使うかは S-1 で決める）。**推測**: 対応しないアプリでは `GetTextExt` が論理の座標を返す。
- 倍率と作業領域は、アンカーの中心を含む画面（`MonitorFromRect`）のもの。2 枚の画面をまたぐときもアンカーの画面に従う。

## 7. 契約（決めること 4）

### 7.1 共通と OS 固有の線

| 部分 | 共通（Rust、両 OS） | Windows 固有 |
| --- | --- | --- |
| 内容 | `CandidatePanelModel`（`developer_note` を含む）。shell-core が作る。変えない | なし |
| 上下と横の規則 | `panel::layout_panel`、`resolve_anchor`、`is_usable_anchor`（y 下向きの入口を足す） | なし |
| 行の高さの値 | — | `windows/candidate` の lib（macOS は Swift が持つ。値は見た目なので OS ごと） |
| アンカー | — | TIP の COM の層（`GetTextExt`、DPI の変換、画面と作業領域） |
| 送るかどうか、何を送るか | — | TIP の `ops`（純粋。Mac でテスト） |
| 描く | — | `ikari-candidate.exe`（Direct2D） |

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
- 枠は `pipe` の枠（`post`）。返事はない。exe は届いたものの最後だけを描く（間に合わないものは飛ばす）。
- フィールドの名前と番号は契約コミットで確定する（この ADR は形だけを決める）。

### 7.3 TsfOp と ops
- `TsfOp::ShowCandidates(model)` / `MoveCandidates(model)` / `HideCandidates` は**変えない**。COM の層がアンカーを取り、`ops` の純粋な関数（新しい候補の部分）に「アンカー、作業領域、倍率、モデル」を渡すと、「送る命令」と「`panel_shown` に渡す `side` と `source`」が返る。COM の層は送ることと `panel_shown` を呼ぶことだけをする。
- `MoveCandidates` のときも、行数が変わって `side` が変わる場合に備えて計算し直す（予約の行があるので、ふつうは変わらない。ADR 0014 §2.2）。
- 文書のフォーカスが外れたとき、TIP が外れるとき（`Deactivate`）は `Hide` を送る（shell-core が `HideCandidates` を出さない経路の取りこぼしを防ぐ）。
- `developer_note`（ADR 0061、#363）は、モデルに入って運ばれ、exe が 1 行を描くだけ。Windows で別に作るものはない。

### 7.4 依存の向き
`windows/tip → windows/candidate`（lib の純粋な部分: 命令の型、行の高さ）、`windows/tip → shell-core`、`windows/candidate（exe）→ shell-core`（型と `panel`）、`windows/candidate → pipe`、`windows/engine-host`（exe を起動するだけ。lib に依存しない）。循環はない。

## 8. 確かめるスパイク（推測を確かめる。捨てるコード、ログつき）
ブランチ `spike/w3-0b-candidate-out-of-proc`（main に入れない）。今のスパイク（`spike/w3-0-candidate-window`）の A と B は、S10 の実測としてそのまま流す（D-a と D-b の比較の材料）。足すものは次。

| 番号 | 確かめる推測 | 作るもの | ログに出すもの | 合格の目安 |
| --- | --- | --- | --- | --- |
| S-1 | DPI に対応しないアプリで、`GetTextExt` を物理の座標に直せる | 今の TIP に、アンカーの矩形、ホストの窓の DPI の扱い、直したあとの矩形、画面、DPI を書く開発用のログ（文字は書かない） | `anchor raw=… awareness=… physical=… monitor=… dpi=…` | Notepad（Per-Monitor）と、DPI に対応しない小さな試験の exe（スパイクに入れる）で、100% と 150%、2 枚の画面で、直した矩形がキャレットの位置とほぼ一致する（`GetGUIThreadInfo` を物理に直したものと比べ、差を記録する） |
| S-2 | 片道の送信と Direct2D の描画が速い | B を Direct2D + DirectWrite に替えた exe と、TIP からの `post` | `post_us`（TIP の側）、`recv→present_us`（exe の側。QPC はプロセスをまたいで比べられる） | キーから描き終わりまでの増え分の p99 が、コアの時間と足して NFR-02 に入る（窓に使える割合は §10 Q4） |
| S-3 | 別プロセスの窓が、フォーカスを奪わず最前面に出る | S-2 の exe に `WS_EX_NOACTIVATE`、`WS_EX_TOOLWINDOW`、`WS_EX_TOPMOST` | 表示の前後の前面の窓、窓の拡張スタイル、Z 順で自分より上の窓 | Notepad、Edge、Chrome、VS Code で、前面が変わらない、打鍵が続く、Alt+Tab とタスクバーに出ない。最前面のアプリ（タスクマネージャーの「常に手前」）の上にも出る |
| S-4 | UI 要素を受け取るアプリと、スクリーンリーダーが候補を読むか | TIP に `ITfCandidateListUIElement` の最小（`show` の値をログに） | アプリごとの `show`、ナレーターの読み上げ（耳で確かめる。ユーザー） | アプリごとの `show` の一覧が取れる。ナレーターが候補を読むかが分かる |
| S-5 | Windows 11 の角丸と影が、この種の窓に付く | S-2 の exe に `DWMWA_WINDOW_CORNER_PREFERENCE` と影 | 呼び出しの戻り値 | PNG（下）と目視で、角丸と影がある。付かないなら自前（レイヤードの窓）にする |
| S-6 | 画面なしで窓を確かめられる | exe に「描いた絵を PNG に書く」口、セッション 1 での `CopyFromScreen` の試し | PNG の場所、`CopyFromScreen` に窓が映るか | PNG が描いたとおり。`CopyFromScreen` が窓を映すかを記録する（映らなくても PNG で足りる） |

- 順は S-1 → S-3 → S-2 → S-5 → S-6 → S-4（S-1 と S-3 が P-c の成否を決める）。
- S-3 が通らない（別プロセスの窓が出ない、フォーカスを奪う）なら、P-a（DLL の中）に戻して S-2 を DLL の中で測る。
- 合格の目安の数値（S-1 の許す差、S-2 の割り当て）は、実測を見てからユーザーが決める。結果は ADR 0040 §2.4 の表と、この ADR に追記する。

## 9. 実装の進め方（決めること 5）

### 9.1 レーンと順序
0. **スパイク**（§8）。ユーザーの方式の決定（§10 Q1）のあとに 1 へ。
1. **契約コミット**（リード）: `proto/ikari/candidate/v1/candidate.proto`、`windows/candidate` の lib の骨組み（命令の型、行の高さの関数のシグネチャ）、`panel` の y 下向きの入口のシグネチャ、`ops` の候補の部分のシグネチャとスタブ。
2. **並行**（`isolation: worktree`。持つファイルの境界を決める）:
   - **L1 TIP**（`windows/tip/`）: アンカー（先頭の 1 文字、閉じるまで固定、DPI の変換、作業領域）、`ops` の候補の部分、送信（`post`、つなぎ直し）、`panel_shown`、フォーカスが外れたときの `Hide`。`ops` は Mac でテスト。
   - **L2 窓**（`windows/candidate/`、`shell-core/src/panel.rs` の y 下向きの入口）: lib の中身（行の高さ、命令を受けて「描く状態」を作る純粋な部分。最後だけを描く、持ち主の接続、切れたら隠す。Mac でテスト）と exe（パイプの受け手、Per-Monitor V2、Direct2D と DirectWrite、ライトとダーク、上下逆の描画、開発者の行、PNG の口）。
   - **L3 起動と道具**（`windows/engine-host/`、`windows/dev/`、手順書）: 子の起動と見張り、`build.ps1` と `start-engine.ps1` への追加、`candidate-probe.ps1`（窓の有無、矩形、拡張スタイル、前面の窓、DPI を書く）、手順書の新しい項目。
3. **統合**（リード）: `scripts/check.sh`、CI の Windows のジョブ、実機の手順書の新しい項目（§9.2）。
4. **W3-c**（UI 要素。§4.3）: 窓の後の別の issue。
5. 1.0.0 の後: 窓の UIA、ストアのアプリ。

### 9.2 実機で確かめる方法（画面が映らない前提）
- `candidate-probe.ps1`（`run-in-session.ps1` でセッション 1 で流す）: クラス名で窓を探し、見えているか、矩形、`WS_EX_NOACTIVATE` などのスタイル、前面の窓がホストのままか、画面と DPI を書く。キーは今までどおり `virsh send-key`。
- exe の PNG の口: 開発者向けの設定（`developer.*`）か、開発用の環境変数があるときだけ、描いた絵を開発用の場所に書く。**候補の文字が絵に入るので、既定ではオフ。ログには文字を出さない**（今のログの約束と同じ）。
- 合わせて読むもの: TIP の `dbgcap.ps1`（`show candidates` に、アンカー、`side`、送った `seq`）、exe のログ（受けた `seq`、描いた矩形、かかった時間）。`seq` で突き合わせる。
- 目で見る確認（ユーザー、5 分程度）: 見た目（字、色、角丸、影、ダーク）、ちらつき。方法は §10 Q3。

## 10. ユーザーの判断が要る点（推奨つき）

### Q1. 方式: Direct2D の自前描画を、別プロセス `ikari-candidate.exe` に置く（§3 の 1・2）でよいか
- 推奨: **はい。ただし本決定は §8 の S-1 と S-3 が通ってから**。通らなければ DLL の中（P-a）に切り替えて、もう一度聞く。

### Q2. 1.0.0 のアクセシビリティの範囲
- 推奨: **TSF の UI 要素（W3-c）は 1.0.0 に入れる。窓そのものの UIA は 1.0.0 の後**。S-4 でナレーターが UI 要素から読まないと分かったら、もう一度聞く。

### Q3. 目で見る確認の方法
- 推奨: **確認のときだけ、Mac から VM のコンソールの画面を開いて見る**（virt-viewer など。RDP は別のセッションになり、パイプの名前とエンジンが変わるので避ける）。手段が無ければ PNG の口の絵で代える。

### Q4. 窓に使える時間の割り当て（NFR-02 の 16ms の内訳）
- 推奨: **今は決めない。S-2 の実測のあとに、数値を出して聞く**。

見た目の値（字の大きさ、余白、色）は、macOS の比率に合わせて L2 が作り、Q3 の確認でユーザーに聞いて直す。字の書体は OS の UI の書体（Yu Gothic UI と Segoe UI）に従い、ライトとダークは OS の設定に従う。

## 11. 却下した案（まとめ）
- WebView2 の別プロセス（D-b）: §4.2。
- WinUI 3（D-c）: §4.2。
- engine-host に窓を置く（P-b）: §5.1。
- TIP が exe を起動する: ストアのアプリや管理者のアプリから起動すると、権限と寿命がアプリに引きずられる。起動は常駐の engine-host だけ。
- exe が上下を決めて返事で返す: 返事を待つと打鍵の道に往復が乗り、待たないと次のキー（↑/↓）に間に合わないことがある。行の高さを固定の関数にして TIP が決める（§6.2）。
- engine-host を中継する: 往復が増え、エンジンが落ちると窓も止まる（§5.3）。

## 12. 懸念と前提
- **すべての「推測」は実機で 1 回も確かめていない。** とくに S-1（DPI の変換）と S-3（別プロセスの窓が最前面に出てフォーカスを奪わない）が崩れると、置き場所（§5）が変わる。
- 1 つのプロセスに 1 つの窓なので、2 つのアプリで同時に候補を出すことはない（打鍵は前面の 1 つ）。持ち主の切り替えは「最後に Show を送った接続」で足りる見込み（推測）。
- `virsh screenshot` が窓を映さない原因は分かっていない（GPU を渡したあとの 2 つの画面のどちらかを映している可能性。推測）。確かめ方は PNG と道具で組み、画面に頼らない。
- exe が 1 つ増えるので、配布（W5）とセキュリティレビュー（W6）の対象が増える（パイプ 1 本、子の起動）。
- 行の高さを固定にするので、OS の「テキストのサイズ」（文字の拡大）に追従するかは、L2 で決める（追従するなら、倍率と同じく TIP と exe に同じ値を渡す）。
- ADR 0060 Q4 の本決定（LM の別プロセス）が変わっても、この ADR の決定は変わらない（見張りの共通化だけが変わる）。

## 参考
- ADR 0040 §2.2・§2.4・§6（S2、S10）・§7（W3）、ADR 0042、ADR 0014 §2.2、ADR 0061 §6.2、ADR 0060（PR #347）
- issue #319（W3-0）、#363（Windows の候補窓に開発者の行）、#86（アンカーを動かさない）
- [`docs/windows-real-device-check.md`](../windows-real-device-check.md) §1.6、項目 33
- スパイク `spike/w3-0-candidate-window` の `spikes/candidate-window/README.md`
