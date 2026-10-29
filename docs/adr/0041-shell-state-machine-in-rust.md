# ADR 0041: 入力の状態機械（IkariShellCore）を Rust に移し、macOS と Windows で共有する

- 状態: 提案
- 日付: 2026-10-30
- 対象: 方針と契約の形だけ（コードは変えない）。採ったときに触るのは、新しい crate `shell-core`、`shell-ffi`（口を足す）、新しい `proto/ikari/shell/v1/shell.proto`、`macos/Sources/IkariShellCore/**`（段階的に消す）、`macos/Sources/IkariIME/**`（状態機械の外の分岐を移す）、`macos/Sources/IkariXPC/**`（`Envelope` の組み立てを Rust に移す）、`macos/Tests/IkariShellCoreTests/**`（シナリオを共通の形式に移す）
- 前提: main = a43ccb1。ADR 0004 §3.3・§4.3（合成は殻のプロセスで Rust を FFI で呼ぶ）・§6.3（状態機械の表）・§7.5（Convert と Page は `lm_budget + 40ms` まで同期で待つ）・§12（Space から 1 ページ目まで p99 16ms 以下）。ADR 0006 §8.2・§16（文節のモード、layoutRev、restore）。ADR 0026（英語のモード）。ADR 0027（ショートコード）。ADR 0040（Windows。#126 の draft。main にはまだない）の §3.2 と §6 の S1
- タスク: #81（Windows）の前提。ユーザーの方針（2026-10-30）: 「状態機械は早く Rust に書き直すべきだ。移行は早いうちにやったほうが、変更が積み上がらなくて済む」。Windows は 1.0 に含めない。Windows の TIP は Rust。Windows の開発環境は x64 の VM（#128）

---

## 0. 現状の把握

### 0.1 確認した事実（行番号は main = a43ccb1）
- `macos/Sources/IkariShellCore/` は約 4,800 行。大きいのは `InputStateMachine.swift`（1,501）、`ShortcodeMachine.swift`（419）、`Settings.swift`（413）、`EnglishPostal.swift`（337）、`EngineClient.swift`（323）、`DiagEpisode.swift`（221）、`EnglishMode.swift`・`ShortcodeEnglish.swift`（各 213）、`Keymap.swift`（212）。どのファイルも `Foundation` だけを使い（`PanelPlacement.swift` は CoreGraphics の型だけ）、AppKit、IMK、XPC には依存しない。
- 入口は `InputStateMachine.handle(_:surrounding:selection:isRepeat:) -> [KeyAction]`（`InputStateMachine.swift:233`）。周辺テキストと選択範囲は `@autoclosure` で、Convert の直前と `reconvert` のキーのときだけ評価される。出力の `KeyAction`（`KeyAction.swift:100-131`）は `passThrough`、`consume`、`setMarkedText`、`commit`、`showCandidates`、`moveSelection`、`hideCandidates`、`restore`、`resolveStrayMarkedText`、`prefetch` の 10 種。範囲はすべて UTF-16 の単位（`TextRange`、`MarkedText.highlightedLength`・`segments`）。
- OS との境目は 3 つの protocol: `EngineClient`（`EngineClient.swift:243`。`throws` と `Duration`。誤りは `unavailable`・`timeout`・`invalidReply`・`engine(code)` の 4 種。`:221`）、`Composing`・`ScriptConverting`（`InputStateMachine.swift:7-21`。実体は shell-ffi の Rust）。時計は `diagClock`（`DispatchTime`）。
- `EngineClient` の実装（`IkariXPC/XPCEngineClient.swift`、529 行）は、`Envelope` を Swift で組み立て、`EngineTransport`（`IkariXPC/EngineTransport.swift:14-20`。`request(Data, timeout) throws -> Data` と片道の `post(Data)`）に渡す。つまり**エンジンとの口は、すでに「バイト列の往復」と「バイト列の片道」の 2 つに絞れている**。
- 殻（`IkariIME/IkariInputController.swift:90-132`）は、状態機械を呼ぶ前に、設定の同期、JIS の英数・かなキーの吸い込み（`EnglishMode.absorbsKeyCode`）、`IsSecureEventInputEnabled()` の読み取り、英語のモードの分岐（`handleEnglish`）、`mode.*` の切り替え（`EnglishMode.modeSwitchTarget`）を行う。`run(actions)`（`:301-335`）は `KeyAction` を順に実行し、`passThrough` を含めば false を返す。
- 設定は 2 か所で読む: 殻の `Settings.swift`（`settings.tsv` の殻のキー）と、エンジンの `engine/src/settings.rs`。設定アプリ（`IkariSettings`）も `IkariShellCore` の `Settings`・`SettingsFile` を使う。
- Rust の側の既存: `input`（ローマ字、JIS かな、字種。辞書に依存しない）と `shell-ffi`（`ikari_composer_*`、`ikari_convert_script`）。状態機械、キー割り当て、設定、ショートコードに当たる Rust はない。`engine-ffi` の口は `Envelope` のバイト列だけ。
- テスト: `macos/Tests/IkariShellCoreTests/` に Swift Testing で 559 件（29 ファイル）。`MockEngine` と `FakeComposer` で差し替える作り。単体とキー列のシナリオが混ざっている。`eval/` に状態機械のシナリオはない。
- 変更の頻度: 2026-10-28 の 1 日だけで十数件（#62、#64、#69〜#72、#74、#78、#84、#86〜#88、#103）が `IkariShellCore` に入った。

