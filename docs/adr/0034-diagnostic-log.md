# ADR 0034: 診断ログ（直し方、不具合の手がかり、機能の利用、性能を、文字なしで engine.log に常時ためる）

- 状態: 承認（2026-10-25。§9 の 3 点はすべて推奨どおりにユーザーが決定した）
- 日付: 2026-10-25
- 対象: issue #295。`engine/src/log.rs`、`engine/src/dispatch.rs`、`proto/ikari/engine/v1/engine.proto`、`macos/Sources/IkariShellCore/`（`EngineClient`、`InputStateMachine`）、`macos/Sources/IkariIME/IkariInputController.swift`、`macos/Sources/IkariXPC/XPCEngineClient.swift`、集計スクリプト（新設）
- 前提: main = d8b20f7。ADR 0004 §7.5・§7.6、ADR 0021 L0・L2、spec の NFR-18・NFR-19、`engine/src/session.rs` の「client_bundle_id は保存もログもしない」
- この ADR ではコードを変えない。事実はコードの確認による。確かめていないことは「推測」「要確認」と書く
- 承認されたら ADR 0004 §7.6（1MB で回し 3 世代）を §3 の形に一部改める（本文は書き換えない）

---

## 0. 現状（分かったこと）

| 項目 | 今の形 | 根拠 |
|------|--------|------|
| 書き先 | `<UserDir>/logs/engine.log`。1 行 = `{ミリ秒} {kind} {所要}us lm={LM の状態} err={種類} {注記}` | `engine/src/log.rs` |
| 書き方 | **同期**。`Engine::handle` が要求を処理したあと、返事を返す前に `record_note` を呼ぶ。`Mutex` の中で `File::write_all`（バッファなし。1 行ごとに write）。回すとき（rename ×3）もその場で行う | `engine/src/lib.rs` の `handle`、`log.rs` の `record_note`・`rotate` |
| 回し方 | 1MB で回し、`.1`〜`.3` の 3 世代（合計 約 4MB）。圧縮なし。issue の記述では数日で消える | `log.rs` の `MAX_LOG_BYTES`、`KEPT_GENERATIONS` |
| 今ある kind | 要求ごと（`hello` `begin_session` `prefetch` `convert` `page` `commit` `commit_text` `cancel` `segment_page` `resize_segment` `commit_segments` `reconvert` と管理の要求）、`prefetch_done`、`ctx_change`、`reload`、`save` | `dispatch.rs` の `request_kind`、`workers.rs`、`convert.rs`、`data.rs` |
| 今ある注記 | `convert`: `cache= lm_applied= miss=`（L0）、`ctx_rev=`。文節の要求: `segments= dir=`。`ctx_change`: `rev= prev_rev= left= right= avail= dleft= dright= req= reading= lext= tail=`（確定のあとは末尾に `since_commit_ms= commit_chars=`。数と固定の語） | `cache.rs`、`segments.rs`、`ctx_trace.rs` |
| 殻のログ | `AppContext.log`（os の `Logger`、category = shell）。設定、ブリッジ、絵文字の表の事象だけ。ファイルには書かない | `macos/Sources/IkariIME/AppContext.swift` |
| 殻からの片道のメッセージ | `Prefetch`、`Commit`、`CommitText`、`CommitSegments`、`Cancel` は返事を待たない（ADR 0004 §7.5） | `engine.proto`、`EngineClient.swift` |
| 圧縮の crate | ワークスペースに圧縮の crate はない（`Cargo.lock` に flate2 / xz / zstd なし） | `Cargo.lock` |
| エンジンの Sandbox | app-sandbox あり。`~/Library/Application Support/ikari-ime/` の読み書きだけ一時例外 | `macos/scripts/templates/engine.entitlements` |

**エンジンからは見えないもの**: Space で候補を進めた回数（ページの中の移動は殻だけで行う）、文節のフォーカス移動、2 文節以上での数字キーの選択（proto のコメント）、Esc で読みに戻したこと（`Cancel` は送るが、読みに戻したのか消したのかは分からない）、打鍵数と時間、英語・日本語のモードの切り替え、ショートコードと小さい文字（英語のモードは殻だけで完結）、`commitComposition` / `deactivateServer` の直前の状態、候補ウィンドウの位置の取り方、XPC の失敗。issue の範囲 1〜4 の大半は殻にしかない。

---

## 1. 決定の要約（案）

