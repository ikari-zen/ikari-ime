# ADR 0041: 殻の OS に依存しない処理を Rust（`shell-core`）に移し、macOS と Windows で共有する

- 状態: 提案
- 日付: 2026-10-30（同日、ユーザーの決定と棚卸しに合わせて改訂）
- 対象: 方針、範囲、境界の形、段階、レーンと契約、性能の基準（コードは変えない）。採ったときに触るのは、新しい crate `shell-core`、`shell-ffi`（口を足し、古い口を消す）、新しい `proto/ikari/shell/v1/shell.proto`、`macos/Sources/IkariShellCore/**`（消す）、`macos/Sources/IkariXPC/**`・`IkariBridge/**`・`IkariEngineService/EngineGate.swift`（トランスポートと OS の検査だけを残す）、`macos/Sources/IkariIME/**`（薄い入口と UI だけを残す）、`macos/Sources/IkariSettings/Model/**`（検証と整形を Rust に頼む）、`macos/Tests/**`（シナリオを共通の形式に移す）。スクリプトの `cargo xtask` は [ADR 0043](0043-xtask.md)
- 前提: main = a43ccb1。ADR 0042（技術選定の方針。PR #137。2026-10-30 ユーザー承認。main にはまだない）。ADR 0004 §3.3・§4.3・§5.1・§6.3・§7.5（Convert と Page は `lm_budget + 40ms` まで同期で待つ。落ちた回数での休止）・§12（Space から 1 ページ目まで p99 16ms 以下）。ADR 0005 §8.3（設定アプリからの管理の要求）。ADR 0006 §8.2・§16。ADR 0026、0027、0032、0034。ADR 0040（Windows。PR #126。main にはまだない）
- タスク: #138（Rust への移行。全タスクのなかで最優先）。#81（Windows）、#128

---

## 0. 現状の把握

### 0.1 確認した事実（行番号は main = a43ccb1）
- `macos/Sources/IkariShellCore/` は約 5,400 行（researcher の `wc -l`）。大きいのは `InputStateMachine.swift`（1,501）、`ShortcodeMachine.swift`（419）、`Settings.swift`（413）、`EnglishPostal.swift`（337）、`EngineClient.swift`（323）、`DiagEpisode.swift`（221）、`EnglishMode.swift`・`ShortcodeEnglish.swift`（各 213）、`Keymap.swift`（212）、`InputMenuModel.swift`（176）、`PanelPlacement.swift`（114）。import は `Foundation` と `IkariProto` だけ。
- 入口は `InputStateMachine.handle(_:surrounding:selection:isRepeat:) -> [KeyAction]`（`InputStateMachine.swift:233`）。周辺テキストと選択範囲は `@autoclosure` で、要るときだけ評価される。出力の `KeyAction`（`KeyAction.swift:100-131`）は 10 種。範囲はすべて UTF-16 の単位。
- エンジンの口: `EngineClient`（`EngineClient.swift:243`）の実装 `XPCEngineClient`（529 行）は、`Envelope` の組み立て、要求番号、再接続（世代）、落ちた回数の休止（`recordCrash`、`handleDown`）を持ち、下の `EngineTransport`（`IkariXPC/EngineTransport.swift:14-20`）に渡す。`EngineTransport` は `request(Data, timeout) throws -> Data`、`post(Data)`、`close()`、`processIdentifier` の 4 つと、落ちた通知 `TransportDown`（`interrupted`・`invalidated`）だけで、**メッセージの意味を知らない**。`request` はセマフォで同期に待つ（`:66-88`）。
- 殻（`IkariIME`）の中に、OS に依存しない判断がある（researcher の棚卸し。関数名から判断）: `IkariInputController` の `selectMode`、`leaveCurrentMode`、`recordModeSwitch`、`syncSettings`、`syncEmojiTable`、`handleEnglish`、`flushEnglishShortcode`、`flushEnglishPool`、`requestPostalLookup`、`apply(EnglishTextOutcome)`、`AnchorMemory`。`AppContext` の `settingsDidChange`、`restartEngine`、`refreshEngineInfo` のリトライ（`AppContext.swift:191` の `asyncAfter`）、`perform(MenuCommand)`、`nextSessionID`。`ClientAccess.isAvailable(left:documentLength:)`（純関数）。`BridgeServer` の `handle`、`apply(UpdateSettings)`、`snapshotFrame`、`relay`、`RelayGate`。`SettingsStore` の `reloadIfFileChanged`、`update(set:reset:)`。
- ほかに OS に依存しないもの: `IkariXPC` のコーデックと再接続・休止（`EnvelopeCodec` 218、`XPCEngineClient` 529、`EngineAdmin` 89 ほか、計 約 950）。`IkariBridge`（`FrameCodec` 78、`RelayPolicy` 51、`BridgeProtocol` 65、`BridgeTransport` 157）。`IkariEngineService/EngineGate.swift`（75）。`IkariSettings` の Model 約 1,850（`@Observable` の UI の状態と、検証・整形が混ざる）と Bridge・Fake 約 830。
- 既存の Rust: `shell-ffi`（524 行、`extern "C"` 11 個。composer と `convert_script`）、`engine-ffi`（265 行、5 個）、`input`（1,419 行）。状態機械、設定、英語、ショートコードの Rust 実装はなく、重複は composer の周りだけ。ヘッダは手書き（cbindgen なし）。macOS のビルドは `macos/scripts/build.sh` が cargo で staticlib を作り、XCFramework にまとめる。
- テスト: Swift のテストのうち約 470 件は純関数的（状態機械、設定、キー、コーデック）で、共通のシナリオにしやすい。しにくいのは `XPCEngineClient` の 50（非同期、再接続）、Bridge 系の 46、AppKit 系の約 15、設定の Model の約 100（`@Observable`）。
- 性能の計測の今: `scripts/check.sh --perf`（`run_perf`、`check.sh:240-247`）は、`ikari-core` の perf テストと、エンジンの文節・再変換（`segment_perf`。p99 2ms 以下、LM なし）だけを測る。**殻（状態機械、FFI、XPC）の時間は測っていない**。XPC 込みの変換の往復は smoke（`macos/Sources/IkariSmoke/main.swift:417-425`）が p50・p99・max を出し、ADR 0004 §12 の予算（`latencyBudgetMillis`）を超えたら FAIL にする。キー 1 回の時間、殻の起動の時間、殻のメモリは、どこでも測っていない。