### 0.2 測っていないこと
- **`handle` 1 回ごとに `KeyAction` の列を FFI で渡す分の時間**。軽いと見込むが、推測。§7 の M0 で測る。
- 今の Swift の `handle` 1 回の時間（Convert を含まないキー）の分布。M0 で比べる元として一緒に測る。

## 1. 問題の定義
- Windows（ADR 0040）の殻は Rust で書く。状態機械を Windows 用に別に書くと（0040 の案 S-b）、同じ振る舞いを Swift と Rust で 2 回書き、2 回直すことになる。0.1 の頻度で `IkariShellCore` に変更が入る今、二重の期間は、ずれ（macOS で直した不具合が Windows に残る）を毎日生む。
- 移すなら、変更が積み上がる前のほうが安い。移す量は 1 日ごとに増える（ユーザーの方針）。
- 今の状態機械のテストは Swift（macOS）でしか流せない。`cargo test` で OS なしに流せれば、CI の Linux・Windows でも守れ、将来の Linux の殻（IBus・Fcitx）でも使える。

## 2. 決定（提案）

### 2.1 要約
1. **今、段階的に移す**（案 P-a）。Windows の W2 を待たない。
2. 新しい crate **`shell-core`**（Rust。`input` と `proto` に依存。OS に依存しない）に、状態機械、キー割り当て、殻の設定、英語のモード、ショートコード、郵便番号、診断の数え上げ、メニューの中身、候補窓の配置の計算を置く。
3. macOS からは **`shell-ffi` に足す C ABI** で呼ぶ。Windows の TIP は Rust なので、`shell-core` を **Rust の crate として直接**使う（C ABI を通らない）。
4. エンジンとの口は、殻が渡す **2 つのコールバック（バイト列の往復と片道）** にする。`Envelope` の組み立ては Rust（`proto` crate の prost）に移す。macOS は今の `EngineTransport`（XPC）を、Windows は名前付きパイプをそのまま渡す。
5. 移行の間は **Swift 版と Rust 版を、開発者向けの隠し設定で切り替えられる**ようにし、作者が Rust 版を日常で使ってから既定を替え、Swift 版を消す。
6. 振る舞いの基準は、**Swift と Rust の両方が読む共通のシナリオ（JSON）**にする（§5）。

### 2.2 置き場所（どこまで Rust に移すか）
| 置き場所 | 中身 |
| --- | --- |
| `shell-core`（共有） | `InputStateMachine`、`InputState`、`ShortcodeMachine`・`ShortcodeTable`・`ShortcodeEnglish`、`EnglishMode`（`classify`、`modeSwitchTarget`、`isPrintable`）、`EnglishPostal`、`Keymap`、`Settings`（殻のキーの解釈と検証）・`SettingsFile`（書き換え）、`SmallLetters`、`DiagEpisode`、`InputMenuModel`、`PanelPlacement`（座標の計算だけ）、`EngineTimeouts`・`LmBudget`、`Envelope` の組み立てと読み取り（今の `XPCEngineClient` の中身）、殻の中の英語のモードの分岐（今の `handleEnglish` のうち OS に依存しない部分。Q1） |
| 殻（OS ごと） | キーのイベントから `KeyInput` を作る（macOS は `KeyCodes.swift` の keyCode、Windows は仮想キー）、OS 固有のキーの吸い込み（JIS の英数・かな）、secure の読み取り（`IsSecureEventInputEnabled()`）、周辺テキストと選択範囲の取得、`KeyAction` の実行（マーク、確定、候補窓の描画）、入力メニューの `NSMenuItem` への写し、トランスポート（XPC・名前付きパイプ）と再接続・落ちた回数の休止（ADR 0004 §7.5） |

