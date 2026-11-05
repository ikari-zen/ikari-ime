# ADR 0051: 前回の異常終了を、次の起動で診断ログに記録する（起動中の印、致命的なシグナルの記録、panic の記録）

- 状態: 採用（2026-11-05 ユーザー決定。§10 の 3 点はすべて推奨どおり）
- 日付: 2026-11-05
- 対象: issue #98。新しい crate `runmark/`（印の書式と判定、シグナルの処理）、`engine/src/lib.rs`・`engine/src/diag.rs`・`engine/src/diag_schema.rs`、`engine-ffi/src/lib.rs`、`shell-core/src/engine/`（`sh_prev_exit` と `sh_panic` の相乗り）、`shell-ffi/src/shell.rs`（設定に `diag_dir`、終了の口）、`macos/Sources/IkariIME/`（`diag_dir` を渡す、終了の口を呼ぶ）、`scripts/diag-report.py`
- 前提: main = bde8c35。ADR 0034（診断ログ。§2.1 の案 B、§2.3、§4.1 の決まり、§4.2 の `sh_engine`）、ADR 0004 §5.1・§7.4（XPC サービスと `engine.lock`）・§7.5、ADR 0041・0042（判断は Rust）、spec の NFR-18・NFR-19
- 判断の置き場所: **判定はすべて Rust**（`runmark` の書式と判定、engine の `prev_exit` の行、shell-core の相乗り）。Swift は `diag_dir` を設定に入れて渡すことと、`applicationWillTerminate` で終了の口を 1 回呼ぶことだけ（どちらも OS の入口でしか取れない情報の受け渡し）
- この ADR ではコードを変えない。確かめていないことは「推測」「要確認」と書く

---

## 0. 現状の把握（コードで確かめたこと）

| 項目 | 今の形 | 根拠 |
|------|--------|------|
| 異常終了の記録 | ない。殻の `sh_engine` に `interrupted` `invalidated` `breaker_open` などの**数**があるだけ。エンジンが落ちた理由も、殻が落ちたことも残らない | `shell-core/src/engine/counts.rs`、issue #98 の調査 |
| エンジンの終わり方 | 正常: 殻の `Shutdown`（版の不一致のとき）→ Swift の `EngineHost.close()` → `ikari_engine_close` → `Engine::drop`（学習の保存、`lm_unload reason=shutdown`、ログの flush）→ `exit(0)`。`close` が 5 秒で終わらなければ解放せずに `exit(0)`（`.leaked`） | `macos/Sources/IkariEngineService/main.swift`、`engine/src/lib.rs` の `Drop` |
| シグナルの処理 | エンジンにも殻にもない（`SIGPIPE` を無視するのは設定のブリッジだけ）。`SIGTERM` は既定の動作で即座に終わる | `macos/Sources/IkariBridge/BridgeTransport.swift` |
| panic | `engine-ffi` と `shell-ffi` の入口が `catch_unwind` で受け、`Internal` の返事か既定値を返す。**panic したこと自体はどこにも残らない**（ログの行は `Engine::handle` の中で書くので、panic した要求の行は出ない）。ワーカーのスレッドの panic は黙ってスレッドが終わる（推測） | `engine-ffi/src/lib.rs`、`shell-ffi/src/lib.rs` |
| 単一のエンジン | `engine.lock` を flock で持つ。プロセスが死ぬと解放され、ファイルは残る。CLI の学習を書き換えるコマンドも同じロックを取る | `engine/src/lock.rs` |
| 書き込める場所 | エンジンは Sandbox で `~/Library/Application Support/ikari-ime/` だけ。殻（`ikari`）は Sandbox なし。殻の Rust はすでに設定ファイルを読み書きしている | `macos/scripts/templates/engine.entitlements`、`shell-core/src/settings/store.rs` |
| install | `install.sh` は `killall ikari IkariEngine …`（`SIGTERM`）で止める | `macos/scripts/install.sh` |

**分かっていないこと（推測）**: launchd が XPC サービスをいつ止めるか。(1) 親のアプリ（`ikari`）が終わったとき、(2) メモリが足りないとき（処理中の返事がない「きれいな」状態なら）、(3) ログアウト・再起動のとき（`SIGTERM` のあと `SIGKILL`）。(1)(2) は `SIGKILL` で、プロセスの側では何もできない（推測。§7 のスパイクで確かめる）。