### 0.2 確かめていないこと（推測を含む）
- `handle` 1 回ごとに入出力を FFI で渡す時間。M0-a で測る。
- Rust（`objc2` など）から InputMethodKit の入口を作れるか。M0-b で試す。
- `InputStateMachine` が Swift の並行処理に依存する箇所の有無。`BridgeServer` の `handle` がどのスレッドで状態機械の状態を読むか。M1 の初めに確かめる。
- Windows の TSF で同じ `KeyAction` の列が使えるか（ADR 0040 の W1 で確かめる）。

## 1. 問題の定義
- 殻の判断が Swift にあり、Windows（TIP は Rust。ADR 0040）でもう一度書くと、二重の保守とずれが恒久になる。
- 判断が Swift の 3 か所（`IkariShellCore`、`IkariIME`、`IkariXPC`・`IkariBridge`）と Rust（composer）に散り、言語をまたぐ境界が多い。境界ごとに型の写しと誤りの写しが要り、バグの元になる（ADR 0042 §1）。
- 移す量は、`IkariShellCore` に変更が入るたびに増える（2026-10-28 の 1 日で十数件）。
- 移行の前後で性能を比べる基準がない（§0.1 の最後）。ユーザーは「移行前より悪化しないか確かめたい」（2026-10-30）。

## 2. 決定

### 2.1 要約（2026-10-30 ユーザー決定を反映）
1. **OS に依存しない殻の処理は、すべて `shell-core`（Rust）に移す**（ADR 0042 §1・§2。範囲は §2.2）。Swift に残すのは「薄い OS の入口」と「UI」だけ。
2. **設計の規則: 入口に判断（分岐）が増えたら Rust に移す。** 入口の関数は、OS のイベントを `shell-core` の入力に写すことと、`shell-core` の返す動作を実行することだけを行う。レビューで、入口に OS の API を呼ばない `if`・`switch` が足されたら、`shell-core` に移すよう求める。
3. **境界は 1 本の口にする**（§4）。「OS のイベント → `shell-core` → 動作の列」。エンジンとの通信（`Envelope`、再接続、休止、検証）も Rust に置き、OS 側はバイト列を運ぶトランスポートだけを持つ。
4. **大胆に移す**（ADR 0042 §1-6）。併存の期間、Swift 版と Rust 版の切り替えの設定、日常利用の日数、新機能の凍結は持たない（旧 Q2〜Q4 は不要になった。§11.1）。守るのは**共通のシナリオ（JSON）の一致**と、**性能の基準値から悪化しないこと**（§6.3）。
5. 移行は**並行のレーン**で進める（§7）。作者の実機は、レーンが終わるたびに入れ替える。
6. macOS の入口を Rust で書けるか（M0-b）を先に試す。だめなら Swift の薄い入口で続ける（§6.2）。
7. 移行の前に性能の基準値を取り（M0-c）、レーンの統合ごとに同じ計測を流す。測る道具は `cargo xtask perf` にする（§6.3、ADR 0043）。

### 2.2 置き場所
| 置き場所 | 中身 |
| --- | --- |
| `shell-core`（Rust、共有） | `IkariShellCore` の全部（状態機械、ショートコード、英語のモード、郵便番号、キー割り当て、殻の設定と書き換え、診断の数え上げ、メニューの中身、候補窓の配置の計算、時間の予算）。エンジンとの通信（`Envelope` の組み立てと読み取り、要求番号、再接続の世代、落ちた回数の休止、`EngineAdmin` の素通し、`EngineGate`）。設定アプリとの連絡（`FrameCodec`、`RelayPolicy`、`BridgeProtocol`、`BridgeServer` の `handle`・`apply(UpdateSettings)`・`snapshotFrame`・`relay`・`RelayGate`）。殻の判断（§0.1 の `IkariInputController`・`AppContext`・`ClientAccess.isAvailable`・`SettingsStore` の関数）。設定アプリの Model のうち、検証、整形、ファイルの読み書き（§2.3） |
| macOS の薄い入口（Swift。M0-b の結果で Rust にする可能性あり） | `IkariInputController` の `handle`（`NSEvent` → キーの入力）と `modifiers(of:)`。`ClientAccess` の IMK の呼び出し（周辺テキスト、選択、マーク、確定）。`AccessibilityAnchor`。`IsSecureEventInputEnabled()`。XPC の `EngineTransport`（97）、`Identifiers`（73）、`PeerSignature`（28）、`BuildSigning`（18）。`BridgeServer` の待ち受けと相手の検査（`makeListener`、`acceptLoop`、`admit`、`peerHasSameUser`、`peerSatisfies`）。`AppContext.openSettings`。エンジンのサービスの `main`。`KeyCodes.swift`（macOS の keyCode の表） |
| macOS の UI（Swift。ADR 0042 §1-4 で保守性を妥協してよい所） | `CandidatePanel`、`MenuBuilder`（`NSMenu` への写し）、`MarkedTextAttributes`、`SettingsLauncher`、`IkariSettings` の View と `@Observable` の状態 |
| 例外（Rust にしない。ADR 0042 §3） | `scripts/diag-report.py`（診断の集計。Python の分析の道具で、殻とエンジンの実行に関わらない） |