- `KeyCodes.swift` は macOS の keyCode の表なので殻に残す。`KeyInput` の意味（どのキーが `character`、`jis`、`space` か）は契約に入れる。
- 設定アプリ（`IkariSettings`）は、設定の検証と書き換えを `shell-ffi` 経由で Rust に頼む（同じ規則を 2 回書かないため）。エンジンの `engine/src/settings.rs` との統合は本 ADR の範囲外（別 ADR。キーの集合が違う）。
- 管理の要求（ADR 0005 §8.3。設定アプリからの `Envelope` の素通し）は今のまま Swift に残す。状態機械が使わないため。

## 3. 検討した案

| 案 | 中身 | 利点 | 欠点 |
| --- | --- | --- | --- |
| **P-a（推す）: 今移す。段階的、切り替えつき** | §2 | 二重の期間が短い（移行の間だけ）。量が増える前に移せる。`cargo test` で OS なしに守れる。Windows の W1・W2 が最初から共有の状態機械で作れる | 毎日使う macOS の殻の中心を書き換えるので、回帰の危険がある（§8 の対策）。移行の間は状態機械への新機能を止める（§6） |
| P-b: Windows の W2 の後に移す | 0040 の案 S-b の続き | Windows の見込みが立ってから決められる | Windows 用の Rust 版と Swift 版を並べて持つ期間が W2 まで続き、その間の Swift の変更を Rust に写し続ける。移す量も増える。ユーザーの方針に反する |
| P-c: Windows 用だけ別に書く（共有しない） | 0040 の S-b を共有なしで固定 | macOS に触れない | 二重の保守が恒久になる。ずれを止める手段がシナリオの共有しかない |
| P-d: エンジンの側に置く（0040 の案 S-c） | 殻はキーを全部エンジンに送る（Mozc の形） | 殻が一番薄い | macOS で今の「キーごとにエンジンへ往復しない」（ADR 0004 §3.3）を崩し、エンジンが落ちたときに入力できなくなる（§7.5 のフォールバックを殻で作れない）。ただし `shell-core` をライブラリにしておけば、同じものをエンジンのプロセスに載せることもできるので、Windows で採るかは 0040 の S1 で別に決められる（§4.4） |
| P-e: Windows でも Swift を使う（0040 の案 T-d） | — | 移行がない | 0040 §3.1 のとおり、全アプリのプロセスに Swift の実行環境を載せることになる。Windows の TIP は Rust に決まっている |

- 移し方の比較: 一気に書き換える案は、1,500 行の本体と周辺を同時に替えるので、回帰の原因を切り分けられない。2 つを同時に動かして出力を比べる案（シャドー）は、エンジンへの片道の送信（Commit、Prefetch、学習）が 2 回になるので採らない（片方を記録だけの替え玉のエンジンにつなぐ手はあるが、殻の中に 2 つの状態を持つ複雑さに見合わない）。

## 4. 契約

### 4.1 共通の約束
- **文字列は UTF-8**（FFI の上は今の `IkariStr` と同じ「ポインタと長さ」）。**位置と長さは UTF-16 の単位**のまま（`TextRange`、`highlightedLength`、`segments`）。macOS の `NSRange` も Windows の TSF（ACP）も UTF-16 の単位なので、殻で換算しない。Rust の中では UTF-16 の長さを数える関数を 1 か所にまとめ、`String::len()`（バイト数）を位置に使わない（型（UTF-16 の長さの newtype）で防ぐか、レビューの規則にするかは契約コミットで決める）。
- **スレッド**: 状態機械は 1 つのコントローラが持ち、1 つのスレッドから呼ぶ（Rust では `Send` だが `Sync` ではない）。コールバックは `handle` の中で同期に呼ばれ、コールバックの中から状態機械を呼び直してはいけない。
- **panic**: FFI の口はすべて `catch_unwind` で包む。panic したら、その状態機械を壊れた扱いにし、その回は `passThrough` を返し、殻は状態機械を作り直す。Windows の TIP はアプリのプロセスに入るので、panic でアプリを落とさないことが特に要る（`shell-core` の Rust の入口でも同じ扱いにする）。
- **時計**: Rust の単調時計（`std::time::Instant`）を使い、テストでは差し替える（trait）。