---

## 1. 問題の定義

- 1.0 で作者以外が使うと、「変換できなくなった」「候補が出ない時間があった」の原因が、エンジンの異常終了か、殻の異常終了か、OS が止めただけかを、診断ログから区別できない。
- 「起動中の印を置き、次の起動で印が残っていたら異常終了」という素朴な方式は、**OS が `SIGKILL` で止めた正常な終わりを異常終了と誤判定する**。誤判定の多い記録は、本物の異常を埋もれさせる。
- 制約: 診断ログは文字なし（NFR-19。数と固定の語だけ）、secure では書かない（NFR-18）、入力の経路に同期の I/O を足さない（ADR 0034 §3.1）、ログはエンジンに 1 本化（ADR 0034 §2.1 B）。

---

## 2. 決定の要約（提案）

1. **起動中の印**: エンジンは `<UserDir>/logs/engine.run`、殻は `<UserDir>/logs/shell.run` を、起動のときに作り直す。中身は固定の語と数だけのテキスト（§3）。正常に終わるときに `exit=clean` を、終わらせるシグナル（`SIGTERM` など）と致命的なシグナル（`SIGSEGV` など）を受けたときはシグナルの処理の中で `exit=term sig=N` / `exit=crash sig=N` を足す。
2. **次の起動での判定**: 最後の `exit=` の行で `clean` `term` `crash` を決め、`exit=` の行がなければ **`unknown`**（`SIGKILL`、電源断、カーネルの panic、シグナルの処理が動かなかった場合）。**`unknown` を `crash` と呼ばない**。これで launchd の `SIGKILL` の誤判定は「異常終了」ではなく「分からない終わり方」として残る（§4）。
3. **`SIGTERM`**: 記録してから既定の動作で即座に終わる（今の動作を変えない。学習は 5 秒ごとの保存のまま）。`SIGTERM` で学習を保存してから終わる処理は、この ADR では入れない（§6 の案 F）。
4. **panic**: `std::panic::set_hook` で、panic のあった場所（ワークスペースの中のソースのパスと行だけ）とスレッドの種類を記録する。panic のメッセージは書かない（§5.3）。
5. **書き先**: エンジンは自分の起動のときに engine.log に `prev_exit` の行を書く。殻は前回の判定を持っておき、既存の `DiagEvent` で `sh_prev_exit` を送る（`sh_engine` と同じく、secure でないセッションの事象に相乗り）。殻の panic は数えて `sh_panic` で送る。**ログの行はこれまでどおりエンジンだけが書く**。殻が書くファイルは印の 1 つだけ（§3.3 で ADR 0034 §2.1 との関係を書く）。
6. **DiagnosticReports**: 常駐のプロセス（殻・エンジン）は読まない。集計の `scripts/diag-report.py`（ユーザーが手で動かす）が、ikari のプロセス名で始まる `.ips` の**ファイル名と時刻だけ**を並べ、`prev_exit` の行と時刻で突き合わせる。中身は読まない（§5.4）。
7. **置き場所の言語**: 印の書式・判定・シグナルの処理・panic の場所の絞り込みは新しい crate `runmark`（Rust）。シグナルの処理を入れるのは、プロセスの入口である `engine-ffi`（`ikari_engine_open`）と `shell-ffi`（`ikari_shell_new`）。CLI とテストには入れない。

---

## 3. 起動中の印

### 3.1 書式

```
v=1 start_ms=1793000000123
exit=crash sig=11
```

- 1 行目は起動のときに書く（`O_TRUNC` で作り直す。権限は `PRIVATE_FILE_MODE`）。2 行目以降は追記（`O_APPEND`）。
- シグナルの処理の中では、**起動のときに作っておいた固定のバイト列**（シグナルごとの `exit=term sig=15\n` など）を `write(2)` で書くだけ（async-signal-safe。整形もメモリ確保もしない）。fd は起動のときに開いておき、静的な場所に置く。
- 正常の終わりは `exit=clean\n`。終わった時刻は書かず、次の起動でファイルの更新時刻（mtime）から `up_s`（動いていた秒数）を出す。`exit=` の行がない（`unknown`）ときは `up_s` を出さない。
- 読めない、`v=` が違う、壊れている → `unknown`。ファイルがない（初めての起動、`logs/` を消した）→ 何も記録しない。