1. **書き先は engine.log に 1 本化する。** 殻の事象は、新しい片道のメッセージ `DiagEvent` でエンジンへ送り、エンジンが許可リストで確かめてから engine.log に書く（§2）。殻は自分でファイルに書かない。
2. **書き込みを別スレッドにする。** `EngineLog` は有界のキュー（`std::sync::mpsc::sync_channel`）に 1 行を入れるだけにし、専用のスレッド `ikari-log` がバッファつきで書く。満杯なら捨てて数える。新しい crate は要らない（§3.1）。
3. **圧縮は「回した世代を gzip にする」だけ。** 日ごとの集計は作らない（集計スクリプトがいつでも作れる）。gzip は `flate2`（既定の純 Rust の実装）で、ログのスレッドで行う（§3.2）。
4. **記録するもの**は §4 の表。殻の事象は「合成 1 回（idle から idle まで）に 1 行」の `sh_episode` を中心にし、打鍵ごとには送らない。エンジンが知っていることは既存の行の注記に足す。
5. **集計**は `scripts/diag-report.py`（Python の標準ライブラリだけ）。`.log` と `.log.gz` を読み、直し方の分布、1 位のまま確定した率、状態が終わった理由の内訳、機能の利用、性能、ログの健全性を表で出す（§5）。
6. **段階**は 書き込みの非同期化と圧縮 → 契約 → エンジンの事象 → 殻の事象 → 集計 の 5 issue（§6）。Bundle ID は了解が出てからの別 issue（§7）。

---

## 2. どこに書くか

### 2.1 比べた案

| 案 | 中身 | 良い点 | 悪い点 |
|----|------|--------|--------|
| A | エンジンが見える事象だけを engine.log に書く | proto を変えない | issue の範囲 1〜4 の大半（§0 の「見えないもの」）が取れない。目的を満たさない |
| **B（推奨）** | 殻が片道のメッセージ `DiagEvent` をエンジンへ送り、エンジンが engine.log に書く | 書き先、時刻、回し方、圧縮、保持が 1 か所。集計は 1 本のファイルを時刻順に読めばよい。NFR-18・19 の確認（secure のセッションは捨てる、許可リストにない語は捨てる）をエンジンの 1 か所で行える | proto を 1 つ足す。エンジンが落ちている間の殻の事象は失われる（§2.3 で数だけ持ち越す） |
| C | 殻が自分のファイル（`logs/shell.log`）に書く | エンジンが落ちていても書ける | 書き込みのスレッド、回し方、圧縮、保持を Swift でもう 1 つ作る（同じものが 2 つ）。2 つのファイルを時刻で突き合わせる集計になる。NFR-19 の確認も 2 か所 |
| C' | 殻は os の `Logger`（統合ログ）に書く | 実装が最小 | 保持期間を決められず、圧縮して長くためる目的に合わない。`log show` での取り出しは集計に向かない。今の開発用の使い方（`AppContext.log`）はそのまま残す |

**B を選ぶ理由**: 殻とエンジンはもともと 1 対 1 で、殻の操作はすでに `Commit` などの片道のメッセージでエンジンへ流れている。同じ経路に 1 種類足すのが、新しい仕組み（Swift のファイル書き込みとローテーション）を作るより小さい。失われるのはエンジンが落ちている間だけで、その間は殻がフォールバックの候補を出している（ADR 0004 §7.5）ので、件数を持ち越せば集計には足りる。

### 2.2 proto の最小の形（契約）

```proto
// Envelope の oneof に足す（33 は空き番号）
DiagEvent diag = 33;   // 殻 → エンジン。返事は Ack（殻は待たない）。secure のセッションでは送らない

message DiagEvent {
  uint64 session_id = 1;
  string kind = 2;                 // 許可リストの kind（sh_episode など）
  repeated DiagNum nums = 3;       // 数の値
  repeated DiagTag tags = 4;       // 固定の語の値
}
message DiagNum { string key = 1; sint64 value = 2; }
message DiagTag { string key = 1; string value = 2; }
```