### 4.2 入力と出力（`proto/ikari/shell/v1/shell.proto`）
- `KeyInput` と `KeyAction` の列は、新しい proto のメッセージにする（`KeyInput`、`MarkedText`、`CandidatePanelModel`、`KeyAction`（oneof）、`HandleResult`（`KeyAction` の列）、`SurroundingText`、`SelectedText`）。macOS は swift-protobuf の生成コードをコミットする（ADR 0004 §4.1 と同じ）。Rust は prost。
- 理由: `CandidatePanelModel` のように入れ子と列がある型を C の構造体で書くと、手書きの解放と版の管理が増える。proto ならすでに両方の道具がある。**ただし、1 キーごとの符号化と復号の時間は M0 で測る。目標に対して大きければ、C の構造体（平らな配列と文字列のプール）に替える**。この切り替えは `shell-ffi` の中に閉じ、`shell-core` の型は変えない。
- Windows の TIP は proto を使わず、`shell-core` の Rust の型をそのまま受け取る。

### 4.3 C ABI（`shell-ffi/include/ikari_shell.h` に足す。形の案）
```c
typedef struct IkariShellHost {
  void *ctx;
  /* エンジンへの往復。成功なら 0 と返事のバイト列。失敗は IKARI_SHELL_E_*（unavailable / timeout） */
  int32_t (*engine_request)(void *ctx, IkariStr envelope, uint32_t timeout_ms, IkariShellBuf *reply);
  /* 片道。返事を待たない */
  void (*engine_post)(void *ctx, IkariStr envelope);
  /* 遅延の取得。handle の中で、要るときだけ同期に呼ばれる。proto の SurroundingText / SelectedText */
  int32_t (*surrounding)(void *ctx, IkariShellBuf *out);
  int32_t (*selection)(void *ctx, IkariShellBuf *out);
} IkariShellHost;

IkariMachine *ikari_machine_new(IkariStr settings_tsv, IkariStr keymap, IkariShellHost host);
IkariShellBuf ikari_machine_handle(IkariMachine *m, IkariStr key_input, bool is_repeat); /* HandleResult */
void ikari_machine_set_secure(IkariMachine *m, bool secure);
void ikari_machine_set_panel_side(IkariMachine *m, uint8_t side);
/* ほか、今の public な口（activateIfNeeded、deactivate、commitComposition(reason)、設定の適用、メニューの状態）を 1 対 1 で写す */
void ikari_machine_free(IkariMachine *m);
```
- `@autoclosure` の遅延は、コールバックで表す。2 段の呼び出し（「周辺テキストが要る」を返して殻が呼び直す）は、状態機械を途中で止めて再開する形になり、本体の書き方を大きく変えるので採らない。
- エンジンの誤りの 4 種（0.1）は、`engine_request` の戻り値（トランスポートの誤り: unavailable、timeout）と、返事の中身（Rust の側で読む: invalidReply、engine(code)）に分かれる。Swift の `throws` は Rust の `Result` に写す。
- `Composing`・`ScriptConverting` の protocol はなくなる（`shell-core` が `input` を直接使う）。今の `ikari_composer_*` は移行が済むまで Swift 版のために残し、M5 で消す。
- 各メッセージの欄の名前と番号、誤りの番号は、契約コミット（M1）で決める。

### 4.4 Windows の側
- TIP（Rust）は `shell-core` に依存し、ホストの trait（往復、片道、周辺テキスト、選択）を名前付きパイプと TSF で実装する。
- 状態機械を TIP の DLL に置くか、エンジンの側に置くかは、0040 の S1（キー 1 回の往復の時間）で決める。`shell-core` はどちらにも載る（エンジンの側に置くなら、往復のコールバックがプロセスの中の呼び出しになるだけ）。