### 3.2 置き場所

| 案 | 良い点 | 悪い点 |
|----|--------|--------|
| **`logs/engine.run`・`logs/shell.run`（推奨）** | 診断のためだけのファイルで、`logs/` ごと消しても失うものがない。エンジンの Sandbox の中。ログの保持と同じ場所で、利用者に説明しやすい | ログの回し方の後始末（`engine-*` と `engine.log.*` を探す）と名前がぶつからないことを確かめる必要がある（`engine.run` はどちらの形にも当たらない。テストで固定する） |
| `engine.lock` の中身に書く | ファイルが増えない | ロックは CLI も取る。CLI が取ったときに印を読む・消すかの取り決めが要り、役が 2 つになる。殻の印には使えない |
| `<UserDir>/state/` などの新しいディレクトリ | 役で分けられる | ディレクトリが増える。中身は診断にしか使わない |

### 3.3 殻が印を書くこと（ADR 0034 §2.1 B との関係）

ADR 0034 は「殻は自分でファイルに書かない」とした。理由は、ログの書き込み・回し方・保持を 2 つ作らないため。印はログではなく、1 つのファイルを起動のたびに作り直すだけで、回し方も保持もない。また、**シグナルの処理の中からは XPC で送れない**ので、殻が死んだことを記録するには殻のプロセスがファイルを持つしかない。よって、殻が書くのは `shell.run` の 1 つだけ、ログの行はこれまでどおりエンジンに 1 本化、と範囲を限って例外にする。書くのは Swift ではなく `shell-ffi` / `runmark`（Rust）。

### 3.4 書く順番（エンジン）

1. `Engine::open`: `engine.lock` を取れたあとで、前回の `engine.run` を読んで判定し、作り直す（ロックを取れなかったエンジンは印に触らない。2 つのエンジンが同じ印を読み書きしない）。
2. `EngineLog` を開いたら、最初の行として `prev_exit` を書く。
3. `Engine::drop`: 学習の保存、`lm_unload`、ログの flush のあと、最後に `exit=clean` を書く。
4. 印は、ログが有効なとき（`EngineLog::open` を使うとき）だけ扱う。CLI やテストで `EngineLog::disabled` のときは印も作らない。

殻: `ikari_shell_new` で `diag_dir` が空でなければ `shell.run` を読んで判定し、作り直し、シグナルの処理を入れる。Swift の `applicationWillTerminate` から `ikari_shell_note_exit`（新しい口）を呼ぶと `exit=clean` を書く。設定アプリ（`settings_app.rs`）は `diag_dir` を渡さないので、印もシグナルの処理も持たない。

---

## 4. 誤判定の扱い

### 4.1 終わり方ごとの記録

| 終わり方 | 印に残るもの | 記録される `exit` |
|----------|-------------|------------------|
| `Shutdown` → `close` → `exit(0)` | `exit=clean` | `clean` |
| `close` が時間切れ（`.leaked`）→ `exit(0)` | なし | `unknown`（まれ。§9） |
| `killall`（install）、ログアウト（推測）の `SIGTERM`、`SIGINT`、`SIGHUP` | `exit=term sig=N` | `term` |
| `SIGSEGV` `SIGBUS` `SIGILL` `SIGFPE` `SIGABRT`（二重の panic、`abort`）、`SIGTRAP`（Swift の `fatalError` など）、`SIGSYS` | `exit=crash sig=N` | `crash` |
| launchd の `SIGKILL`（親の終了、メモリ、ログアウトの後半。推測）、`kill -9`、電源断、カーネルの panic、スタックの溢れでシグナルの処理が動けない | なし | `unknown` |

- **`crash` は「致命的なシグナルを受けた」という事実だけ**で、誤判定はない（シグナルを `kill` で送られた場合は除く）。
- **`unknown` は異常とも正常とも言わない。** 集計では `crash` と分けて数え、「`unknown` が多い」こと自体を調べる材料にする。直前の行（最後の `convert` の時刻と `app=`、`sh_engine` の `interrupted`）は既存のログから読めるので、新しい値は足さない。
- 殻の `unknown` とエンジンの `unknown` が同じころに並ぶ（殻が `SIGKILL` され、launchd が XPC サービスも止めた、など）ことは、集計で時刻の近さとして見せる。