### 2.3 設定アプリの Model
- Rust に移すのは、設定のキーの解釈と検証（警告の種類を含む）、`settings.tsv` の書き換え（`SettingsFile`）、既定値、管理の要求の組み立て。
- Swift に残すのは、`@Observable` の状態（画面の選択、編集中の値、非同期の読み込みの進み）と、View への写し。
- 境界をまたいで受け渡すのは、「設定の一式（proto）」と「検証の結果（キーと警告の種類の列）」の 2 つだけにする。どこまでを Rust の型にするかの細部は §11 の Q1。

## 3. 検討した案
| 案 | 中身 | 判断 |
| --- | --- | --- |
| **R-a（採る）: 殻の OS 非依存の処理を全部、今、並行のレーンで移す** | §2 | ADR 0042 の方針どおり。境界が 1 本になり、Windows の TIP と同じ Rust を使える。回帰の危険は共通のシナリオで、性能の悪化は基準値との比較で受ける |
| R-b: `IkariShellCore` だけ移す（旧版の範囲） | `IkariInputController`、XPC、Bridge の判断は Swift に残す | 判断が入口に残り、Windows で同じ分岐を書き直す。境界が状態機械と XPC の 2 本になる。ユーザーの決定（旧 Q1「可能な限り Rust」）に反する |
| R-c: 段階的に移し、隠し設定で Swift 版と Rust 版を切り替え、日常利用の期間を置く（旧版の P-a） | — | 作者以外の利用者がいない今は、併存の費用（両方への修正、切り替えの分岐、凍結）に見合わない（2026-10-30 ユーザー決定）。作者の日常利用はテストを兼ね、壊れても代替手段がある |
| R-d: 状態機械をエンジンのプロセスに置く（Mozc の形。ADR 0040 の S-c） | 殻はキーを全部エンジンに送る | macOS ではキーごとの往復が増え、エンジンが落ちたときに入力できなくなる（ADR 0004 §3.3・§7.5）。`shell-core` はライブラリなので、Windows で採るかは ADR 0040 の S1 で別に決められる |
| R-e: 一度に全部を書き換えて 1 回で入れ替える | — | 1,500 行の本体と周辺を同時に替えるので、回帰の原因を切り分けられない。§7 のレーンの単位で入れ替える |

## 4. 境界の設計

### 4.1 形: 1 本の口
```text
OS のイベント ──▶ shell-core ──▶ 動作の列
（キー、フォーカス、メニュー、設定の変更、タイマー、Bridge の受信）
                    │  ▲
       要るときだけ │  │ 同期の返事
                    ▼  │
  ホストの口（周辺テキスト、選択、エンジンへの往復と片道）
```
- `ShellEvent`（oneof）: `Key`（キー、修飾、`isRepeat`、secure）、`Activate`（アプリの bundle ID、文書の長さなど `isAvailable` に要る生の値）、`Deactivate`、`MenuCommand`、`SettingsFileChanged`、`Timer`（id）、`BridgeFrame`（接続の id とバイト列）。
- `ShellAction`（oneof）: 今の `KeyAction` の 10 種に加え、`ShowMenu`（`InputMenuModel`）、`ScheduleTimer`（id、ミリ秒）、`CancelTimer`、`BridgeReply`（接続の id とバイト列）、`OpenSettings`。
- 戻り値の「キーを食べたか」は、列に `passThrough` を含むかで決める（今の `run(actions)` と同じ）。
- タイマーは `shell-core` が `ScheduleTimer` で頼み、OS 側は時間が来たら `Timer` のイベントを渡すだけにする（今の `asyncAfter` によるリトライなど）。
- 周辺テキストと選択は、ホストの口のコールバックで要るときだけ同期に取る（今の `@autoclosure` と同じ遅延）。「要る」と返して呼び直させる 2 段の形は、本体を途中で止めて再開する書き方になるので採らない。

### 4.2 共通の約束
- 文字列は UTF-8。位置と長さは UTF-16 の単位（`NSRange` も TSF の ACP も UTF-16）。Rust の中では UTF-16 の長さの newtype（`Utf16Len`、`Utf16Range`）を使い、`String::len()` を位置に使わない（契約の型で防ぐ）。
- スレッド: 状態機械は 1 つのスレッド（macOS は主スレッド）から呼ぶ。`Send` だが `Sync` ではない。コールバックの中から状態機械を呼び直さない。例外は §4.3 の落ちた通知だけ。
- panic: FFI の口はすべて `catch_unwind` で包む。panic したらその回は `passThrough` を返し、殻は `shell-core` を作り直す。`shell-core` では `unwrap`・`expect` を clippy で禁じる。Windows の TIP はアプリのプロセスに入るので、Rust の入口でも同じ扱いにする。
- 時計は `shell-core` の trait にし、本番は `std::time::Instant`、テストは差し替える。