## 5. シナリオの共通の形式
- 置き場所: `shell-core/scenarios/*.json`。Rust のテスト（`cargo test -p ikari-shell-core`）と Swift のテスト（`IkariShellCoreTests` の 1 つの実行器）が同じファイルを読む。
- 1 つのシナリオ: 名前、設定（`settings.tsv` の行）、キー割り当て、替え玉のエンジンの台本（受け取るはずの要求の種類と主な欄、返す返事。順に）、手順の列（キー、修飾、isRepeat、その時点の周辺テキストと選択範囲、secure、期待する `KeyAction` の列、期待するエンジンへの送信）。
- 比べ方: `KeyAction` の列は順序つきで完全一致。エンジンへの送信は、台本に書いた欄だけを比べる（`request_id` などは比べない）。
- 559 件のうち、キー列で書けるもの（シナリオ）は JSON に移す。状態機械の中の関数の単体テストは移さず、言語ごとに書く（Rust に移した関数は Rust の単体テストにする）。何件がシナリオに当たるかは M1 の初めに数える。
- JSON にする理由: Rust は serde、Swift は Codable で、追加の道具なしに両方が読める。人が差分を読める。proto の JSON の対応は prost に標準でないので使わない。

## 6. 移行の間の運用
- 期間: M1（契約）の承認から M5（Swift 版を消す）まで。
- **状態機械への新機能の追加は止める**（Q2）。止められないもの（ユーザーが急ぐもの）は、**Rust の側に先に入れ**、Swift 版には入れない（Swift 版が既定の間は、その機能は Rust 版に切り替えたときだけ使える）。
- **バグ修正は Swift と Rust の両方に入れてよい**。ただし、**修正には必ず共通のシナリオを 1 つ以上足す**（両方が通ることでずれを止める）。まだ Rust に移していない部分の修正は、Swift とシナリオだけでよい（移すときにシナリオが Rust を縛る）。
- 殻（`IkariIME`）の変更のうち、§2.2 で `shell-core` に移すと決めた部分も同じ扱い。
- 移行の PR は 1 段 1 PR を基本にする（1 ブランチ 1 タスク）。

## 7. 段階と完了の条件
| 段 | 中身 | 完了の条件 |
| --- | --- | --- |
| **M0: スパイク（捨てるコード）** | FFI の分の時間を測る（下の手順） | 数値を本 ADR に追記し、ユーザーが §4.2 の形（proto か C の構造体か）を決める |
| **M1: 契約** | `shell.proto`、`ikari_shell.h` の追加分、`shell-core` の空の crate（型とスタブ）、シナリオの JSON の形と Swift の実行器、既存のシナリオの JSON への移し替え | Swift 版が全シナリオを通る。`scripts/check.sh` |
| **M2: 周辺の純粋な部分** | `Keymap`、`Settings`・`SettingsFile`、`SmallLetters`、`ShortcodeTable`、`InputMenuModel`、`PanelPlacement`、`EngineTimeouts`・`LmBudget`、`Envelope` の組み立て | Rust の単体テストが Swift の単体テストの項目を写している。設定アプリが Rust の検証を使い、Swift 版の `Settings` と同じ入力で同じ警告になる。`check.sh` |
| **M3: 英語のモード、ショートコード、郵便番号、診断** | `EnglishMode`、`ShortcodeMachine`・`ShortcodeEnglish`、`EnglishPostal`、`DiagEpisode` | 該当のシナリオが Rust で通る |
| **M4: 本体と切り替え** | `InputStateMachine` と、殻の英語のモードの分岐。`shell-ffi` の口。殻の隠し設定で Swift 版と Rust 版を切り替える | 全シナリオが Rust で通る。`check.sh`。smoke の Convert の時間が ADR 0004 §12 の目標（p99 16ms 以下）を満たし、Swift 版の値から悪化しない。作者が Rust 版で日常に使い、決めた期間に回帰の報告がない（Q4） |
| **M5: 既定を Rust 版にし、Swift 版を消す** | `IkariShellCore` の状態機械と、使われなくなった `ikari_composer_*` などの口を消す | `check.sh`、smoke。`docs/architecture.md` の更新 |