### 4.2 シグナルの処理の決まり

- `sigaction` で `SA_RESETHAND | SA_ONSTACK` を付け、前の処理（あれば）を保存する。処理の中では印に書いたあと、**前の処理に戻してシグナルを送り直す**。プロセスの終わり方（終了コード、クラッシュレポートの作成）は今と同じにする。
- macOS のクラッシュレポート（ReportCrash）は Mach の例外ポートで受けるので、シグナルの処理を足しても `.ips` は作られる（推測。§7 のスパイクで確かめる。作られなくなるなら、致命的なシグナルの処理は入れず `unknown` に寄せる）。
- 代わりのスタック（`sigaltstack`）は、入口のスレッドに 1 つ用意する。ほかのスレッドのスタックの溢れは `unknown` になってよい。
- `unsafe` と `libc`（すでに `Cargo.lock` にある）は `runmark` の `#[cfg(unix)]` のモジュールに閉じる。Windows（ADR 0040）では、のちに `SetUnhandledExceptionFilter` で同じ印を書く（この ADR では作らない）。

---

## 5. 何を記録するか（許可リスト）

### 5.1 エンジンが自分で書く行

| kind | 値 | いつ |
|------|----|------|
| `prev_exit` | `exit=clean\|term\|crash\|unknown`、`sig=<数>`（`term` と `crash` のときだけ）、`up_s=<数>`（`unknown` 以外） | `Engine::open` のあとの最初の行。印がなければ書かない |
| `panic` | `thread=<語>`、`loc=<場所>` | panic の hook。行はキューに `try_send` で入れるだけ（hook の中でディスクに触らない） |

- `thread` の語: `main`（要求を処理するスレッド）`ikari-log` `prefetch` `warmup` `patrol` `other`（実装のときに、エンジンが名前を付けているスレッドの一覧に合わせる）。
- 例: `1793000000123 prev_exit 0us lm=off err=- exit=crash sig=11 up_s=5310`

### 5.2 殻の事象（`DiagEvent`。`diag_schema.rs` の許可リストに足す）

| kind | 数のキー | タグのキーと値 |
|------|---------|---------------|
| `sh_prev_exit` | `sig`、`up_s` | `exit`: `clean` `term` `crash` `unknown` |
| `sh_panic` | `n`（前回送ってからの回数） | `loc`: §5.3 の場所（最後の 1 つ）、`thread`: `main` `other` |

- 送り方は `sh_engine`（ADR 0034 §4.2）と同じ: 専用のタイマーを持たず、secure でないセッションの事象を送るときに相乗りする。`sh_prev_exit` は起動ごとに 1 回だけ。送れなかったら持ち越す。secure のセッションには相乗りしない（NFR-18）。
- `loc` の値は許可リストの「一覧」にできないので、`diag_schema.rs` に「場所の形」の検査を足す: `[A-Za-z0-9_/.-]+\.rs:[0-9]+` で 96 バイトまで、`/` で始まらない、`..` を含まない。外れたら `external` に置き換える（行は捨てない）。

### 5.3 panic の場所（プライバシー）

- **メッセージは書かない。** `expect` や `format!` のメッセージには読みや候補が入りうる（NFR-19）。
- **場所は、ワークスペースの中のソース（相対パス）だけ書く。** 依存の crate や標準ライブラリの場所は `~/.cargo/registry/…` や `/rustc/…` のような絶対パスになり、ホームのパス（ユーザー名）を含むので、`external` に置き換える。ワークスペースの中のパスは、公開しているソースのパスと行番号で、ユーザーの情報を含まない。
- 絞り込みは `runmark` の 1 つの関数にし、エンジンの行と殻の事象の両方がそれを通る。殻の事象はエンジンの許可リストでもう一度確かめる（二重の守り）。

### 5.4 DiagnosticReports（`~/Library/Logs/DiagnosticReports`）