- **kind・key・タグの値は文字列にし、エンジンの許可リスト（新設 `engine/src/diag_schema.rs`）で確かめる。** 許可リストは kind ごとに「使える数のキー」「使えるタグのキーと、そのタグが取れる値の一覧」を持つ。kind が許可リストにないもの、キーが許可リストにないもの、タグの値が一覧にないもの、文字列が 32 バイトを超えるものは、その値（または行）を捨てて `diag_reject` を数える。**タグの値に一覧の外の文字列が入る経路がないので、ユーザーの文字（英語のモードのショートコード名などの ASCII も含む）は構造的に書けない**（NFR-19）。
- 列挙型（enum）で kind とキーを表す案は、型で守れる利点があるが、事象を 1 つ足すたびに proto と `engine.pb.swift` の再生成が要る。許可リストはどちらの案でも NFR-19 の最後の守りとして要るので、文字列 + 許可リストを選ぶ。
- エンジンは `session_id` のセッションが secure なら捨てる（殻は secure では送らないので、二重の守り。NFR-18）。セッションがなければ捨てる（`UNKNOWN_SESSION` の err の行は書かない。ログを汚さない）。
- 行の形は今と同じ: `{ミリ秒} {kind} 0us lm={状態} err=- k=v k=v …`。時刻はエンジンが受けた時刻。殻の側の時間（`dur_ms` など）は値として送る。
- Swift の `EngineClient` には `func diag(_ event: DiagEventValue)`（返事を待たない）を足し、既定の実装は何もしない。

### 2.3 エンジンが落ちている間

- `XPCEngineClient` は、エンジンなしの間に起きた失敗（タイムアウト、中断、無効化、フォールバックの候補を出した回数、連続して落ちたので止めている間）を**数だけ**持ち、次に `Hello` が通ったあとで `sh_engine` の 1 行として送る。その間の `sh_episode` は送らない（貯めない。メモリと実装を増やさないため）。
- `DiagEvent` を送るためにエンジンを起こさない。エンジンなしの状態と、連続して落ちたので止めている 5 分間は、`diag` は何もしない。

---

## 3. 入力の経路で遅れを出さない書き方

### 3.1 書き込みの非同期化（新しい crate なし）

今は `Engine::handle` の中で同期で write している（§0）。`Convert` は殻が IMK の主スレッドで待つ（ADR 0004 §7.5）ので、write とローテーションの時間はそのまま入力の遅れに入る。これを次の形にする。

- `EngineLog` は `SyncSender<LogLine>` を持つ。`LogLine` は `{ millis, kind, micros, lm, err, note }` で、**時刻は呼んだ側で取る**（キューで待っても時刻がずれない）。文字列の組み立て（1 行にする部分）は書き込みのスレッドへ移す。
- 呼ぶ側は `try_send` だけ。満杯（容量は 8192 行の案）なら捨てて `AtomicU64` の `dropped` を足す。書き込みのスレッドは、次に書けたときに `log_dropped n=N` の行を書く。
- 書き込みのスレッド `ikari-log` は `BufWriter`（64KB）で書き、キューが空になったら（`recv_timeout(1 秒)` が切れたら）flush する。ローテーションと圧縮（§3.2）もこのスレッドで行う。
- 終了: `Engine::drop` は今もワーカーを join している（`lib.rs`）。`EngineLog` の送り手を落としてから `ikari-log` を join し、残りを flush する。`Shutdown` のあとプロセスが終わるまでに書き切る。
- テスト: 今のテストは `engine.log` を直接読む（`tests.rs`、`tests/convert.rs` など）。`EngineLog::flush_for_test()`（送ったものを書き終えるまで待つ）を足し、読む前に呼ぶ。`EngineLog::disabled()` は今のまま。
- 確かめ方: `scripts/check.sh --real --perf` の warm p99 を、変える前と後で比べる（issue の完了の条件）。

`crossbeam-channel` などの crate は使わない。書き手は 1 本、量は 1 秒に数行〜数十行で、標準の `mpsc` で足りる。

### 3.2 圧縮とローテーション