### 4.3 エンジンとの通信を Rust に置く
- `shell-core::engine::EngineLink` が、今の `XPCEngineClient` の中身（`Envelope` の組み立てと読み取り、要求番号、誤りの 4 種への写し、`isValid`・`clipped` の検証、再接続の世代、落ちた回数の休止と `shutdownAndWaitForExit` の判断、診断の数え上げの片道、`EngineAdmin` の素通し）を持つ。
- OS 側に残るのは、**メッセージの意味を知らないトランスポート**だけ（今の `EngineTransport` と同じ線）。ホストの口は 5 つ:
  - `connect() -> handle`（macOS は `NSXPCConnection` を作り、署名の要件を付けて `resume`。Windows は名前付きパイプを開く）
  - `request(handle, bytes, timeout_ms) -> 結果`（0 と返事のバイト列、unavailable、timeout のどれか。同期。今のセマフォの待ちをそのまま使う）
  - `post(handle, bytes)`
  - `close(handle)`
  - `pid(handle) -> i32`
- 落ちた通知（XPC の `interruptionHandler`・`invalidationHandler`）は XPC のキューから来る。これを主スレッドに回すと、主スレッドが `request` で待っている間に届かない。そこで **`EngineLink` の再接続の状態（世代、落ちた時刻の列）だけを内部の `Mutex` で守り、`ikari_shell_engine_down(generation, kind)` をどのスレッドからでも呼べる口にする**（境界の唯一の例外。今の `handleDown(_:generation:)` と同じ意味）。
- 誤りの写し: トランスポートの誤り（unavailable、timeout）はホストの戻り値、中身の誤り（invalidReply、engine(code)）は Rust が返事を読んで決める。
- 管理の要求（設定アプリ → IME → エンジン。ADR 0005 §8.3）も、Bridge の受信（`BridgeFrame`）→ `shell-core` の `RelayGate`・`RelayPolicy` → `EngineLink` の素通し → `BridgeReply`、の形にし、Swift は運ぶだけにする。
- 結果として、`IkariXPC` に残るのは `XPCTransport`（`EngineTransport.swift` の後半）、`Identifiers`、`PeerSignature`、`BuildSigning` の約 220 行になる。

### 4.4 C ABI（macOS。`shell-ffi/include/ikari_shell.h` に足す形の案）
```c
typedef struct IkariShellHost {
  void *ctx;
  int32_t (*surrounding)(void *ctx, IkariShellBuf *out);  /* SurroundingText */
  int32_t (*selection)(void *ctx, IkariShellBuf *out);    /* SelectedText */
  uint64_t (*engine_connect)(void *ctx);
  int32_t (*engine_request)(void *ctx, uint64_t h, IkariStr bytes, uint32_t timeout_ms, IkariShellBuf *reply);
  void (*engine_post)(void *ctx, uint64_t h, IkariStr bytes);
  void (*engine_close)(void *ctx, uint64_t h);
  int32_t (*engine_pid)(void *ctx, uint64_t h);
} IkariShellHost;

IkariShell *ikari_shell_new(IkariStr config, IkariShellHost host);  /* 設定の置き場所など */
IkariShellBuf ikari_shell_dispatch(IkariShell *s, IkariStr event); /* ShellEvent -> ShellAction の列 */
void ikari_shell_engine_down(IkariShell *s, uint64_t generation, uint8_t kind); /* どのスレッドからでも */
void ikari_shell_buf_free(IkariShellBuf b);
void ikari_shell_free(IkariShell *s);

/* 設定アプリ（別のプロセス）用。§2.3 */
IkariShellBuf ikari_settings_validate(IkariStr settings);
IkariShellBuf ikari_settings_rewrite(IkariStr file, IkariStr changes);
```
- 入口は 7 個（殻 5、設定 2）。今の `ikari_composer_*` と `ikari_convert_script`（11 個）は、`shell-core` が `input` を直接使うので消す。
- 入出力の符号化は proto を第一にする（入れ子と列の多い `CandidatePanelModel` を C の構造体で書くと、解放と版の管理が手書きになるため）。M0-a の計測で目標に対して大きければ、C の構造体（平らな配列と文字列のプール）に替える。この切り替えは `shell-ffi` の中に閉じ、`shell-core` の型は変えない（2026-10-30 ユーザー決定。旧 Q5）。
- ヘッダを手書きのまま続けるか cbindgen にするかは M1-b で決める（口が 7 個なら手書きで足りる見込み）。

### 4.5 Windows の側
- TIP（Rust）は `shell-core` を crate として直接使い、ホストの口を trait（`TextAccess`、`EngineTransport`）として名前付きパイプと TSF で実装する。proto の符号化は通らない。
- `shell-core` を TIP の DLL に置くかエンジンの側に置くかは、ADR 0040 の S1 で決める。どちらにも載る。

## 5. シナリオの共通の形式
- 置き場所: `shell-core/scenarios/*.json`。Rust（`cargo test -p ikari-shell-core`）と Swift（移行の間だけの実行器）が同じファイルを読む。
- 1 つのシナリオ: 名前、設定（`settings.tsv` の行）、キー割り当て、替え玉のエンジンの台本（受け取るはずの要求の種類と主な欄、返す返事、落ちる時点）、手順の列（`ShellEvent`、その時点の周辺テキストと選択、期待する `ShellAction` の列、期待するエンジンへの送信）。
- 比べ方: `ShellAction` の列は順序つきで完全一致。エンジンへの送信は、台本に書いた欄だけを比べる（`request_id` などは比べない）。
- 移すもの: 純関数的な約 470 件のうち、イベントの列で書けるもの。関数の単体テストは Rust の単体テストに写す。`XPCEngineClient` の 50 と Bridge 系の 46 は、Rust の単体テスト（替え玉のトランスポートで落ちる・遅れるを流す）に写す。AppKit 系の約 15 は Swift に残す。設定の Model の約 100 は、検証と整形の分を Rust に、`@Observable` の分を Swift に分ける。
- JSON にする理由: Rust は serde、Swift は Codable で読め、人が差分を読める。
- **Swift 版を消す条件**: そのレーンの範囲のシナリオが Rust で全部通り、`scripts/check.sh` が `CHECK_EXIT=0` で、§6.3 の性能の比較が悪化なし。Swift 側で同じシナリオを流すのは、移す前の振る舞いを JSON に固める M1 から、そのレーンの入れ替えまで。