### M0 の手順（計測の小さなスパイク。値の基準は決めない）
1. 捨てるブランチで、`shell-ffi` に「`KeyInput` を受け、代表の `HandleResult`（`setMarkedText` 1 つと、注釈つきの 9 件の `showCandidates` 1 つ）を返す」だけの関数を作る。proto の版と、C の構造体の版の 2 つ。
2. Swift の release ビルドの小さな実行ファイルから 1 万回呼び、1 回あたりの時間の中央値と 99% を出す（符号化、FFI、復号、解放を含む）。周辺テキストのコールバックを 1 回はさむ版も測る。
3. 今の Swift の `handle` の時間を、スパイクのログで作者の日常の打鍵から集める（Convert を含むキーと含まないキーに分ける）。検証版を入れる前に、テストの入力と通常の入力を確かめる。smoke を使うときは、ユーザーの了解を取ってから。
4. 結果を本 ADR に追記する。比べる目標は ADR 0004 §12 の「Space から 1 ページ目まで p99 16ms 以下（XPC 込み）」。その中で FFI の分が占める割合を示し、**採る形と許す時間はユーザーに聞いて決める**。

## 8. 危険な箇所と対策
| 箇所 | 危険 | 対策 |
| --- | --- | --- |
| 文節のモードの `layoutRev` と、区切り直しの後の選択の持ち回し（ADR 0006 §16） | 古い `layoutRev` の返事を当てる。選択がずれる | 区切り直し、ページ、確定のシナリオを M1 で重点的に JSON にする。エンジンの台本で `layoutRev` の不一致の返事も流す |
| 再変換の `TextRange`（UTF-16）と `restore` の範囲 | バイト数・文字数と UTF-16 の混同。絵文字（サロゲートペア）、異体字セレクタ、結合文字で崩れる | §4.1 の区別。それらを含むシナリオを足す |
| `passThrough` を含む列の順序と、`handle` の Bool | 「確定してから素通し」の順が崩れると、文字が消えるか二重になる | 列の完全一致で比べる（§5）。secure と `resolveStrayMarkedText` のシナリオ |
| 殻の外の分岐（`handleEnglish`、`mode.*`、吸い込み） | 移す・残すの境目で、二重に処理するか、抜ける | §2.2 の表を M1 の契約にし、殻に残す処理の一覧をヘッダの注釈に書く |
| 同期の待ち（Convert、Page） | Rust の中の待ちが、殻の主スレッドを今より長く止める | 待つのはコールバックの中（殻のトランスポート）だけにし、上限は今の `EngineTimeouts` を写す。M4 の smoke で比べる |
| panic | macOS では入力が止まり、Windows ではアプリが落ちる | §4.1 の `catch_unwind` と作り直し。`shell-core` では `unwrap` を clippy で禁じる |
| 移行の間のずれ | Swift にだけ入った修正 | §6 の「修正には共通のシナリオ」 |
| 作者の日常の入力が止まる | M4 で Rust 版に切り替えたあとの不具合 | 隠し設定ですぐ Swift 版に戻せる（再起動なしで効くかは M1 で決める）。入れ替えたら、開いているアプリを再起動する |

## 9. ADR 0040 §3.2 との関係
- 0040 の推す案 S-b（Windows 用に書き、共有は W2 の後に決める）を、本 ADR の P-a（今、共有の `shell-core` に移す）で置き換える。0040 の Q3 の推す答えは「共有する（ADR 0041）」に改める。
- 0040 の S-a は本 ADR の P-a と同じ向き。0040 が挙げた危険（毎日使う殻の中心の書き換え）は、§7 の段階と §8 の切り替えで受ける。
- 0040 の S-c（エンジンの側に置く）は、本 ADR の P-d として macOS では採らない。Windows の置き場所は 0040 の S1 で決める（§4.4）。
- 0040 §4・§8 の「状態機械が Swift と Rust の 2 つになる期間」は、本 ADR の移行の期間（M1〜M5）だけになる。0040 の §3.2、§4、§8、§9 の Q3 は、本 ADR に合わせて 0040 の PR（#126）で直す。
- 着手の順: W0（コアの Windows のビルド）は本 ADR と並べて進めてよい（触るファイルが重ならない）。W1（TIP のスパイク）は、M1 の契約の後なら `shell-core` の最小の口（読み、Space、確定）で作れる。

## 10. レーン分けとファイルの境界（実装に入るときの案）
- M0 と M1 はそれぞれ 1 レーン（M1 は契約コミット）。
- M2・M3 は並行にできる。