- **回すきっかけ**: `engine.log` が 4MB を超えたとき、または**ローカルの日付が変わって最初の行を書くとき**。日付で回すのは、集計を日ごとに切りやすく、保持を日数で消しやすいため。今の名前 `engine.log`（書き中）は変えない（テストと手作業の確認がこの名前を読む）。
- **回した世代**: `engine.log` を `engine-YYYYMMDD-HHMMSS.log`（回した時刻）に rename し、すぐに gzip にして `engine-YYYYMMDD-HHMMSS.log.gz` を作り、元を消す。gzip は `flate2`（既定の `miniz_oxide`。純 Rust、C のビルドなし）。依存は `flate2`、`miniz_oxide`、`adler2`、`crc32fast`、`cfg-if` 程度が増える（推測。実装のときに `cargo tree` で確かめる）。
- **保持**: 日数と合計の容量の両方で消す。案は **180 日、合計 100MB**（§9 でユーザーに聞く）。回したときに古い順に消す。
- **起動時の後始末**: 前の起動で gzip の途中で終わった `engine-*.log`（`.gz` がない）があれば、ログのスレッドが起動のあとで gzip にする。
- **日付**: ローカルの日付は `jiff`（ワークスペースにすでにある）で取る案。時刻の列はこれまでどおり UTC のミリ秒。
- **量の見積もり（推測）**: 今は 1MB × 4 が数日で埋まる → 1 日あたり 1MB 前後。診断の行を足して 2MB/日 とし、テキストのログは gzip で 1/8〜1/10 になると見て、**1 日 200〜250KB、180 日で 40〜45MB**。日ごとの集計を別に残さなくても、生のログを半年分持てる見込み。実装のあと 1 週間の実測で確かめ、合わなければ保持の値を見直す。

### 3.3 比べた案（圧縮）

| 案 | 依存 | 良い点 | 悪い点 |
|----|------|--------|--------|
| **gzip（flate2）（推奨）** | 純 Rust の小さい crate 数個 | ビルドが軽い。`gzcat`、`zless`、Python の `gzip` で読める | xz より大きい（推測で 1.3〜1.5 倍） |
| xz（`xz2`） | liblzma の C のビルドか、システムのライブラリ | 圧縮率が高い | C のビルドが入る。macOS は liblzma のヘッダを公開していない（推測）。量が §3.2 の見積もりなら差は十数 MB で、得るものが小さい |
| xz（`lzma-rs`） | 純 Rust | C なし | 遅い、保守が活発でない（推測） |
| macOS の Compression フレームワーク（FFI） | crate なし | 依存が増えない | エンジンに unsafe な FFI と macOS 専用のコードが入る。`cargo test` を macOS 以外で流せなくなる。gzip の形式にはヘッダを自分で書く必要がある |
| 殻が macOS の gzip コマンドを起動する | なし | 依存なし | 書き手（エンジン）と圧縮する側（殻）が分かれ、回したファイルの受け渡しの取り決めが増える |
| 日ごとの集計（件数と分布だけ）をエンジンが別ファイルに残す | なし | 長く持っても小さい | 集計のロジックがエンジンと集計スクリプトの 2 か所になる。§3.2 の見積もりなら生のログを半年持てるので、今は要らない。保持を短く（例: 30 日）決めたときだけ、集計スクリプトの出力を日ごとに保存する形で足す |

---

## 4. 何を記録するか

### 4.1 決まり

- 値は**数と固定の語だけ**。読み、候補、表記、周辺テキスト、ショートコード名は書かない（NFR-19）。文字数（長さ）は数なので書く（`ctx_change` の `reading=` と同じ扱い）。
- secure のセッションでは何も書かない（殻は送らず、エンジンは捨てる。NFR-18）。英語のモードでも secure は素通しで、事象も作らない（ADR 0026）。
- 既存の行（`convert` の `cache`・`lm_applied`、`resize_segment` の `segments`・`dir`、`ctx_change` など）と同じ値は足さない。**エンジンの行 = 要求とエンジンの内部、`sh_episode` = ユーザーの操作**、と役を分ける。
- 殻は打鍵ごとには送らない。合成 1 回（`idle` から次の `idle` まで。英語のモードのショートコードの状態も含む）に `sh_episode` を 1 行。XPC の量は `Commit` と同じくらいに収まる。

### 4.2 殻の事象（`DiagEvent`）

**`sh_episode`**（状態が `idle` に戻ったときに 1 行。範囲 1・2・3・4）