## 6. スパイクと基準値（M0）

### 6.1 M0-a: FFI の時間（捨てるコード。main に入れない）
1. `shell-ffi` に、`ShellEvent` の `Key` を受けて代表の動作の列（`setMarkedText` 1 つと、注釈つき 9 件の `showCandidates` 1 つ）を返すだけの関数を、proto 版と C の構造体版の 2 つ作る。
2. Swift の release ビルドの小さな実行ファイルから 1 万回呼び、符号化、FFI、復号、解放を含めた 1 回の時間の中央値と 99% を出す。周辺テキストのコールバックを 1 回はさむ版も測る。
3. 今の Swift の `handle` の時間を、検証用のログで作者の日常の打鍵から集める（Convert を含むキーと含まないキーに分ける）。検証版を入れる前にテストの入力と通常の入力を確かめ、入れ替えたら開いているアプリを再起動する。
4. 結果を本 ADR に追記する。比べる目標は ADR 0004 §12（Space から 1 ページ目まで p99 16ms 以下、XPC 込み）。**許す時間はユーザーが計測を見て決める**（2026-10-30 ユーザー決定）。

### 6.2 M0-b: macOS の入口を Rust で作れるか（捨てるコード）
- 作るもの: Rust だけの入力メソッドのバンドル（`objc2` で `IMKInputController` の子クラスを実行時に登録し、`IMKServer` を立てる）。キーを受けて、決まった文字列（例: 「あ」）を `insertText` で確定するだけ。署名と配置は今の `macos/scripts/` の手順を写す。
- 確かめること: (1) TextEdit、Safari、VS Code でキーが届き、確定できる。(2) `Info.plist` の `InputMethodServerControllerClass` で、Rust が登録したクラスを IMK が見つける。(3) `NSEvent` の keyCode と修飾が取れ、マークした文字列（`setMarkedText`）が出せる。(4) cargo だけでバンドルを作り、Apple Development の署名で入力ソースに出せる。(5) panic を `catch_unwind` で止めたとき、IMK のプロセスが落ちない。
- 見立て（**未検証**）: `objc2` で Objective-C のクラスを定義する仕組みはある（`define_class!`）。InputMethodKit の生成済みの binding の crate があるかは確かめていない（手元の cargo の cache には `objc2` 本体だけ）。なければ、使う 5〜10 個のメソッドを手で宣言する。山は (2) と、IMK が呼ぶメソッドの名前と引数の型（`handleEvent:client:` など）を正しく合わせられるか。作れる見込みは高いと見るが、前例を確かめていないので推測に留まる。
- 成ったときの分岐: 入口（キー、IMK の呼び出し、XPC のトランスポート、Bridge の待ち受け）を Rust にし、IME のプロセスの中の Swift ↔ Rust の境界をなくす。候補窓とメニューをどうするかは §11 の Q2。
- だめだったときの分岐: §4.4 の C ABI と Swift の薄い入口で続ける。`shell-core` の設計は変わらないので、M1 以降は M0-b の結果を待たずに進める。
- 打ち切り: (1)〜(3) が通らなければ、だめと判断して Swift の入口にする（時間の上限はリードが決める）。
- 入れ替えるのは作者の実機ではなく、別の入力ソースとして入れる。smoke は使わない。

### 6.3 M0-c: 性能の基準値と、レーンごとの比較（2026-10-30 ユーザーの要件）
**測るもの**（移行の前、どのレーンも入れ替えていない Swift 版で取る）
| # | 指標 | 測り方 | 今の道具 |
| --- | --- | --- | --- |
| P1 | キー 1 回（Convert を含まない）の殻の時間。p50、p99、max | 共通のシナリオ（§5）のキー列を、即答の替え玉のエンジンで再生し、`handle`（移行後は `dispatch`。macOS では FFI と符号化を含む）の 1 回ごとの時間を集める。シナリオ全部を 100 回くり返す | なし（新しく作る） |
| P2 | 変換の往復（Space から 1 ページ目まで、XPC 込み）。p50、p99、max | smoke の latency（`IkariSmoke/main.swift:417-425`）。LM あり・なしの両方 | smoke（ユーザーの了解が要る） |
| P3 | 殻の起動（`shell-core` の初期化: 設定、キー割り当て、ショートコードの表の読み込み、エンジンへの hello まで） | P1 の道具で、初期化だけを 100 回くり返す（hello は替え玉）。実機の IME のプロセスの起動は、検証用のログの時刻で作者の実機から 1 回ずつ取る（参考値。比較の条件にはしない） | なし |
| P4 | メモリ | P1 の道具の最大 RSS（`eval/src/rss.rs` を使う）。実機の IME のプロセスの RSS を `ps` で読む（作者が普段どおり使った後。読むだけ。参考値） | `eval/src/rss.rs` |
| P5 | エンジンとコアの性能 | 今の `scripts/check.sh --perf`（`ikari-core` の perf、`segment_perf`） | あり |