| 案 | 可否 | 理由 |
|----|------|------|
| 常駐のプロセスが中身を読んで送る | **否** | エンジンは Sandbox の外で読めない。殻が読むと、読む範囲が「ikari のデータのディレクトリ」から OS の共通のディレクトリに広がる。中身にはホームのパス、ほかのライブラリの情報、abort のメッセージ（panic のメッセージを含みうる）が入る。文字なしの原則と合わない |
| 常駐のプロセスがファイル名の数だけ数える | 否（今は） | シグナルの処理で `crash` は分かるので、得るものが小さいわりに読む範囲が広がる |
| **`diag-report.py` がファイル名と時刻だけを並べる（推奨）** | 可 | ユーザーが手で動かす道具で、常駐のプロセスは読まない。`ikari-` `IkariEngine-` で始まる `.ips` の名前だけを読み、`prev_exit exit=crash` の時刻の前後 2 分にあるものを「添えるとよいレポート」として名前で示す。中身は読まない、出力に中身を含めない。不具合の報告にレポートを添えるかは、ユーザーが中身を見て決める |

### 5.5 書かないもの

- Bundle ID（落ちたときの前面のアプリ）は印にも `prev_exit` にも書かない。直前の `convert` の行の `app=`（ADR 0034 §4.3、#109）で足りる。
- 処理中の要求の種類、読みの長さ、スタックトレース。

---

## 6. 検討した案

| 案 | 中身 | 判断 |
|----|------|------|
| A | 印だけ（シグナルの処理なし）。印が残っていれば「異常終了」 | 却下。`SIGTERM`（install のたび）と launchd の `SIGKILL` が全部「異常終了」になり、本物が埋もれる |
| **B（推奨）** | 印 + 終わらせるシグナルと致命的なシグナルの記録 + panic の hook。`exit=` がなければ `unknown` | 採用。`crash` に誤判定がなく、`SIGKILL` は `unknown` として量だけ見える。追加の I/O は起動と終了の 1 回ずつで、入力の経路に何も足さない |
| C | 常駐のプロセスが DiagnosticReports を読む | 却下（§5.4） |
| D | エンジンが、殻との接続が切れたことから殻の異常終了を推定する | 却下。接続が切れた理由（終了、`SIGKILL`、クラッシュ）はエンジンから分からない |
| E | 処理中の要求の有無を印に書く（要求の始めと終わりに `pwrite` 1 バイト） | 保留。`SIGKILL` のときに処理中なら launchd の idle の停止ではない、と分けられる（推測）が、`Convert` の経路に 2 回の書き込みが入る。スパイクと実測で `unknown` が多いと分かったら別の issue で足す |
| F | `SIGTERM` を受けたら学習を保存してから終わる | 保留（別の issue）。シグナルの処理から `Engine` を閉じるには、自分宛てのパイプと Swift 側の `close` との取り決めが要り、`.leaked` と同じ難しさがある。今も `SIGTERM` では保存せずに終わっており、失うのは最大 5 秒の学習 |
| G | `engine.lock` の中身を印にする | 却下（§3.2） |
| H | 殻の印を Swift（`UserDefaults` など）で持つ | 却下。判定が Swift に入る（ADR 0042）。シグナルの処理の中から `UserDefaults` には書けない |

---

## 7. スパイク（捨てるコード。実機で確かめる。ユーザーの了解が要る）

確かめること:
1. シグナルの処理を入れても、`SIGSEGV` と `SIGABRT` で `.ips` が今までどおり作られるか（作られなければ §4.2 のとおり致命的なシグナルの処理を外す）。
2. launchd が XPC サービスを止めるとき、どのシグナルが来るか（親の `ikari` を `SIGTERM` したとき、長い idle、ログアウト）。`SIGKILL` ならエンジンの記録は `unknown` になる。
3. 殻（`ikari`）に `applicationWillTerminate` が来る場面があるか。

方法: シグナルを受けたら `logs/spike-signals.txt` に固定の語を追記するだけの検証版。手順書つき。**入れ替えと `kill` を伴うので、ユーザーの ikari が使えない時間ができる。実施の前にユーザーに聞く。** 結果は本 ADR の §0 の「分かっていないこと」に書き足す。B の設計はスパイクの結果に依らず成り立つ（変わるのは `unknown` の量の見込みと、1 の結果しだいで致命的なシグナルの処理を入れるかだけ）。