| キー | 種類 | 値 | 範囲 |
|------|------|----|------|
| `mode` | タグ | `romaji` `kana` `shortcode_en` `shortcode_ja` | 2 |
| `start` | タグ | `typing` `reconvert`（確定後の再変換から始まった） | 1 |
| `end` | タグ | 状態が終わった理由: `commit_cand`（候補を選んで確定） `commit_segments` `commit_kana`（変換せずに確定。Enter） `esc_clear`（Esc で消した） `bs_empty`（Backspace で空にした） `commit_composition` `deactivate` `mode_switch` `click` `secure` `shortcode_literal`（ショートコードをそのまま確定） `shortcode_pick` | 3 |
| `prev` | タグ | `end` が `commit_composition` `deactivate` `mode_switch` `click` のときの直前の状態: `composing` `converting` `segments` `ascii_run` `shortcode` `shortcode_choice` | 3 |
| `keys` | 数 | 打鍵の数（合成の間に殻が受けたキー） | 2 |
| `keys_to_conv` | 数 | 最初の Space までの打鍵の数（変換しなければ書かない） | 2 |
| `rlen` | 数 | 最初の変換のときの読みの文字数 | 2 |
| `dur_ms` | 数 | 最初の打鍵から終わりまで | 2 |
| `pick_ms` | 数 | 最初の Space から確定まで（変換しなければ書かない） | 2 |
| `spaces` | 数 | 変換中の Space の回数（最初の Space を含む） | 1 |
| `cand_moves` | 数 | 候補の移動（Space の 2 回目以降と ↑↓）の回数 | 1 |
| `pages` | 数 | ページを進めた回数 | 1 |
| `pick` | 数 | 確定した候補の番号（通し番号。`commit_cand` のときだけ） | 1 |
| `pick_kind` | タグ | 確定した候補の種類: `dict` `hiragana` `katakana` `rewriter` `literal` `fallback` `emoji` `small_letter` | 1・4 |
| `digit_pick` | 数 | 数字キーで選んだ回数 | 1 |
| `segs` | 数 | 確定したときの文節の数 | 1 |
| `seg_focus` | 数 | 文節のフォーカス移動の回数 | 1 |
| `seg_resize` | 数 | 区切り直しの回数（エンジンの `resize_segment` の行は要求ごとの記録。こちらは合成 1 回の合計で、集計の単位が違う） | 1 |
| `seg_changed` | 数 | 0 番以外を選んだ文節の数（`commit_segments` のとき） | 1 |
| `esc_back` | 数 | Esc で変換から読みに戻した回数 | 1 |
| `bs` | 数 | Backspace の回数 | 1・3 |
| `panel` | タグ | 候補ウィンドウの位置の取り方（最初に出したとき。殻の `AnchorSource` と 1 対 1）: `client`（IMK の `firstRect`。ふつう） `accessibility`（権限があるときのアクセシビリティ） `last_known`（同じアプリで最後に取れた値） `mouse_screen`（取れず、マウスのある画面の中央下） `none`（出さなかった） | 3 |
| `ctx` | タグ | 殻が周辺テキストを取れたか（最初の変換のとき。変換しなければ書かない）: `ok` `none`（`available = false`） | 3 |
| `wait_us` | 数 | 殻が最初の `Convert` の返事を待った時間（XPC を含む。エンジンの `convert` の所要とは別） | 5 |
| `fallback` | 数 | エンジンなしの候補を出した回数 | 3 |

**語の確定（D4b、#300。2026-10-25）**: `panel` と `ctx` の値の名前は、殻の取り方（`PanelPlacement.swift` の `AnchorSource`、`ClientAccess.surroundingText`）に合わせて決めた。当初案の `line_rect` `first_rect` `fallback` は殻に対応する区別がない（取り方は `firstRect` の 1 通り）ので、`client` `accessibility` `last_known` `mouse_screen` に替えた。`denied` は IMK が「取れなかった」としか返さず、権限の拒否を見分けられないので作らない。`pick_kind` の `emoji` `small_letter` は、ショートコードの確定（日本語・英語のどちらのモードも）で付く。`end=bs_empty` は、ショートコードの `:` だけで Backspace した場合も指す（日本語のモードの D4a は `esc_clear` にしていたのを直した）。英語のモードの `sh_episode` は `EnglishMode` が数え、`recordEnglishEpisode` で送る（`wait_us` `ctx` `rlen` `pick` `segs` は付かない）。

**`sh_mode`**（英語・日本語のモードが替わったとき。範囲 2）: `from`（`ja` `en`）、`to`（同）、`via`（`key` = ikari のキー割り当て `mode.*`、`os` = macOS 経由）。入力メニュー・英数キー・かなキーは、macOS からの通知の時点では区別できない（英数・かなキーは通知のあとに届く）ので `os` にまとめた。合成中に替えた場合は、その合成の `sh_episode`（`end=mode_switch`）が先に出る。