- P1 と P3 は、移行前（Swift）と移行後（Rust）で**同じシナリオを同じ手順で**流すので、差がそのまま移行の影響になる。Swift 版の測り手は、移行の間だけの小さな実行ファイル（`macos/Sources/IkariShellBench/`、M5 で消す）にする。シナリオが JSON になるのは M1-a なので、**P1・P3 の基準値は M1-a の終わり（どのレーンも入れ替える前）に取る**。P2・P4・P5 は M0 で取る。
- ADR 0004 §12 との関係: §12 の「p99 16ms 以下」は**絶対の上限**で、今までどおり smoke が FAIL にする。本節の比較は**基準値からの相対**で、上限の内側でも悪化を止めるためのもの。両方を満たすことを統合の条件にする。
- 基準値の置き場所: `eval/reports/shell-perf/baseline.json`（計測した SHA、機械、OS の版、各指標の値）と、索引 `eval/reports/README.md` への 1 行。

**比べる時点と統合の条件**
- L-B、L-C、L-E の統合（作者の実機への入れ替え）の前と、M5 の前に、同じ計測（P1〜P5）を流す。L-A1・L-A2 は入れ替えを伴わないので、Rust の P1 を参考に流すだけ。
- 条件: 各指標が「基準値 + 許す幅」以内。**許す幅は、基準値（とくに測るたびのばらつき）を見てユーザーが決める**（本 ADR では決めない。§11 の Q3）。ばらつきを抑えるため、P1・P3 は 5 回流した中央値で比べる。
- 超えたら統合しない。原因を直すか、許す幅を変えるかはユーザーに聞く。
- P2 は smoke を使うので、流すたびにユーザーの了解を取る。

**道具**
- `cargo xtask perf`（ADR 0043 §3）: `perf shell`（P1・P3・P4。Rust は `shell-core` の bench と FFI 越しの両方。macOS では Swift 版の測り手も呼ぶ）、`perf engine`（P5。今の `run_perf` を包む）、`perf smoke`（P2。`--allow-smoke` を付けたときだけ流す）、`perf compare --baseline <file>`（表を出し、許す幅を超えたら 0 以外で終わる。許す幅は `eval/reports/shell-perf/tolerance.toml`。ユーザーが決めるまでは表だけを出し、終了コードは 0）。
- 計測の道具は、ほかのレーンの入れ替えより先に要るので、L-P（§7.2）として早く作る。

## 7. 段階、レーン、契約

### 7.1 依存と順序
```text
M0-a FFI の計測 ──┐（M1-b の符号化を決める）
M0-b Rust の入口 ─┤（L-E の形を決める）
M0-c 基準値（P2・P4・P5）
                  │
M1-a 契約（Rust の型、シナリオ、xtask の骨組み）
   │  └─ P1・P3 の基準値（Swift 版）
   ├─ L-P 計測の道具 ──────────────┐（L-B・L-C の入れ替えより先）
   ├─ L-A1 状態機械の周辺 ─────────┤
   ├─ L-B エンジン通信・Bridge ────┤（終わったら入れ替え）
   ├─ L-C 設定 ────────────────────┤（終わったら入れ替え）
   └─ L-D cargo xtask（ADR 0043）  │
M1-b 契約（C ABI。M0-a の後）──────┤
                                   └─ L-A2 状態機械の本体と殻の判断 ─ L-E macOS の入口の付け替え ─ M5 片付け
```
- M1-a は M0 を待たない（`shell-core` の型は FFI の形に依存しない）。M1-b（ヘッダ）は M0-a の後。
- 「早く Swift の行数を減らす」ため、L-B と L-C は**終わった時点で Swift を消して入れ替える**。状態機械（L-A）は、本体と周辺を分けて入れ替えると仮の口が多くなるので、L-A2 の後の L-E でまとめて入れ替える。
- L-D（xtask）はどのレーンとも独立。ただし `xtask` の骨組みは M1-a に入れ、L-D と L-P が別のモジュールを持つ。

### 7.2 レーン
| レーン | 持つファイル（ほかのレーンは触らない） | 中身 | 入れ替えと Swift を消す時点 | モデル |
| --- | --- | --- | --- | --- |
| L-P 計測 | `xtask/src/perf/`、`shell-core/benches/`、`macos/Sources/IkariShellBench/`、`eval/reports/shell-perf/` | §6.3 の道具と基準値 | 基準値を取ったら完了。`IkariShellBench` は M5 で消す | Sonnet |
| L-A1 状態機械の周辺 | `shell-core/src/` の `keymap`、`small_letters`、`menu`、`panel`、`timeouts`、`english/`、`shortcode/`、`postal/`、`diag/` とそのテスト | `Keymap`、`SmallLetters`、`InputMenuModel`、`PanelPlacement`、`EngineTimeouts`・`LmBudget`、`EnglishMode`、`ShortcodeTable`・`ShortcodeMachine`・`ShortcodeEnglish`、`EnglishPostal`、`DiagEpisode` | L-E と一緒 | Sonnet |
| L-A2 状態機械の本体と殻の判断 | `shell-core/src/machine/`、`shell-core/src/controller/` | `InputStateMachine` と §0.1 の殻の判断（`selectMode` ほか、`AnchorMemory`、`isAvailable`、`nextSessionID`、メニューの命令、タイマー）。L-A1 の公開の型は M1-a の stub で書き始め、L-A1 のマージの後で通す | L-E で入れ替え | Opus（同期の待ちと状態の持ち回し） |
| L-B エンジン通信・Bridge | `shell-core/src/engine/`、`shell-core/src/bridge/`、`macos/Sources/IkariXPC/`、`macos/Sources/IkariBridge/`、`macos/Sources/IkariEngineService/EngineGate.swift` | §4.3 | 終わった時点で、Swift の `EngineClient` の実装を「`EngineLink` を呼ぶ薄い型」（仮の口）に替え、`XPCEngineClient`・`EnvelopeCodec` などを消す | Opus（再接続、スレッド） |
| L-C 設定 | `shell-core/src/settings/`、`shell-ffi/src/settings.rs`、`macos/Sources/IkariSettings/Model/`、`macos/Sources/IkariShellCore/Settings*.swift` | `Settings`・`SettingsFile`、`SettingsStore` の `reloadIfFileChanged`・`update(set:reset:)`、設定アプリの検証と整形（§2.3） | 終わった時点で、設定アプリと殻の両方が Rust の検証と書き換えを使い、Swift の `Settings.swift` を消す（殻の側は Swift の値を Rust の結果から作る仮の口） | Sonnet |
| L-D `cargo xtask` | `xtask/src/` の `check`、`dict`、`data`、`eval`、`.cargo/config.toml`、`scripts/check.sh`（包む部分だけ） | ADR 0043 | ADR 0043 の段ごと | Sonnet |
| L-E macOS の入口の付け替え | `shell-ffi/src/shell.rs`、`macos/Sources/IkariIME/`、`macos/Tests/` の実行器 | `ikari_shell_dispatch`、ホストの口の実装、`run(actions)` を `ShellAction` の実行に替える。M0-b が成っていれば、入口そのものを Rust にする | L-A2 の後。終わったら `IkariShellCore`、仮の口、`ikari_composer_*` を消す | Opus |