### 7.1 結果（2026-11-06。実機の検証版で、エンジンのプロセスに `kill` を送った）

| 送ったもの | 記録（`engine.run`） | 次の起動の `prev_exit` | クラッシュレポート |
|---|---|---|---|
| `SIGSEGV` | `exit=crash sig=11` | `exit=crash sig=11 up_s=71` | `IkariEngine-….ips` が作られた |
| `SIGABRT` | `exit=crash sig=6` | `exit=crash sig=6 up_s=44` | 同上 |
| `SIGKILL` | 追記なし | `exit=unknown` | — |
| `SIGTERM` | 追記なし（**エンジンは 10 秒たっても終了しなかった**） | — | — |

- 項目 1: シグナルの処理を入れても `.ips` は今までどおり作られる。致命的なシグナルの処理は、このまま入れる。
- `SIGTERM`: エンジンのプロセスは `kill -TERM` で終了しなかった。runmark は、起動のときに無視されているシグナルには処理を入れないので、`SIGTERM` は XPC の仕組みが受けている（推測。確かめていない）。したがって、**エンジンで `exit=term` が書かれる場面は、今のところ見つかっていない**。launchd が止めたときは `unknown` になる（`SIGKILL` と同じ扱い）。§3 の 3 の「`SIGTERM` を記録してから終わる」は、殻（`ikari`）に限った話になる。
- 項目 2（launchd がどのシグナルで止めるか）と項目 3（殻の `applicationWillTerminate`）は、まだ確かめていない。

---

## 8. 実装の進め方（レーンとファイルの境界）

| 段階 | 中身 | 持つファイル | 前提 |
|------|------|-------------|------|
| C0 契約 | `runmark` の型とシグネチャのスタブ（`Exit`、`PrevExit { exit, sig, up_s }`、`RunMark::begin(path) -> (RunMark, Option<PrevExit>)`、`RunMark::finish_clean()`、`install_signal_handlers()`、`install_panic_hook(sink)`、`sanitize_location(&str) -> &str`）、`diag_schema.rs` に `sh_prev_exit` `sh_panic` と場所の形の検査、`IkariShellConfig.diag_dir`、`ikari_shell_note_exit` の宣言とスタブ、`prev_exit` `panic` の kind の名前。**テストの先書き**（下記）は `#[ignore]` で入れる | `runmark/`（新設）、`Cargo.toml`（members）、`engine/src/diag_schema.rs`、`shell-ffi/src/shell.rs`、`macos/Sources/IkariShellABI/` の宣言 | なし |
| R1 | `runmark` の中身: 書式、判定、mtime からの `up_s`、シグナルの処理（unsafe、cfg(unix)）、panic の hook、場所の絞り込み | `runmark/**` | C0 |
| R2 | エンジン: §3.4 の順で印を扱い、`prev_exit` を書く。panic の hook から `panic` の行。`engine-ffi` の `ikari_engine_open` で 1 回だけシグナルの処理と hook を入れる。hook はエンジンを開き直しても使えるよう、差し替えられる場所を通して `EngineLog` に送る | `engine/src/lib.rs`、`engine/src/diag.rs`、`engine/src/log.rs`（後始末が `*.run` に触らないテスト）、`engine-ffi/src/lib.rs`、`engine/tests/` | C0（R1 と並行。スタブで書ける） |
| R3 | 殻: `ikari_shell_new` で印と処理と hook、前回の判定と panic の数を `shell-core` の engine の状態に持たせ、`sh_engine` と同じ相乗りで送る。Swift は `diag_dir` を渡す 1 行と、`applicationWillTerminate` で `ikari_shell_note_exit` を呼ぶ 1 行 | `shell-core/src/engine/`（`counts.rs` の隣に新しい型）、`shell-ffi/src/shell.rs`、`macos/Sources/IkariIME/AppContext.swift`・`ShellDriver.swift`・`main.swift` | C0（R1・R2 と並行） |
| R4 | 集計: `diag-report.py` に「異常終了」の節（エンジン・殻の `exit` の内訳と日ごとの数、`crash` の直前の `convert` の `app=` の内訳、`panic`・`sh_panic` の `loc` の内訳、DiagnosticReports のファイル名の突き合わせ） | `scripts/diag-report.py`、`scripts/testdata/diag/` | C0（行の形で先に作れる） |

