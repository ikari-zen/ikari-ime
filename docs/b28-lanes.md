# B28 実装のレーン分割（ADR 0015 §8.1）

契約コミットのあと、3 レーンを worktree ごとに並行で実装する。レーン間で共有する型は下の「契約」にあり、**レーンは契約を変えない**（変えたくなったらリードに報告する）。

## 契約（このコミットで入れたもの）
| 契約 | 場所 | 約束 |
| --- | --- | --- |
| `BeginSession.share_context`（bool、番号 4） | `proto/ikari/engine/v1/engine.proto`、`macos/Sources/IkariProto/engine.pb.swift`（`macos/scripts/gen-proto.sh` で再生成） | false（省略）= 貸し借りしない（今の挙動）。殻は除外でないアプリで true を送る。エンジンは Bundle ID を保存しない |
| `SessionInfo.shareContext`（既定 false） | `macos/Sources/IkariShellCore/EngineClient.swift`、`XPCEngineClient.beginMessage` で proto に載せる | レーン B が値を決めて渡す |
| `Context.left_borrowed`（既定 false）、`with_left_borrowed()` | `core/src/context.rs` | true のとき `LearningRanker` は学習の文脈の特徴量を空にする（引き当ても記録も「文脈なし」）。話題、LM、連接、文書内キャッシュは通常どおり使う |
| シナリオの `borrowed` 列（省略可、既定 false） | `eval/src/scenario.rs` | `Context::left_borrowed` に渡る |
| `ikari_engine::context_limits`（`PAUSED_SESSION_TTL` 30 分、`HISTORY_MAX_CHARS` 2000、`RECENT_COMMITS_TTL` 10 分、`RECENT_COMMITS_MAX_CHARS` 2000、`BORROW_WINDOW` 2 分、`ContextLimits`） | `engine/src/context_limits.rs` | 値は ADR 0015 §10。使うのはレーン A。テストで短くしたいときは `ContextLimits` を使う |

## レーン A: エンジン（D1、D4、D5、D6 のエンジン側）
- 所有: `engine/src/session.rs`、`engine/src/lib.rs`（`HISTORY_LIMIT` の置き換え、`Inner` に「最近の確定」を足す部分だけ）、`engine/src/context_limits.rs`（値の使い方。公開する名前は変えない）、`engine/src/tests.rs` と `engine/src/tests/`
- 触ってよい契約: `BeginSession.share_context` を読む。`Context::with_left_borrowed(true)` を、ほかのセッションから借りたときだけ付ける
- 触らない: core、殻、proto
- やること: 休止と再開（`paused_at`、janitor）、履歴を文字数で、「最近の確定」と X5 の借り方、`share_context=false` のセッションは入れない・貸さない。再変換は借りない
- テスト: `engine/src/tests.rs`、`engine/src/tests/`（`begin` のヘルパーは今 `share_context: false`。共有側のテスト用ヘルパーを足してよい）

## レーン B: 殻（D2、D6 の殻側）
- 所有: `macos/Sources/IkariIME/ClientAccess.swift`、`macos/Sources/IkariShellCore/Settings.swift`、`SettingsFile.swift`、`macos/Sources/IkariIME/SettingsStore.swift`、`macos/Sources/IkariSettings/`（設定アプリ）、`IkariInputController.swift` の `BeginSession` を送る部分、対応する `macos/Tests/`
- 触ってよい契約: `SessionInfo.shareContext` に値を入れる（既定は除外でないアプリで true）
- 触らない: `engine.proto`、`engine.pb.swift`、`EngineClient.swift` の `SessionInfo` の形（足すだけでも報告する）、Rust 全般
- やること: 左が空で `client.length() == 0` なら `.unavailable`（D2）、設定 `context.shared_exclude`（Bundle ID の一覧。既定はパスワード管理とターミナル。ADR 0015 §3.5）と設定アプリの欄、`share_context` の送信
- テスト: `macos/Tests/IkariShellCoreTests/`、`IkariIMETests/`、`IkariSettingsTests/`（swift test は macOS でだけ流せる）

## レーン C: core（学習の特徴量）
- 所有: `core/src/ranker/learning.rs`、`core/src/user/features.rs`（と、その周辺の `core/src/user/`）、`eval/src/scenario.rs` のテストと、必要なら `eval/sets/` のシナリオ
- 触ってよい契約: `Context.left_borrowed` を読む
- 触らない: `core/src/context.rs` のフィールド（ドキュメントコメントの約束を実装で守る）、engine、殻、proto
- やること: `left_borrowed` のとき、引き当てと記録の両方で文脈の特徴量を空にする。borrowed 列を使ったシナリオ（借りた文脈では学習が効かず、通常の文脈では効く）を足す
- テスト: `core/src/ranker/learning.rs` と `core/src/user/` の単体テスト、`borrowed` を使うシナリオ

## 衝突しやすい点
- `engine/src/lib.rs` は A だけが触る。`core/src/context.rs` は誰も触らない。
- `proto` を変える必要が出たら、レーンで直さずリードに報告する（生成物が衝突する）。
- 統合はリード: `scripts/check.sh`、`eval/scripts/snapshot.sh`（既存のセットが一致）、ADR 0015 付録 A の ext-stale と ext-mixlong の再測定。