**`sh_engine`**（エンジンとの通信の失敗。§2.3。範囲 3）: `timeout` `interrupted` `invalidated` `fallback_convs` `breaker_open` `reconnect`（接続の作り直し） `resync`（`Hello` とセッションの送り直し）（いずれも数）。前回送ってから 0 でないときだけ。送る専用のタイマーは持たず、ほかの殻の事象（`sh_episode` など）を送るときに相乗りする（secure では何も送らないので、そのセッションの事象には相乗りしない）。つながらず送れなかった数は持ち越す。`fallback_convs` は `convert` が失敗（時間切れ、落ちた、壊れた返事）した回数で、返事が空だった場合は含まない（`sh_episode` の `fallback` は空の場合も含む）。

### 4.3 エンジンの事象（エンジンが自分で書く）

| kind / 足す注記 | 値 | 範囲 |
|------|----|------|
| `commit`、`commit_segments` に `rw=` | 確定した候補が Rewriter のものなら、その Rewriter の名前（`postal` `date` `weekday` `number` `calc` `unicode` `symbol` `emoticon` など。Rewriter の固定の名前。実装のときに一覧を許可リストにする） | 4 |
| `convert`、`prefetch` に `ctx=0` | 周辺テキストが `available = false` のときだけ足す（`ctx_change` は番号が変わったときしか出ないので、回数はここで数える） | 3 |
| `lm_load` | 所要（us）、`err`（失敗なら種類）。読み込みが終わったとき | 5 |
| `lm_unload` | `reason=idle` `settings` `shutdown` | 5 |
| `stat` | `rss_kb=`、`sessions=`。前回の `stat` から要求があったときだけ、10 分ごと（見回りのスレッド） | 5 |
| `log_dropped` | `n=`（キューが満杯で捨てた行の数） | 健全性 |
| `diag_reject` | `n=`（許可リストで捨てた値・行の数）。1 分ごとにまとめて | 健全性 |

**実装での決め方（D3、#299・#302。2026-10-25）**:
- `DiagEvent` の行には、エンジンが `session=<id>` を先頭に足す（`begin_session` の行の `session=` `app=` と結ぶため。殻は送らない）。許可リストを 1 つでも外れたら行ごと捨てる。
- `diag_reject` は事象ごとに `n=1 reason=<語> [kind=<許可リストの kind>]` を 1 行（1 分ごとのまとめはしない。集計は `n` の合計で足りる）。理由の語は `kind` `too_many` `too_long` `num_key` `tag_key` `tag_value` `dup_key` `no_session`。送られた文字列は書かない。secure のセッションは何も書かずに捨てる。休止中（`EndSession` のあと）のセッションの事象は書く。
- `begin_session` の行は `session=N app=<Bundle ID>`（`[A-Za-z0-9.-]` で 128 バイトまで。外れたら `invalid`、空なら `none`）。secure は `session=N` だけ（Bundle ID を書かない。NFR-18）。
- `end_session` の行には `session=N` を足す。`err=UNKNOWN_SESSION` の行にも、要求が持つ `session=N` を足す（`prefetch` `convert` `commit_text` `reconvert`。番号だけ。#313）。集計が、セッションの区切りと、知らないと言われた番号の状態（begin なし / end のあと）を調べるため。
- `rw=` は `commit` `commit_segments` の確定した候補の Rewriter の名前（複数の文節なら `,` でつなぐ）。`ctx=0` は `convert` `prefetch` で周辺テキストがない（`available = false` または送られなかった）とき。
- `lm_load` は暖機のスレッドが読み込みを終えたとき（失敗は `err=lm_load_failed`）。`lm_unload` の `idle` は見回りが「読み込み済み → 未読み込み」を見つけたとき（最大 1 秒の遅れ）、`settings` は Stack の差し替えで読み込み済みの LM が替わったとき、`shutdown` は `Engine` を閉じるとき。
- `stat` は見回りが 10 分ごと（前回から要求があったときだけ）に書く。RSS は `ps` を見回りのスレッドから呼んで取る（unsafe と新しい依存を避けるため）。取れなければ `rss_kb` を省く。

**性能の p50 / p99（範囲 5）は新しい行を足さない。** 既存の `convert` `segment_page` `page` `prefetch_done` の所要（us）と `cache` `lm_applied` から集計スクリプトで出す。殻から見た待ち時間は `sh_episode` の `wait_us`。

### 4.4 書かないもの（今回）

- Bundle ID（§7。ユーザーの了解待ち）。
- 打鍵ごとの事象、キーの種類ごとの回数（ADR 0026 の統計の役。既定オフで別のスイッチ）。
- 確定した文字列の長さ以外の性質（字種の割合など）。必要になったら別に決める。