| レーン | 持つファイル | 段 |
| --- | --- | --- |
| R1: Rust の周辺 | `shell-core/src/` の `keymap.rs`、`settings.rs`、`settings_file.rs`、`small_letters.rs`、`menu.rs`、`panel.rs`、`timeouts.rs`、`envelope.rs` とそのテスト | M2 |
| R2: Rust の英語・ショートコード | `shell-core/src/` の `english.rs`、`shortcode.rs`、`postal.rs`、`diag.rs` とそのテスト | M3 |
| S1: Swift の側 | `shell-ffi/src/machine.rs`（FFI の口）、`macos/Sources/IkariIME/`（切り替え、ホストのコールバック）、`macos/Sources/IkariSettings/`（設定の検証を Rust に）、`macos/Tests/IkariShellCoreTests/` のシナリオの実行器 | M2〜M4 |
| R3: Rust の本体 | `shell-core/src/machine/` | M4。1,500 行の本体は分けにくいので 1 レーン。同期の待ちと状態の持ち回しが難しいので Opus を使う（CLAUDE.md のモデルの使い分け） |

- 共有の型（`shell-core/src/types.rs`、`shell.proto`、`ikari_shell.h`）は M1 で固め、レーンは変えない。
- `IkariShellCore` の Swift のファイルは、移行の間は §6 のとおり新機能を止める。消すのは M5 だけ。

## 11. 結果（採ったときに変わること）
- 状態機械の正が Rust になる。`cargo test` で OS なしに振る舞いを守れ、CI の Linux・Windows でも流せる。Windows の TIP と、将来の Linux の殻が同じものを使う。
- macOS の殻は、イベントの変換、テキストの取得と書き込み、描画、トランスポートだけの薄い層になる（ADR 0004 §3 の方針の延長）。
- `docs/architecture.md` の依存の向きに「`input`、`proto` ← `shell-core` ← `shell-ffi`」が入る。`Envelope` の符号化の正が Rust の 1 か所になる（Swift の `EnvelopeCodec` のうち状態機械の分は消える）。
- 失うもの: 移行の間（M1〜M5）、状態機械への新機能が止まるか、Rust 版でしか使えない。殻を直すのに Rust の知識が要り、Swift だけで殻を直す手軽さがなくなる。

## 12. ユーザーに聞くこと
- **Q1. 移す範囲**。推す: `IkariShellCore` に加え、`IkariInputController` の中の OS に依存しない分岐（英語のモードの `handleEnglish`、`mode.*` の切り替え）まで `shell-core` に含める（§2.2）。理由: 残すと Windows で同じ分岐をもう一度書く。OS 固有のもの（keyCode、secure の読み取り、JIS の英数・かなの吸い込み、描画）は殻に残す。
- **Q2. 移行の間の方針**。推す: 状態機械と、§2.2 で移す殻の部分への新機能の追加を止める。バグ修正は可で、必ず共通のシナリオを足す。止められない新機能は Rust 版に先に入れ、Swift 版には入れない（§6）。
- **Q3. Swift 版と Rust 版の切り替えの出し方**。推す: 開発者向けの隠し設定（`settings.tsv` のキー。設定アプリには出さない）。M5 で消す。利用者に見せると、外すときに説明が要るため。
- **Q4. M4 の日常利用の期間**。推す: 作者が Rust 版を既定にして 3 日、回帰の報告がなければ M5 に進む。3 日は根拠のない目安なので、日数は作者が決める。
- **Q5. FFI の形と許す時間**。推す: M0 の計測の後で決める。形は proto を第一にし、目標（ADR 0004 §12）に対する割合が大きければ C の構造体にする。許す時間の数値は、計測の結果を見てユーザーが決める。

## 参考
- [ADR 0004](0004-phase2-macos.md) §3.3、§4.3、§6.3、§7.5、§12
- [ADR 0006](0006-segment-editing.md) §8.2、§16
- [ADR 0026](0026-english-mode-and-key-stats.md)、[ADR 0027](0027-emoji-input.md)、[ADR 0034](0034-diagnostic-log.md)
- ADR 0040（Windows。#126 の draft PR。main にはまだない）§3.2、§6 の S1、§9 の Q3
- タスク: #81、#128