- 仮の口（L-B、L-C）は、Swift の状態機械が L-E まで残るためのもので、それぞれ 100 行程度を見込み、L-E で消す。仮の口を置いてでも先に入れ替えるのは、L-B（約 1,100 行）と L-C（約 400 行と設定アプリの検証）の Swift を早く消せ、作者の日常利用で早く確かめられるため。
- 同じファイルを 2 つのレーンが持たない。`shell-core/src/lib.rs`、`types.rs`、`Cargo.toml`、`shell.proto`、`xtask/src/main.rs` は M1-a で固め、レーンは変えない（足りなければリードに戻す）。

### 7.3 契約コミット
**M1-a（M0 を待たない）**
1. `shell-core` の crate（`input`、`proto` に依存）と、§7.2 の各モジュールの空のファイル。`lib.rs` と `types.rs` で公開の型を固める: `ShellEvent`、`ShellAction`、`KeyInput`・`Modifiers`・`KeyKind`、`MarkedText`、`CandidatePanelModel`、`Utf16Len`・`Utf16Range`、`SurroundingText`・`SelectedText`、`MenuModel`・`MenuCommand`、`TimerId`、`EngineError`（4 種）、`SettingsSnapshot`・`SettingsIssue`、`ConnId`。
2. trait: `TextAccess`（周辺テキストと選択）、`EngineTransport`（`connect`・`request`・`post`・`close`・`pid`）、`Clock`。
3. 公開の関数のシグネチャと stub: `Shell::new`、`Shell::dispatch(&mut self, ShellEvent, &mut dyn TextAccess) -> Vec<ShellAction>`、`EngineLink` の公開の関数と `notify_down`、`settings::validate`・`settings::rewrite`。stub は空の結果か `Err` を返し、panic しない。
4. `proto/ikari/shell/v1/shell.proto`（`ShellEvent`、`ShellAction`、上の型。欄の名前と番号を固める。Swift の生成コードをコミット）。
5. シナリオの JSON の形（serde の型と `shell-core/scenarios/README.md`）、Swift の実行器、Rust の実行器（stub に対しては `#[ignore]`）。既存の Swift のテストのうちイベントの列で書けるものを JSON に移し、**Swift 版が全部通ること**を確かめる（ここで今の振る舞いを固める）。
6. `xtask` の crate の骨組み（`check`、`dict`、`data`、`eval`、`perf` の下位コマンドの入口だけ。ADR 0043 §3）。

**M1-b（M0-a の後）**
7. `ikari_shell.h` の追加分（§4.4）と、`shell-ffi` の stub。符号化（proto か C の構造体か）はここで固める。

### 7.4 段と完了の条件
| 段 | 完了の条件 |
| --- | --- |
| M0-a、M0-b、M0-c | 結果を本 ADR に追記。M0-a は許す時間をユーザーが決める。M0-b は §6.2 の成否と分岐をリードが決める。M0-c は P2・P4・P5 の基準値を記録 |
| M1-a、M1-b | Swift 版が全シナリオを通る。`scripts/check.sh` が `CHECK_EXIT=0`。P1・P3 の基準値を記録 |
| L-P | `cargo xtask perf` で P1〜P5 が流れ、基準値のファイルができている |
| L-A1、L-A2 | そのレーンのシナリオと単体テストが Rust で通る。`check.sh` |
| L-B、L-C | 上に加え、入れ替えて Swift を消し、§6.3 の比較が悪化なし（許す幅が決まる前は、表をユーザーに見せて了解を取る） |
| L-E | 全シナリオが Rust で通る。`check.sh`。§6.3 の比較が悪化なし。ADR 0004 §12 の上限を満たす |
| M5 片付け | `IkariShellCore`、仮の口、`ikari_composer_*`、Swift の実行器、`IkariShellBench` を消す。`docs/architecture.md` の依存の向き（`input`、`proto` ← `shell-core` ← `shell-ffi`）を更新。最後の §6.3 の比較 |

- 作者の実機への入れ替えは、L-B、L-C、L-E が終わるたびに行う（インストールは聞かずに行ってよい。smoke は了解を取る）。戻すときは前の版を入れ直す（切り替えの設定は持たない）。入れ替えたら開いているアプリを再起動する。