### 4.5 行の例

```
1793000000123 sh_episode 0us lm=ready err=- mode=romaji start=typing end=commit_cand keys=9 keys_to_conv=8 rlen=6 dur_ms=2140 pick_ms=830 spaces=2 cand_moves=1 pages=0 pick=1 pick_kind=dict segs=1 seg_focus=0 seg_resize=0 esc_back=0 bs=1 panel=client ctx=ok wait_us=6200 fallback=0
1793000000456 sh_episode 0us lm=ready err=- mode=shortcode_en start=typing end=commit_composition prev=shortcode keys=4 dur_ms=900 bs=1 panel=none
1793000000789 commit 12us lm=ready err=- rw=postal
```

---

## 5. 集計

- 置き場所: `scripts/diag-report.py`（ユーザーのデータを読む道具なので `eval/` ではなく `scripts/`）。Python 3 の標準ライブラリだけ（`gzip`、`argparse`、`collections`、`statistics`）。
- 入力: `--logs DIR`（既定 `~/Library/Application Support/ikari-ime/logs`）、`--since YYYY-MM-DD`、`--until YYYY-MM-DD`。`engine.log` と `engine-*.log.gz` を時刻順に読む。知らない kind とキーは無視する（古い・新しいログが混ざっても止まらない）。
- 出力: 既定は Markdown の表を標準出力に、`--tsv` で機械向け。ファイルには書かない（リポジトリに入れるときは手で `eval/reports/` に貼る）。

| 節 | 中身 |
|----|------|
| 直し方の分布 | 変換して確定した合成について、`spaces`（0 / 1 / 2 / 3 / 4+）、`pick`（0〜8 / 9+）、`pages` > 0 の率、`segs` > 1 の率、`seg_focus` > 0、`seg_resize` > 0、`esc_back` > 0、`digit_pick` > 0 の率。`mode` 別 |
| 1 位のまま確定した率 | 分母 = `end` が `commit_cand` か `commit_segments` の合成。1 位のまま = `pick = 0`（または `seg_changed = 0`）かつ `seg_resize = 0` かつ `esc_back = 0`。外れた率 = 1 − それ。`start=reconvert` は別に数える。日ごとの推移 |
| 状態が終わった理由の内訳 | `end` × `mode` の件数と率。`end` が `commit_composition` `deactivate` `mode_switch` `click` のものは `prev` の内訳。`bs_empty` と `esc_clear` の率 |
| 入力の流れ | `rlen`、`keys_to_conv`、`dur_ms`、`pick_ms` の p50 / p90。`sh_mode` の 1 日あたりの回数 |
| 不具合の手がかり | `ctx` の内訳と `convert` の `ctx=0` の率、`panel` の内訳、`sh_engine` の合計、`err` が `-` でない行の kind 別の件数 |
| 機能の利用 | `pick_kind` の件数（`emoji` `small_letter` `rewriter` など）、`rw` の内訳、`mode=shortcode_*` の合成の件数と `end=shortcode_pick` の率 |
| 性能 | `convert` の所要の p50 / p95 / p99（`cache` × `lm_applied` 別）、`wait_us` の p50 / p99、`stat` の `rss_kb` の中央値と最大、`lm_load` の回数と所要、`lm_unload` の理由 |
| ログの健全性 | `log_dropped` と `diag_reject` の合計、日ごとの行数 |

- テスト: 小さな固定のログ（`.log` と `.log.gz`）を `scripts/testdata/diag/` に置き、出力の形を確かめる。`scripts/check.sh` に足すかは実装の issue で決める（Python が要るため。要確認）。

---

## 6. 段階と issue の切り方（1 PR 1 タスク）