**先に書くテスト**（C0 で `#[ignore]`、各レーンで外す）:
- `runmark`: 判定の表（印なし → なし、1 行目だけ → `unknown`、`exit=clean` → `clean`、`exit=term sig=15` → `term`、`exit=crash sig=11` → `crash`、壊れた行・別の版 → `unknown`）。子のプロセス（テストの実行ファイルを環境変数つきで自分で起動）で `SIGSEGV`・`abort`・`SIGTERM`・`SIGKILL` を起こし、次の `begin` の判定が `crash`・`crash`・`term`・`unknown` になること。場所の絞り込み（相対の `.rs:行` は残す、絶対パス・`..`・長すぎ → `external`）。
- エンジン: 前回の印が `crash` なら最初の行が `prev_exit exit=crash sig=11`、ロックを取れないエンジンは印を変えない、`drop` のあと印の最後が `exit=clean`、`EngineLog::disabled` では印を作らない、ログの後始末が `engine.run` を消さない。
- shell-core（`src/engine/tests.rs` の替え玉の transport）: 前回 `crash` なら最初の secure でないセッションの事象に `sh_prev_exit` が 1 回だけ相乗りする、secure では送らない、送れなければ持ち越す、`sh_panic` の `n` は送った分だけ引く。キー操作の状態機械は変えないので、`shell-core/scenarios/` の JSON は足さない。
- 許可リスト: `sh_prev_exit` の `exit` に一覧の外の語 → `diag_reject reason=tag_value`、`loc` の形が外れる → `external` に置き換えて受ける。

確認: 各レーンで `scripts/check.sh`。統合のあと、スパイクの手順の一部（`kill -SEGV` を検証用のエンジンに送る）を smoke と同じ扱い（ユーザーの了解のあと）で行う。

---

## 9. 懸念点

- **スパイクの 1 が否なら**（シグナルの処理で `.ips` が作られなくなる）、致命的なシグナルの処理は入れず、`crash` と `SIGKILL` は両方 `unknown` になる。そのときは DiagnosticReports の突き合わせ（§5.4 の推奨案）が `crash` を見分ける唯一の手がかりになる。
- `.leaked`（`close` の時間切れ）のあとの `exit(0)` は `unknown` になる。まれだが、分けたければ Swift の `.leaked` の枝から `ikari_engine_note_exit` のような口を呼ぶ（今は作らない）。
- panic の hook はプロセスで 1 つ。依存の crate が hook を差し替えると記録が止まる（今の依存にそうするものはない。推測）。hook は前の hook を呼んでから記録する。
- `up_s` は mtime に頼る。時計が戻ると負になりうるので、負なら書かない。
- 1.0 で作者以外が使うとき、`logs/` に 2 つのファイルが増えることを、診断ログの説明（文書）に書き足す。
- シグナルの処理は `unsafe` で、テストは子のプロセスで行う。CI の macOS と、のちの Windows（cfg で外れる）で流れることを確かめる。

---

## 10. ユーザーに聞くこと（推奨つき）

1. **誤判定の扱いと `SIGTERM`**: 印が残っていても「異常終了」とは呼ばず `unknown` として分けて数え、`crash` は致命的なシグナルを受けたときだけにする。`SIGTERM` は `term` と記録してから今までどおり即座に終わる（学習の保存はしない。保存は別の issue）。**推奨: この形**。
2. **印の置き場所と、殻が書くファイル**: `logs/engine.run` と `logs/shell.run` を足し、殻はこの 1 ファイルだけ自分で書く（ADR 0034 §2.1 の「殻はファイルに書かない」の例外。ログの行はエンジンだけ）。**推奨: この形**。
3. **プライバシーの線引き**: panic のメッセージは書かず、場所はワークスペースの中のソースのパスと行だけ書く（ほかは `external`）。DiagnosticReports は常駐では読まず、`diag-report.py` がファイル名と時刻だけを並べる。**推奨: この形**。

---

## 参考

- issue #98、#295（ADR 0034）、#109
- ADR 0004 §5.1・§7.4・§7.5、ADR 0034、ADR 0040、ADR 0041、ADR 0042
- spec の NFR-18・NFR-19