## 8. 移行の間の運用
- 新機能の凍結はしない。ただし、**移す範囲の新機能とバグ修正は、まだ Swift にあるものでも、まず共通のシナリオを足す**（移すときにシナリオが Rust を縛る）。すでに Rust に移したものは Rust にだけ入れる。
- 移行の PR はレーンの段ごとに 1 つ（1 ブランチ 1 タスク）。レーンのブランチはリードが統合用のブランチにまとめる。
- 本 ADR の後に、入口へ OS の API を呼ばない分岐を足す変更が出たら、§2.1-2 の規則で `shell-core` に移す。

## 9. 危険な箇所と対策
| 箇所 | 危険 | 対策 |
| --- | --- | --- |
| 文節のモードの `layoutRev` と、区切り直しの後の選択（ADR 0006 §16） | 古い返事を当てる。選択がずれる | M1-a で重点的に JSON にし、台本で `layoutRev` の不一致も流す |
| 再変換の `TextRange` と `restore` の範囲 | UTF-16 とバイト数の混同。サロゲートペア、異体字セレクタ、結合文字 | §4.2 の newtype。それらを含むシナリオを足す |
| `passThrough` を含む列の順序 | 文字が消えるか二重になる | 列の完全一致で比べる。secure と `resolveStrayMarkedText` のシナリオ |
| 落ちた通知と再接続（§4.3） | 主スレッドの待ちと XPC のキューの競合。世代の取り違え | 落ちた通知の口だけをどのスレッドからでも呼べるようにし、世代で古い通知を捨てる。L-B の単体テストで、待ちの最中に落ちる・遅れて返事が来るを流す |
| 同期の待ち（Convert、Page） | Rust の中の待ちが主スレッドを今より長く止める | 待つのはホストの `request` の中だけ。上限は `EngineTimeouts` を写す。§6.3 の P2 で比べる |
| 性能の悪化 | 符号化と FFI の分、作り直しの分で遅くなる・重くなる | §6.3 の基準値との比較を統合の条件にする |
| panic | macOS では入力が止まり、Windows ではアプリが落ちる | §4.2 |
| 仮の口（L-B、L-C） | 残り続ける | L-E の完了で消す（§7.4） |
| 作者の日常の入力が止まる | 入れ替えの後の不具合 | 前の版を入れ直す。入れ替えたら開いているアプリを再起動する |

## 10. ADR 0040 との関係
- ADR 0040 §3.2 の状態機械は、本 ADR の共有の `shell-core` に確定する（0040 の改訂で記録）。
- Windows の TIP は、`shell-core` の M1-a の型と trait ができれば、最小の口（読み、Space、確定）で W1 を作れる。W0（コアの Windows のビルド）は本 ADR と並べてよい（触るファイルが重ならない）が、本 ADR の移行を先にする（#138 が最優先）。

## 11. ユーザーに聞くこと
- **Q1. 設定アプリの Model をどこまで Rust にするか**。推す: §2.3 のとおり、検証、整形、ファイルの読み書き、既定値、管理の要求の組み立てまで。`@Observable` の画面の状態は Swift に残す。理由: 画面の状態は UI の一部（ADR 0042 §1-4）で、Rust に置くと値の写しが境界を何度もまたぐ。
- **Q2. M0-b が成ったとき、候補窓とメニューも Rust（`objc2-app-kit`）で描くか**。推す: M0-b の結果を見てから決める。既定は「Swift の UI を残す」（ADR 0042 §1-4。きれいさを優先してよい所。境界は描く内容を渡す 1 本だけになる）。
- **Q3. 性能の許す幅（§6.3）**。推す: M0-c と M1-a の基準値（5 回のばらつき）を見せた後に、指標ごとに決めてもらう。決まるまでは、統合のたびに比較の表を見せて了解を取る。

### 11.1 決まったこと（2026-10-30 ユーザー決定）
- 範囲（旧 Q1）: 「可能な限り Rust」。`IkariInputController`、`AppContext`、`BridgeServer`、`SettingsStore` の OS 非依存の判断も移す（§2.2）。
- 移行の間の凍結（旧 Q2）、Swift 版と Rust 版の切り替えの設定（旧 Q3）、日常利用の日数（旧 Q4）: 持たない。守るのは共通のシナリオと性能の基準値（§2.1-4、§6.3、§8）。
- FFI の形と許す時間（旧 Q5）: 推した案どおり。M0-a の計測の後に形を決め、proto を第一にする。許す時間は計測を見てユーザーが決める（§4.4、§6.1）。
- 性能: 移行の前に基準値を取り、レーンの統合ごとに比べる。悪化したら統合しない（§6.3。許す幅は Q3）。
- 方針は ADR 0042。スクリプトは `cargo xtask`（[ADR 0043](0043-xtask.md)）。移行は全タスクで最優先（#138）。

## 参考
- [ADR 0004](0004-phase2-macos.md) §3.3、§4.3、§5.1、§6.3、§7.5、§12
- [ADR 0005](0005-macos-settings-and-indicator.md) §8.3
- [ADR 0006](0006-segment-editing.md) §8.2、§16
- [ADR 0026](0026-english-mode-and-key-stats.md)、[ADR 0027](0027-emoji-input.md)、[ADR 0032](0032-postal-code.md)、[ADR 0034](0034-diagnostic-log.md)
- [ADR 0043](0043-xtask.md)（`cargo xtask`）
- ADR 0040（Windows。PR #126）、ADR 0042（技術選定の方針。PR #137）。どちらも main にはまだない
- タスク: #138、#81、#128