| 段階 | issue（案） | 中身 | 持つファイル | 前提 |
|------|------------|------|-------------|------|
| D1 | engine.log の書き込みを別スレッドにし、回した世代を gzip にする | §3.1・§3.2。行の形と kind は変えない。warm p99 を前後で比べる | `engine/src/log.rs`、`engine/src/lib.rs`（join）、`engine/Cargo.toml`（flate2）、ログを読むテスト | なし（D2 と並行できる） |
| D2 | 診断ログの契約（`DiagEvent`） | §2.2 の proto、`gen-proto.sh` の生成物、`EngineClient.diag` と既定の何もしない実装、エンジンの dispatch で Ack を返すだけの枝、`diag_schema.rs` に §4.2 の許可リスト | `proto/`、`macos/Sources/IkariProto/`、`EngineClient.swift`、`engine/src/dispatch.rs`、`engine/src/diag_schema.rs` | なし |
| D3 | エンジンの診断の行 | `DiagEvent` を許可リストで確かめて書く（secure を捨てる、`diag_reject`）。§4.3 の `rw` `ctx=0` `lm_load` `lm_unload` `stat` `log_dropped` | `engine/src/` | D1、D2 |
| D4 | 殻の診断の事象 | `IkariShellCore` に合成 1 回の数を貯める型（純粋で、状態機械のテストで確かめる）を足し、`idle` に戻ったときに `sh_episode` を作る。`sh_mode`、`XPCEngineClient` の `sh_engine` と `diag` の送信 | `macos/Sources/IkariShellCore/`、`IkariInputController.swift`、`XPCEngineClient.swift` | D2（D3 と並行できる。エンジンが捨てても殻のテストは通る） |
| D5 | 診断ログの集計スクリプト | §5 | `scripts/diag-report.py`、`scripts/testdata/diag/` | D2（行の形。§4.5 の例で先に作れる） |
| 保留 | Bundle ID を診断ログに足す | §7 | | ユーザーの了解 |

- D1 だけで「数日で消える」は解消する（保持が延びる）。D1 は #295 の他の部分がなくても単独で価値がある。
- D4 は大きくなりやすい。D4a（`sh_episode` の直し方と終わった理由）と D4b（`panel`、`ctx`、`sh_mode`、`sh_engine`）に分けてよい。#294 の調査に要るのは D4a の `end` と `prev`。
- 完了の確かめ: D3・D4 が入ったビルドを 1 週間使ったあと、D5 で表が出ること、`log_dropped` が 0 であること、warm p99 が D1 の前から悪化していないこと。

---

## 7. Bundle ID（了解が出たら足す案。今は入れない）

`session.rs` の決定（client_bundle_id は保存もログもしない）を改めることになるので、ユーザーの了解が要る。了解が出た場合の案:

- **案 7-1（推奨）: そのまま書く。** `begin_session` の行に `app=<Bundle ID>` を足す（`sh_episode` には入れず、集計は `session_id` で結ぶ。そのため `sh_episode` に `session=` の数を足す）。Bundle ID は `[A-Za-z0-9.-]` だけ、128 バイトまでを許し、外れたら `app=invalid`。#294 のような「VSCode でだけ起きる」を直接見られる。
- 案 7-2: 塩つきのハッシュで書く。どのアプリかは分からず、アプリごとの偏りだけ分かる。不具合の切り分けには弱い。
- 案 7-3: よく使うアプリだけ固定の名前（`vscode` `safari` …）にし、他は `other`。許可リストの保守が要る。

いずれも、ログのファイルは手元から出ない（ネットワークに送らない）。

---

## 8. 結果

- 得るもの: 直し方と終わり方が合成ごとに残り、#249・#294 のような「ログを足して待つ」を減らせる。ログが数日でなく半年（案）残る。入力の経路から write と rename が消える。
- 失うもの・増えるもの: proto に 1 種類、エンジンに許可リストと書き込みのスレッド、`flate2` の依存、殻に合成 1 回の数を貯める型。殻とエンジンの両方で事象の名前を合わせる手間（許可リストが 1 か所なので、外れたら `diag_reject` で分かる）。
- 変えないもの: 候補の作り方、学習、LM。ADR 0026 の統計（既定オフ）は別物で、この ADR は統計のスイッチに関わらない。**診断ログは常時オン**（ユーザー決定 2026-10-25）で、スイッチは作らない。

---

## 9. ユーザーの決定（2026-10-25。3 点とも推奨どおり）

1. **Bundle ID を書く**: そのまま、`begin_session` の行だけに書く（案 7-1）。session.rs の「保存もログもしない」の決定は、この点だけ改める。
2. **保持**: **180 日、合計 100MB まで**（どちらか先に来た方で古い順に消す）。見積もり（180 日で 40〜45MB。推測）は、1 週間の実測で見直す。
3. **圧縮**: **gzip（flate2）。日ごとの集計は作らない**。

---

## 参考

- issue #295、#249、#294
- ADR 0004 §7.5・§7.6、ADR 0021 L0・L2、ADR 0026（統計と secure）
- spec の NFR-18・NFR-19
