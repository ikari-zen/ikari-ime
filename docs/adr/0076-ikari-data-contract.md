# ADR 0076: 取得する部品 `ikari-data` の契約（プロセスの形、権限、入力と出力、版の確認、失敗、設定画面との口）

- 状態: 承認（2026-11-17 ユーザー決定。質問はすべて推奨どおり。Q2 は「OS の curl をやめ、`ureq` と OS の TLS にする」。§15 を見る。同じ日の 2 回目の決定（失効した証明書の扱い、版の確認とデータの Release）は §18）
- 日付: 2026-11-16（2026-11-17 に決定と実測を反映。2026-11-19 に [ADR 0078](0078-fetch-child-handle-and-appcontainer.md) の段 1（書く先のハンドル渡し。#449）で §3.2・§5.1・§11 の 2・§17 を改めた）
- 対象: 新しい crate `ikari-data`（置き場所 `fetch/`。lib と bin `ikari-data`）、`resource`（束を作る関数と取得の報告の型）、`windows/settings`（設定画面の核。起動と進み具合）、`windows/settings-ui`（ボタンと表示だけ）、`engine`（ファイルからのモデルの取り込み。Windows）、`shell-ffi`（同。macOS の設定アプリ）、`xtask`（検査）、`deny.toml`、インストーラ。この ADR ではコードを変えない
- 前提: origin/feat/security-impl2 = 0bd7822。[ADR 0071](0071-settings-ui-and-fetcher.md) §3（`ikari-data` の範囲。Q3 で 1.0.0 まではモデルだけ。Q2 の「OS の curl」はこの ADR の §9 で改める）、[ADR 0070](0070-update-and-vulnerability-response.md) §3 の 3 と Q3（「新しい版を確認」のボタン）、[ADR 0066](0066-security-premises.md) §6（通信しないの定義と L1〜L3）、[ADR 0058](0058-offline-resource-install.md)（inbox と束、検証、入れ替え）、[ADR 0060](0060-user-selectable-lm-model.md)（自由指定のモデル、sha256 なしは警告で許可、試しの採点、`.loading`）、[ADR 0068](0068-ipc-endpoints.md) §4（設定の口、パスだけを流す）、[ADR 0072](0072-lm-cage-levels.md)（檻）、[security-design.md](../security-design.md) §8.5 の D-2
- 判断の置き場所: すべて Rust。取得の判断（どこから、何を、どの設定で、何を確かめるか）は `ikari-data`、束の形と sha256 は `resource`、取り込みの判断はエンジン。設定画面（C#、Swift）は、ボタン、ファイルを選ぶ窓、核が作った文の表示だけ（UI であるため）
- ほかの ADR との関係: 0071 §3 は「何をしてよいか」を決めた。この ADR は、それを実装できる形（型、呼び出し、終了の仕方、置き場所、検査）にする。改めるのは 2 つ: 0071 Q2 の通信の手段（OS の curl → `ureq` と OS の TLS。§9）と、0060 §6 の「`ikari-data model import --file`」（§7）

---

## 1. 分かったこと（2026-11-16）

- `Cargo.lock` に HTTP・TLS・ソケットの crate は無い。`deny.toml` の `[bans]` と `xtask/src/check/supply.rs` の `NET_CRATES` が同じ一覧で止めている。例外の欄 `NET_CRATES_ALLOWED` は空。`Cargo.lock` はワークスペースで 1 つなので、この検査は「どの exe に入ったか」までは見分けない。
- `xtask/src/check/product.rs` には、「LM の子の起動は `stack/src/lm.rs` の 1 か所だけ」をソースで確かめる検査（`lm_spawn_single`）がある。同じ形で「通信の呼び出しは 1 か所だけ」を書ける。
- `scan-product`（`xtask/src/scan.rs`）は、開発用の文字列、llama.cpp の印、動的な C ランタイムを見る。通信の DLL（`ws2_32` など）の検査は、まだ入っていない（ADR 0066 §6.2 にはある）。
- 低い整合性で子を起動するコードは `lm/src/host/os/windows.rs` にある（LM の子のため）。
- 設定画面の核（`windows/settings`）は、書き出しと取り込みのパスを `shell-core::bridge::check_transfer_path`（絶対パスだけ、ネットワークとデバイスの場所を拒む）で確かめてから、パスだけを設定の口に送る（B-2）。
- `resource` には、manifest の型（`Origin` は `Export`、`Release`、`Local`）、束の検証 `verify_bundle`、既知のモデルの表 `KNOWN_MODELS` がある。`Origin::User` はまだ無い（ADR 0060 §6 で足す予定）。
- リポジトリの直下の `data/` は辞書などのデータの置き場なので、crate のディレクトリには使えない。
- `deny.toml` の `[graph] targets` は `aarch64-apple-darwin` と `x86_64-pc-windows-msvc` だけ。CI の `rust` の job（fmt、clippy、cargo test）は ubuntu で流れる。
- TLS の crate（手元の crates.io の写しで見た）:
  - `ureq` 3.5.x の既定は `rustls`（暗号は `ring`）と `webpki-roots`（Mozilla の根の証明書を埋め込む）。`native-tls-no-default` の feature にすると、TLS は `native-tls`（Windows は `schannel`、macOS は `security-framework`、そのほかの OS は `openssl`）になり、根の証明書を埋め込まない。
  - `RootCerts` の既定は `WebPki`。`native-tls-no-default` では、`RootCerts::PlatformVerifier`（native-tls が OS の根を使う）を明示しないと、根の証明書が無い設定になる。
  - プロキシ: `ureq` の既定の設定は、`ALL_PROXY`、`HTTPS_PROXY`、`HTTP_PROXY`（と小文字）と `NO_PROXY` を読む。Windows の OS の登録値（レジストリ）は、`win-system-proxy` の feature を入れたときだけ読む。

## 2. 決定の要約

| 項目 | 決定 |
| --- | --- |
| 形 | 別の実行ファイル `ikari-data`（Windows は `ikari-data.exe`、インストール先の固定の場所。macOS は `ikari.app/Contents/Helpers/`）。1 回の起動で 1 つの仕事をして終わる。常駐しない |
| 起こすもの | 設定画面の核（Rust）だけ。利用者が手で打つコマンドとしても動く。エンジン、TIP、殻、LM の子は起こさない |
| 通信の手段 | `ureq` 3.5.x（`default-features = false`、`features = ["native-tls-no-default"]`）。TLS は OS のもの（Windows は SChannel、macOS は Security.framework）を、実行時に `TlsProvider::NativeTls` と `RootCerts::PlatformVerifier` で明示する。依存に入れるのは `ikari-data` の crate だけ（§9） |
| 通信するプロセス | Windows は、`ikari-data` が自分を**低い整合性**で起こした子（`ikari-data transfer`）だけ。macOS は、App Sandbox の中の `ikari-data` 自身（§3） |
| 仕事 | (a) プリセットのモデルの取得、(b) 自由指定の URL のモデルの取得、(c) アプリの版の確認。ファイルからの取り込みは持たない（§7） |
| 書く先 | 自分の作業の場所（`fetch/`）と、`inbox/<名前>.partial/` → `inbox/<名前>/` の rename だけ |
| 確かめ | `ikari-data` は、写すときに数えた sha256 と大きさを確かめる。取り込みの判断（検証、試しの採点、入れ替え）はエンジン（0058、0060） |
| 版の確認 | GitHub の「最新の Release」の転送先のタグを読むだけ。取り込みも導入もしない。古い版を勧めない |
| 失敗 | 全部の失敗で、作業の場所と `.partial` を消し、理由の語を 1 つ返す。自動では取り直さない |
| 設定画面との口 | 設定画面の核の C の ABI（開始、進み具合、取り消し）。核は `ikari-data` の標準出力の行を読み、文は核が作る |

- **現状は取得（上の表のとおり）。#242 の確認後に同梱へ変える決定（ADR 0060 更新、2026-11-19）。** 通常版はモデルと辞書を同梱し、ikari-data の仕事は自由指定の URL のモデルの取得と版の確認になる。オフライン版は `ikari-data` と `ureq` を持たない。同梱の実装が済むまで、この ADR の範囲が今の事実。

## 3. プロセスの形と権限

### 3.1 起動の連鎖
- 設定画面の核（`ikari_settings_core.dll`。設定画面のプロセスの中の Rust）が、自分のモジュールの場所から決めたインストール先の `ikari-data.exe` を、絶対パスで起動する（R2。PATH から探さない。シェルを通さない）。引数は全部核が作る。
- Windows では、`ikari-data` が、自分の実行ファイル（`current_exe`）を、取得の子だけの AppContainer（起こせなければ低い整合性。§3.2）で `ikari-data transfer <引数>` として起動する。通信と TLS は、この子の中でだけ行う。macOS では子を作らず、`ikari-data` 自身が通信する（Sandbox の子は親と同じ Sandbox に入るので、分けても守りが増えないため）。
- 連鎖は「設定画面 → `ikari-data` → `ikari-data transfer`（Windows）」の 1 本だけ。ほかの経路（エンジン、スケジューラ、Run キー、ファイルの関連付け）では起きない。OS の curl などの外の実行ファイルは起動しない。
- 同時に走る取得は 1 つ。作業の場所の錠（`fetch/.lock`。排他で開く）で受ける。取れなければ `busy` で終わる。

### 3.2 Windows の権限
| プロセス | 整合性 | ほかの絞り | 理由 |
| --- | --- | --- | --- |
| `ikari-data` | 中（設定画面と同じ。`asInvoker`） | 高い権限で起動されたら何もせずに終わる（A-3 と同じ規則）。設定画面の核が Job Object に入れる（止めた状態で作り、入れてから動かす。閉じたら終わる。子の数の上限は、`ikari-data` と `transfer` の 2。UI の制限は `transfer` と同じ）。渡すハンドルは標準出力のパイプだけ（`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`。S5）。起動の作りは `transfer` と同じ `ikari-sandbox`（§17） | `inbox` に書くため。受け取ったバイト列の解釈（TLS、HTTP）をしないので、中の整合性で動かしても、外から来たものを読むのは自分の作業の場所のファイルを写すところだけ |
| `ikari-data transfer` | **AppContainer**（取得の子だけのプロファイル `io.github.ikari-zen.data-transfer`。LM の子のプロファイルとは分ける。能力は `internetClient` だけ。トークンの整合性は低。0078 §3.2）。起こせないときは**低**に落とす（下） | `ikari-data` が作る Job Object の中（`ikari-data` が終われば終わる。子の数の上限 1、UI の制限は LM の子と同じ）。継承するハンドルは、標準出力のパイプと、書く先の追記だけの写しの 2 つだけ（`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`。標準入力と標準エラーは渡さない。0078 §3.1）。読み取りの許可（ACE）は 1 つも足さない | ネットワークから来たものを解釈するのはこの子。AppContainer の中では、利用者のファイルとデータの場所を読めず、書けない。届くのは、通信と、継承した 1 つのハンドルと、すべてのアプリのパッケージに開いた場所（Program Files、システムのフォルダ、HKCU\Software の一部など。0072 §12.4 と同じ）だけ。低に落としたときは、書けなくするだけで、読めなくはならない（security-design §4） |

- **Job の UI の制限**（2026-11-17 リード決定）: `ikari-data` の Job にも、`transfer` の Job と同じ UI の制限（クリップボードの読み書き、ほかのプロセスの窓、デスクトップの切り替え、画面とシステムの設定の変更、グローバルなアトム、サインアウト）をかける。理由: `ikari-data` に画面は無く、失うものが無い。中の整合性で動く分、ほかのアプリの窓や設定に触れられないようにしておく。`ikari-sandbox` はどの子にも必ずかける（選べる形にしない）。
- `transfer` は、自分の整合性が低でなければ、何もせずに `refused` で終わる（人が手で打っても、中の整合性では通信しない。検査は §10）。親は、どの檻で起こしたかを引数 `--box appcontainer|low` で知らせ（無ければ `low`）、`appcontainer` のときは、子は自分のトークンが AppContainer であること（`TokenIsAppContainer`）も確かめる。違えば `refused`（0078 §3.2 の 4）。
- **AppContainer で起こせないとき**（プロファイルが作れない、`CreateProcess` が失敗する）は、低い整合性で起こし直して取得を続ける（0078 §3.2 の 5。2026-11-19 ユーザー決定）。`ikari-data` は、子を起こしたあとに `box low` の行を出し（AppContainer なら `box appcontainer`。§8.1）、設定画面の核は取得の様子の文に「隔離なし」の印を付ける（`windows/settings/src/text.rs` の `FETCH_UNBOXED`）。AppContainer の中で起きた失敗（通信できない、など）では落とさない（理由の語で終わる）。
- プロファイルは `ikari-data`（中）が、無ければ作る（利用者ごと。名前は `resource::fetch_report::TRANSFER_PROFILE_NAME`）。消すのはアンインストールのとき（インストーラの helper の `cage-profile-delete`。LM の子のプロファイルと一緒に消す。消えるのは、削除を動かした利用者の分だけ）。
- **子は場所を一つも持たない**（2026-11-19 改め。[ADR 0078](0078-fetch-child-handle-and-appcontainer.md) §3.1）。作業の場所は、両 OS で `<データの場所>\fetch\`（中の整合性。macOS と同じ）。`LocalLow` は使わない。
  - `download.bin` は親（`ikari-data`）が、作業の場所を持ったあとに新しく作り（読み書き）、そのハンドルを最後まで持つ。パスで開き直さない。
  - 子に渡すのは、そのハンドルから `DuplicateHandle` で作った `FILE_APPEND_DATA | SYNCHRONIZE` だけの写し。値は引数 `--out-handle <十進>` で知らせる（`transfer` は `--out` を受けない）。子は読めず、切り詰められず、位置を選べない。子は `sync_all` を呼ばない。
  - 親は子の行とは別に、自分のハンドルでファイルの大きさを一定の間隔（仮置き。`limits::SIZE_WATCH_INTERVAL`）で読み、上限を超えたら Job を閉じて `too-large` で終わる。子が終わったら、親が自分のハンドルで `FlushFileBuffers` してから、先頭から読んで写す。最後に `download.bin` は親が消す（`cleanup`）。
- **子が書いたファイルは信じない**。`ikari-data` は、それを rename せず、`inbox/<名前>.partial/` に**写しながら sha256 を数える**。確かめるのは、写した先のバイト列の sha256。rename にしない理由は、子に渡した写しが子の終了のあとも残りうる（推測）ので、数え終わるまで動かないとは言えないため（0078 §3.1 の 5）。親は読むときにファイルの位置を使わない（写しと位置を分け合うため）。
- 写す分だけ、ディスクを一時に 2 倍使う（§8.3）。
- ファイアウォールの送信ブロックの規則は、`ikari-data.exe` にはかけない（0071 §3.1。`transfer` は同じ exe なので、同じ扱い）。設定画面の exe にはかける（0066 §6.2 の L2）ので、設定画面のプロセス自体は通信できないまま。
- 低い整合性で、OS の証明書の確かめ（SChannel）が動くことは、VM で確かめた（§16。そのときの書く先は `LocalLow` だった）。AppContainer の中で動くか（名前の解決、プロキシ、証明書の確かめ）は、まだ確かめていない（0078 §6 の段 2 の VM の項目）。継承した追記だけのハンドルに低い整合性の子が書けること、読めないこと、切り詰められないこと、位置を選べないことは、Windows の CI のテスト（`fetch/src/spawn_windows/tests.rs`）で確かめる。AppContainer の子について同じことと、パスで利用者のファイルを開けないことを確かめるテストも同じ場所にあるが、GitHub Actions の runner では AppContainer のプロファイルが作れないので飛ばし、VM で流す。ふだんのログオンでの確かめは 0078 §6 の VM の項目。

### 3.3 macOS の権限
- `ikari-data` は App Sandbox に入れ、`network.client` と、データの場所の例外だけを付ける（0071 §3.5）。作業の場所は `<データの場所>/fetch/`。
- **実行ファイルに Info.plist を埋める**（リンクのときに `-sectcreate __TEXT __info_plist`）。埋めないと、Sandbox の初期化で落ちる（§16。ad-hoc でも Apple Development の署名でも同じ）。`CFBundleIdentifier` は署名の識別子と同じにする。
- 設定アプリ（Swift）は、`shell-ffi` の関数を通して `ikari-data` を起動する。起動のコード（引数の組み立て）は Rust。Swift は関数を呼ぶだけ。

### 3.4 環境変数
- Windows の `ikari-data` が `transfer` に渡す環境は、プロキシの変数（`ALL_PROXY`、`all_proxy`、`HTTPS_PROXY`、`https_proxy`、`HTTP_PROXY`、`http_proxy`、`NO_PROXY`、`no_proxy`）と、OS が動くのに要るもの（`SystemRoot`、`TEMP`、`TMP`）だけ。ほかは消す。
- 通信の設定で環境変数から読むのは、プロキシだけ。これは R5（製品は環境変数で挙動を変えない）の、文書に書いた例外（0071 §3.4）。`ureq` の既定に任せず、`ikari-data` のコードで `Proxy::try_from_env()` を明示して渡す（読む変数を 1 か所で見えるようにするため）。
- **Windows の OS のプロキシの登録値（設定アプリの「プロキシ」、レジストリ）は読まない**（`win-system-proxy` を入れない）。プロキシの下の利用者は、環境変数を設定する。この限界は、README と設定画面の取得のところに書く（ユーザー決定。§15）。
- `ikari-data` 自身は、プロキシのほかに、環境変数で挙動を変えない（R5）。

## 4. 入力（何を取りに行くか）

### 4.1 引数（`ikari-data` のコマンドの形。人が打っても、核が起動しても同じ）
```
ikari-data model fetch --preset <id>
ikari-data model fetch --url <https の URL> --format <形式> [--sha256 <64 桁の 16 進>] [--name <表示名>]
ikari-data version-check
ikari-data cleanup          # 落ちたあとに残った作業の場所と .partial を消すだけ（通信しない）
```
- Windows の `ikari-data transfer …` は、`ikari-data` が自分の子にだけ使う内部の形（使い方の表示には出さない。§3.2 の整合性の検査がある）。
- データの場所を変える引数は持たない（製品は `--data-dir` を受けない。0067）。
- 知らない引数、重なった引数は、何もせずに `usage` で終わる。

### 4.2 プリセット（既知のカタログ）
- 表はアプリに埋めたもの（`resource::MODEL_PRESETS`。ADR 0060 §5。今の `KNOWN_MODELS` を広げる）。1 行に、ID、ファイルの名前、sha256、大きさ、URL（https）。
- **表は遠くから取らない**。新しいプリセットは、アプリの更新で届く（0060 §5）。そのため、表の署名と、表の巻き戻しの問題は起きない（表は実行ファイルの中にあり、実行ファイルの真正性は ADR 0069 で守る）。
- プリセットの取得は、表の sha256 と大きさに合わなければ、必ず捨てる（警告で通す道は無い）。

### 4.3 自由指定の URL（ADR 0060 の段階 5）
- Rust で厳しく読む（URL の crate は足さない）: `https://` で始まる。ホストは ASCII だけ（国際化ドメインは `xn--` の形で入れてもらう）。利用者名とパスワードを含まない（`@` を拒む）。長さは 2048 文字まで。空白と制御文字を拒む。
- 確認の画面に出すホストは、Rust が読んだ ASCII のホストそのもの（0060 の「確認の画面でホストを出す」。見た目の似た文字にだまされないため ASCII に限る）。
- sha256 を指定したら照らす。無ければ、核が警告の文を出し、利用者が確かめてから起動する（0060 のユーザー決定）。`ikari-data` は、sha256 なしで取得したことを manifest の `source` に残す。
- 警告の文には、OS を分けずに「sha256 なしでは、通信の経路だけが頼り」と書く。sha256 なしで取ったときは、取得のあとに数えた sha256 を確認の画面に出す（§18.1。2026-11-17 ユーザー決定）。
- `--format` は 0060 の `PromptFormat` の語。`ikari-data` は GGUF の中身を読まない（形式を推定しない。読むのは檻の中の子だけ）。

### 4.4 `ureq` の呼び出し方（Rust が全部決める）
- 通信の設定は、`fetch/src/transfer.rs` の 1 つの関数（`agent(…)`）だけで作る。決めること:
  - TLS: `TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier)`。`disable_verification` は使わない。独自の根の証明書（`RootCerts::Specific`）、クライアント証明書は渡さない。
  - **https だけ**: `https_only(true)`。始めの URL も、転送先も、http なら失敗にする（§16 で、http の URL と、https から http への転送が、どちらも拒まれたのを確かめた）。
  - 転送: `max_redirects(<N>)`、`max_redirects_will_error(true)`（上限を超えたら失敗）。転送先のホストは縛らない（配布元は別のホストに転送する。0071 §3.2）。版の確認だけは `max_redirects(0)`（§6）。
  - 締め切り: `timeout_connect(<接続の秒>)`、`timeout_resolve(<接続の秒>)`（名前の解決は接続の締め切りに含まれないため、同じ値を使う）、`timeout_per_read(<無通信の秒>)`（読むたびに数え直す。止まった通信を切る）。全体の締め切りは置かない（大きいモデルの取得を、回線の速さで落とさないため）。
  - 要求: `user_agent("ikari-data")`（版を付けない）。ほかのヘッダ、Cookie、認証は付けない（`cookies` の feature を入れない）。圧縮の feature（`gzip`、`brotli`）を入れない（受けたバイト列をそのまま数えるため。要求に `Accept-Encoding` を付けない）。
  - プロキシ: §3.4。
- **取得先のホストの検査**: プリセットは、始めの URL のホストが表のホストと同じであることを、要求の前に確かめる（表の書き損じで別のホストに行かないため）。自由指定は §4.3。どちらも、結果の確かめは sha256（プリセットは必ず）で受ける。
- 本文: `body_mut().with_config().limit(<大きさの上限>)` で読む。上限を超えたら `too-large`。応答の `Content-Length` が上限を超えていれば、読む前に `too-large`。プリセットは、`Content-Length` が表の大きさと違えば、読む前に `checksum`。
- 進み具合は、読んだバイト数を数えて、標準出力の行で出す（§8.1。作業の場所のファイルを見回らない）。
- 状態の番号が成功でない応答（4xx、5xx）は、`ureq` の既定（`http_status_as_error`）のまま失敗にする。
- 値（N、秒、上限）は仮置き（Q3。§15）。大きさの上限は、0060 §4 のモデルの上限と同じ値を使う（未定。それまでは仮置き）。
- 使わないもの: 続きからの取り直し（`Range`）、`socks-proxy`、`win-system-proxy`、`platform-verifier`（rustls 用）、`json`、`multipart`、`charset`。検査（§10）で止める。

## 5. 出力（どこに置き、だれが取り込むか）

### 5.1 流れ（モデル）
1. `ikari-data` が作業の場所に `download.bin` を作ってハンドルを持ち、通信するプロセスがそこに落とす（§4.4）。Windows は `transfer` が、継承した追記だけの写しに書く。macOS は `ikari-data` が自分のハンドルに書く（§3.2。0078 §3.1）。進み具合は、読んだバイト数を標準出力の行で知らせる（§8.1。Windows は `ikari-data` が子の行を読み、確かめてから核に流す。大きさの上限は、親が自分のハンドルでも見張る）。
2. 落とし終えたら、`ikari-data`（Windows は中の整合性の親）が、持っているハンドルで先頭から読み（パスで開き直さない）、`inbox/<名前>.partial/<ファイル名>` に写しながら sha256 と大きさを数える。プリセットは表と、自由指定は `--sha256` があれば照らす。合わなければ消して終わる。
3. `resource` の関数で、束の `manifest.json` を書く（`kind = model`。プリセットは `origin = preset`、自由指定は `origin = user` と `profile { format }` と `source { url, sha256_given }`）。
4. `inbox/<名前>.partial/` を `inbox/<名前>/` に rename する。作業の場所を消す。`done` の行を出して終わる。
5. **ここから先はエンジン**（0058 §8.1 の見回り）。束の検証（0058 §6.1）→ モデルの検査（0060 §4。語彙の検査と試しの採点は、llama を持つ檻の中の子 `ikari-lm-host` に頼む。製品のエンジンには llama.cpp が無いため。0072 §5）→ `models/` への入れ替え（0058 §7.2）→ `models/MANIFEST.json`。結果は `inbox/status.json`。
6. 取り込んでも、使うモデル（`lm.model`）は変えない（0060 §2）。利用者が設定画面で選ぶと、LM のワーカーが作り直され、読み込みの前に `models/.loading` を書く（0060 §4。落ち続けを防ぐ）。

- 名前（束のディレクトリ、ファイル）は、プリセットは表の名前、自由指定は Rust が作る名前（`user-<sha256 の先頭 12 桁>`。ファイルは `<名前>.gguf`）。URL、応答のヘッダ、`--name` からは作らない（`--name` は表示にだけ使い、manifest に文字として残す）。
- `resource` に足すもの（契約）:
  - `Origin::Preset`、`Origin::User`。
  - `stage_model_bundle(src: &mut impl Read, inbox: &Path, plan: &StagePlan) -> Result<StagedBundle, StageError>`: 写しながら数え、照らし、manifest を書き、rename するまで。`ikari-data` と、§7 のファイルからの取り込みが同じ関数を使う。
  - `StagePlan { name, file_name, kind, origin, expected: Option<(Sha256, u64)>, profile: Option<ModelProfileRef>, source: Option<Source>, max_size }`。

### 5.2 辞書（1.0.0 より後。0071 Q3。形だけ決める）
- 作者の束（`origin = release`）を配り始めるときに、`ikari-data bundle fetch --release <タグ>` を足す。流れは 5.1 と同じで、エンジンは 0058 §6.1 の検証と、B-5 の辞書の検査（キーの数の上限、索引の検証）と、試しの組み立てをしてから入れる。
- そのとき、`origin = release` の束には署名を必須にする（0058 §6.2。minisign の形。鍵はアプリに埋める）。**巻き戻しの拒否**は、エンジンが manifest の `created` と `release` を、入っている版と比べて行う（0058 §6.3。古い束は拒む。戻すのは「前の版に戻す」だけ）。`ikari-data` は判断しない（書き手と判断を分けるため）。

## 6. 版の確認（ADR 0070 Q3）

- `ikari-data version-check` は、`https://github.com/ikari-zen/ikari-ime/releases/latest` に、転送を追わずに 1 回だけ要求し（`max_redirects(0)`。本文は `limit(<版の確認の上限>)` で読んで捨てる）、応答の `Location` の末尾のタグを読む。URL は実行ファイルに埋めた固定のもの。`Location` は `https://github.com/ikari-zen/ikari-ime/releases/tag/` で始まるものだけを受ける。
- タグは `v<数>.<数>.<数>` だけを受ける（ほかは `bad-reply`。確認できない側に倒す）。各数字には桁の上限を置き、超えたら `bad-reply`（比べるときの桁あふれを避ける。§18.2）。`latest 0.3.1` の行を出して終わる。
- 比べるのは設定画面の核（自分の版は `CARGO_PKG_VERSION`）。表示は 3 つだけ: 「新しい版があります（0.3.1）」、「最新です」、「この版のほうが新しい（開発版）」。**取った版が今より古くても、古い版を勧めない**（巻き戻しの拒否。0070 §3 の 4 で、古い版を入れるのはインストーラの確認つきだけ）。
- 取得も導入もしない。「Releases を開く」のリンクは固定の URL で、押したときだけブラウザで開く（応答の中身を URL にしない）。真正性の確かめ方は ADR 0069 の案内に任せる。
- 署名は付けない。表示するのは版の番号だけで、だまされても起きるのは「表示が違う」までのため（取得と導入に結びつけない限り）。TLS は OS の TLS が確かめる。
- 自動では確認しない（起動のとき、定期、設定画面を開いたとき、のどれもしない）。
- 2026-11-17 の時点で、このリポジトリの「最新の Release」の転送先は、アプリの版ではなくデータのタグ（`webwords-…`）だった（§16）。このままでは `bad-reply` になる。**決定: データの Release は「最新」にしない運用にし、アプリの Release は明示して最新にする**（§18.2。2026-11-17 ユーザー決定）。

## 7. 自由指定のモデルの境界（ADR 0060 の段階 3〜5）と、ファイルからの取り込み

- `ikari-data` が持つのは URL からの取得だけ。**ファイルからの取り込みは `ikari-data` に置かない**（0060 §6 の `ikari-data model import --file` を改める。2026-11-17 ユーザー決定）。理由: 通信してよい部品に、利用者の任意のファイルを読む力を足さない（権限を 1 つに絞る）。B-2 の書き出しと取り込み（パスを流す）と同じ流儀にそろう。
  - Windows: 設定画面の核が、OS のファイルを選ぶ窓で得たパスを `check_transfer_path` で確かめ、設定の口に `ImportModelFile { path, format, sha256 }`（`bridge.proto` に足す。パスだけ）を送る。engine-host が同じ検査をして読み、`resource::stage_model_bundle` で inbox に束を作る。
  - macOS: エンジンは Sandbox で利用者のファイルを読めないので、設定アプリ（Sandbox の外）が `shell-ffi` の関数を呼び、同じ `stage_model_bundle` で inbox に束を作る。
- そのあとは URL のときと同じ（§5.1 の 5、6）。sha256 なしは警告で許可（0060）。試しの採点に通らなくても、警告を出して使えるようにする（0060 Q2）。
- 檻（0060 §4.1、0072）: モデルの中身を読むのは `ikari-lm-host` だけ。`ikari-data` と engine と設定画面は、モデルのバイト列を写して数えるだけで、解釈しない。自由指定のモデルを出すのは段 2 のある OS だけ（0072 Q1）。設定画面の核は、段 2 が無いときは自由指定のボタンを出さない（判断は核）。
- モデルが自分で通信したりファイルを触ったりできないことは、檻（段 2）で受ける。`ikari-data` の側では何もしない。

## 8. 失敗のふるまい

### 8.1 標準出力の行と終了コード（`resource::fetch_report` に型と読み書きを置く。核と `ikari-data` が共有する）
```
progress <受けたバイト> <全体のバイト|->
box <appcontainer|low>
done <束の名前> <sha256>
latest <版>
error <理由の語>
```
- 1 行は 256 バイトまで。核は、知らない行、長すぎる行、`done`・`latest`・`error` のあとの行を捨てる。終了コードは 0（成功）と 1（`error` を出した）と 2（`usage`）。文は核が理由の語から作る（`windows/settings/src/text.rs`）。
- Windows の `transfer` も同じ行を出す（`done` の代わりに、受けた大きさを `progress` の最後の行で出して 0 で終わる）。`ikari-data` は、子の行を同じ規則で読み、子の `progress` の数を信じず、写すときに数え直す。
- `box` は、Windows の `ikari-data` が、取得の子をどの檻で起こしたかを、子を起こしたあとに 1 回だけ出す（§3.2。0078 §3.2 の 5）。終わりの行ではない。`low` なら、設定画面の核が「隔離なし」の印を出す。子が出した `box` の行は `ikari-data` が捨てる（核に流すのは、親が自分で出したものだけ）。macOS は出さない。

### 8.2 理由の語（`FetchError`）
| 語 | いつ（`ureq::Error` との対応） | 片づけ |
| --- | --- | --- |
| `cancelled` | 利用者が取り消した（核が Job Object を閉じる） | 次の起動の `cleanup` か、次の取得の始めに消す |
| `network` | つながらない、名前が引けない、プロキシが断った（`Io`、`HostNotFound`、`ConnectionFailed`、`ConnectProxyFailed`、`Timeout(Connect)`、`Timeout(Resolve)`） | 作業の場所を消す |
| `tls` | 証明書の確かめ、TLS の交渉に失敗（`NativeTls`、`Tls`、`TlsRequired`） | 同じ |
| `http-status` | 成功でない応答（`StatusCode`）、転送の回数を超えた（`TooManyRedirects`、`RedirectFailed`）、https でない転送先（`RequireHttpsOnly`） | 同じ |
| `stalled` | 決めた時間、受けるバイトが増えない（`Timeout(PerRead)`） | 同じ |
| `too-large` | 大きさの上限を超えた（`BodyExceedsLimit`、`Content-Length`） | 同じ |
| `checksum` | sha256 か大きさが合わない（壊れた、途中で変わった、改ざん） | 作業の場所と `.partial` を消す。自動で取り直さない |
| `disk-full` | 始める前の見積もり、または書くときに足りない | 同じ |
| `bad-url` | §4.3 に合わない、プリセットのホストが表と違う | 何も作らない |
| `bad-reply` | 版の確認の応答が読めない（`Location` が無い、形が違う） | 同じ |
| `tool` | 通信の部品が動かない（Windows で取得の子を、AppContainer でも低い整合性でも起こせない。対象外の OS で作ったビルド） | 同じ |
| `busy` | ほかの取得が走っている（作業の場所の錠が取れない）。または、同じ名前の束が `inbox/<名前>/` にすでにある（ほかの取得が先に入れ、エンジンがまだ取り込んでいない。`StageError::Exists`） | 何もしない（後者は取得のあと、写す前に分かる。作業の場所は消し、入っている束には触れない） |
| `refused` | 高い権限で起動された。`transfer` が低い整合性でない | 何もしない |
| `internal` | 上のどれでもない（`ureq::Error` の新しい種類を含む。`#[non_exhaustive]` のため） | 作業の場所と `.partial` を消す |

### 8.3 決まりごと
- 途中で切れたもの（落ちた、電源が切れた）は、`.partial` で終わるディレクトリのまま残り、エンジンは見ない（0058 §4.3）。次の取得の始めに `ikari-data` が消す（`cleanup` と同じ処理）。
- 続きからの取り直し（Range）はしない。最初から取り直す（単純さと、途中のファイルを信じないため）。
- ディスクの見積もり: プリセットは、表の大きさの 2 倍（作業の場所と写し）と余裕。自由指定は大きさが分からないので、始める前は上限の 1 倍と余裕、写す前にもう一度、受けた大きさと余裕で見る。余裕の値は仮置き（Q3）。
- `checksum` のとき、設定画面は「取ったファイルが期待したものと違いました」とだけ出し、取り直しは利用者のボタンだけ。
- エンジンの取り込みの失敗（束の検証、試しの採点）は、今までどおり `inbox/status.json` と `inbox/rejected/`（0058 §8.3）。設定画面は、取得の結果（`done`）のあと、その状態を設定の口で読んで出す。

## 9. 通信の手段（TLS の選定。2026-11-17 ユーザー決定）

- **決定: OS の curl の起動をやめ、`ureq` 3.5.x を `default-features = false, features = ["native-tls-no-default"]` で `ikari-data` の crate にだけ入れる。** 実行時に `TlsProvider::NativeTls` と `RootCerts::PlatformVerifier` を明示する（§4.4）。0071 Q2（OS の curl）を改める。
  - TLS の実装は OS のもの（Windows は SChannel、macOS は Security.framework）。TLS の修正は OS の更新で届く（0071 §3.4 の理由 (1) のうち「修正は OS で届く」は保てる）。ikari の依存に入るのは、HTTP の読み書き（`ureq`、`ureq-proto`、`httparse`）と、OS の TLS への薄い橋（`native-tls`、`schannel`、`security-framework`）。
  - 入れないもの: `rustls`、`ring`、`aws-lc-rs`、`webpki-roots`、`webpki-root-certs`、`openssl`。ISC と CDLA の許可は足さない。入る crate のライセンスは、どれも MIT か Apache-2.0（§16）。
  - 証明書の固定（ピン留め）はしない。配布元（Hugging Face、GitHub）は証明書と配信の先を替えるので、固定すると取得が止まる。中身の真正性は sha256 で受ける。
  - Windows 10 で TLS 1.2 までしか使えないことは許す（ユーザー決定）。
- 取ったときの理由（curl をやめる）: 0076 の初版で curl に引数を渡す形にしたが、版によって引数の効き方が変わること（`--max-filesize` など）、`.curlrc` と環境変数で振る舞いが変わることを、引数と環境の消し方で受けるしかなかった。`ureq` にすると、設定がコードの 1 か所で決まり、単体テストで確かめられる。
- 対象外の OS（Linux など）: `ureq` は `[target.'cfg(any(windows, target_os = "macos"))'.dependencies]` に置き、ほかの OS では通信のコードを持たないビルドにする（`transfer` は `tool` で終わる）。CI の `rust` の job が ubuntu で流れ、`native-tls` が Linux では `openssl-sys`（ビルドに OpenSSL が要る）を引くため。
- 検査の書き方:
  - `deny.toml`: `ureq` は `wrappers = ["ikari-data"]`、`native-tls` は `wrappers = ["ureq"]`、`schannel` と `security-framework` は `wrappers = ["native-tls"]` にする。`webpki-roots`、`webpki-root-certs`、`ring` を禁止の一覧に足す（`rustls`、`openssl`、`openssl-sys` は今のまま禁止）。`[graph] targets` は今の 2 つのまま。
  - `supply.rs`: `Cargo.lock` は対象の OS を区別せず、`native-tls` の Linux 向けの依存（`openssl` など）の名前も載る（§16）。そのため、名前を `Cargo.lock` から拾う形をやめ、配る 2 つの対象ごとに `cargo tree -e normal --target <対象>`（または `cargo metadata --filter-platform`）の結果で、(1) `NET_CRATES` が `ikari-data` の下にしか無いこと、(2) 禁止の追加分（`ring`、`webpki-roots`、`webpki-root-certs`、`rustls`、`openssl`、`openssl-sys`）がどこにも無いことを見る。`NET_CRATES_ALLOWED` は、名前の一覧から「`ikari-data` の下に限る」の規則に替える。
  - `scan-product`: `ikari-data` だけを、通信の DLL の検査から名前で外す（§16 で、`WS2_32.dll`、`secur32.dll`、`crypt32.dll` を取り込むのを見た）。
- 増える大きさ: スパイクの exe の全体（release、LTO、strip）で、Windows x64 が 534KiB、macOS arm64 が 657KiB（§16。空の main との差は測っていない）。

## 10. 設計を守る検査（§13 の契約のレーンで入れる）

| 検査 | 置き場所 | 中身 |
| --- | --- | --- |
| 通信の呼び出しは 1 か所 | `xtask/src/check/product.rs`（`lm_spawn_single` と同じ形） | `ureq::` を使うのは `fetch/src/transfer.rs` だけ。ほかの製品のソースに `ureq`、`native_tls`、`std::net` が無い |
| 子の起動は 1 か所ずつ | 同じ | `ikari-data` を起動する関数を呼ぶのは、`windows/settings/src/fetch.rs` と、`shell-ffi` の該当のファイルだけ。`transfer` を起動するのは `fetch/src/` の 1 か所だけ |
| 通信の設定 | `fetch/` の単体テスト | `agent(…)` の設定が、`https_only` が真、`TlsProvider::NativeTls`、`RootCerts::PlatformVerifier`、転送の上限、締め切りの 3 つ（接続、名前の解決、無通信）、UA が `ikari-data` であること。`disable_verification` を呼ぶコードが無いこと（ソースの検査） |
| `transfer` の整合性 | `fetch/` の単体テスト（Windows の CI） | 中の整合性の `transfer` が `refused` で終わる |
| 通信の crate | `deny.toml`、`supply.rs` | §9 の書き方 |
| 通信の DLL | `scan-product`（ADR 0066 §6.2。未実装の分） | 製品の exe と DLL が `ws2_32`、`winhttp`、`wininet` を取り込まない。`ikari-data.exe` だけを外す（Rust の std がほかの exe でこれを取り込むかは Windows の CI で確かめる） |
| 設定画面が通信しない | 実機（L3）と、C# のソースの検査（`HttpClient`、`WebRequest`、`Socket`、`Process.Start` を書かない） | `windows/settings-ui` |
| macOS の権限 | ビルドの検査（0071 §3.5） | `network.client` を持つのは `ikari-data` だけ。`ikari-data` に Info.plist が埋まっていること |

## 11. 推測と、確かめること（VM・実機。`docs/windows-real-device-check.md` に足す）

1. 低い整合性の `transfer` が、プロキシの環境変数（認証のないプロキシ、`NO_PROXY`）で動くこと。プロキシの URL が `http://` でも、`https_only` が要求を止めないこと（スパイクではプロキシを試していない）。
2. ふだんの中の整合性のログオン（デスクトップの設定画面から起こす形）で、継承した書く先のハンドルへの書き込み（2026-11-19 までは `LocalLow` への書き込み。0078 §6）と、Job Object を重ねた形（設定画面の Job の中で、`ikari-data` が `transfer` の Job を作る）が動くこと。スパイクは管理者の SSH（高い整合性）から起こした（§16）。
3. Hugging Face のプリセットの URL が、転送の回数の上限の中で、最後まで取れること（スパイクは先頭の 8MiB まで）。
4. 取得の間の送信の記録（L3）: 送信するのは `ikari-data.exe` だけで、行き先は表のホストと転送先だけ。設定画面、エンジンは送信しない。
5. 取り消し（Job Object を閉じる）で、`transfer` も終わり、作業の場所が次の起動で消えること。ディスクが足りないとき、途中で回線を切ったときの表示。
6. Windows 10（TLS 1.2 まで）で、Hugging Face と GitHub に TLS でつながること。
7. 版の確認: 転送先のタグが読めること。今の「最新の Release」の応答で `bad-reply` になり、設定画面が落ちないこと（§6）。
8. macOS: `Contents/Helpers/` に置き、設定アプリから起動したときに、Sandbox の中で動くこと（スパイクは Terminal から単独で起動した）。

## 12. 検討した案

- **OS の curl を絶対パスで起動する（0076 の初版、0071 Q2）**: ikari の依存に HTTP の crate が入らない。版の差と `.curlrc`、環境変数の効き方を引数で受けることになり、確かめが VM 頼みになる。2026-11-17 に採らないと決めた（§9）。
- **`ureq`＋`rustls`＋`ring`＋OS の証明書の確かめ**: TLS の実装が OS の版に左右されない。TLS の修正を ikari が配ることになり、ISC の許可が要る。採らない。
- **根の証明書を埋め込む（`webpki-roots`）**: 根の更新を ikari が配ることになり、会社のプロキシの独自の根で通らない。採らない。
- **WinHTTP を直に呼ぶ（Windows）**: 依存が増えない。OS ごとに別のコードになり、Windows の OS のプロキシの設定まで効く（R5 の例外が広がる）。低い整合性で `ureq` が動いたので、要らない。
- **低い整合性の子を作らず、`ikari-data` が中の整合性で通信する（Windows）**: 部品が単純になる。ネットワークから来たものを解釈するプロセスが、利用者のファイルに書けるようになる。低い整合性で動いたので採らない（§16）。
- **設定画面の核が直接通信する（`ikari-data` を作らない）**: 部品が 1 つ減る。設定画面の exe にかけるファイアウォールの規則と、「通信してよいのは 1 つ」の検査の文が崩れる。採らない（0071 §4 と同じ理由）。
- **エンジンが取得を頼まれて `ikari-data` を起こす**: 設定の口に取得の要求が増え、エンジンが通信の連鎖の途中に入る（0066 §6.1 の「エンジンは操作があっても出ない」に近づく）。採らない。
- **作業の場所を `inbox` の下に置き、rename で入れる**: 写す分のディスクが要らない。低い整合性の子に書かせるには `inbox` の下を低い整合性に開けることになり、エンジンが自動で取り込む場所に、低い権限のプログラムが書けるようになる。採らない。
- **版の確認に GitHub の API（JSON）を使う**: リリースのノートも取れる。認証なしの回数の制限があり、JSON を読むコードと受ける量が増える。表示するのは版の番号だけなので、転送先を読むので足りる。
- **sha256 の表を遠くから取り、署名で守る**: アプリの更新なしにプリセットを足せる。新しい通信と信頼の根（鍵）と巻き戻しの問題が増える。0060 §5 の「遠くから表を取らない」のまま。
- **ファイルからの取り込みも `ikari-data` に置く（0060 §6 のまま）**: コマンドが 1 つにまとまる。通信してよい部品が利用者の任意のファイルを読むことになる。採らない（§7）。

## 13. 実装の進め方（1 タスク 1 ブランチ。D-1 のあと）

| 段 | 中身 | 持つファイル | モデル |
| --- | --- | --- | --- |
| D-2a 契約 | `fetch/` の crate の骨組み（引数の型、`FetchError`、`transfer` と `agent(…)` のシグネチャとスタブ）、`resource` の `Origin::Preset`・`Origin::User`・`StagePlan`・`stage_model_bundle` のシグネチャ、`fetch_report` の型と読み書き（実装まで。純粋）、設定画面の核の ABI のシグネチャ（`ikari_settings_fetch_start`・`_poll`・`_cancel`・`ikari_settings_version_check_start`）、`bridge.proto` の `ImportModelFile`、§10 の検査（ソースの 2 つと、`deny.toml`・`supply.rs` の書き換え） | `fetch/`、`resource/src/`、`windows/settings/src/abi.rs`・`fetch.rs`（スタブ）、`shell-core/proto/`、`xtask/src/check/`、`deny.toml`、`Cargo.toml` | 決めるのは architect、入れるのはリード |
| D-2b 取得 | URL の読み、`agent(…)` と本文の読み、作業の場所と錠、写しと sha256、`cleanup`。Windows の低い整合性の起動は、`lm/src/host/os/windows.rs` の起動の部分を、`ikari-data` から使える小さな場所に移して共有する（`ikari-data` が `lm` に依存しないため） | `fetch/src/`、移す先 | Sonnet（低い整合性の起動を移すところだけ Opus） |
| D-2c 設定画面 | 核の起動（Job Object）、行の読み、文、取り消し。画面のボタン、確認の窓（ホスト、sha256 なしの警告）、進み具合、取り込みの状態の表示。取得のところに、プロキシは環境変数だけで効くことを書く | `windows/settings/src/`、`windows/settings-ui/` | Sonnet |
| D-2d 版の確認 | `version-check` と、核の比べ方と表示 | `fetch/src/version.rs`、`windows/settings/src/` | Sonnet |
| D-2e インストーラと macOS | `ikari-data.exe` を置く（規則はかけない）、macOS の Helpers と entitlements と Info.plist の埋め込みとその検査、README のプロキシの限界 | `windows/installer/`、`macos/`、`README.md` | Sonnet |

- ファイルからの取り込み（§7）は D-4（自由指定のモデル）に入れる。試しの採点を檻の子に頼む口は D-3・D-4 の契約で決める。
- どのレーンへの依頼にも「直すまで、弱い点の詳細を公開の issue・PR・コミットメッセージに書かない」（0070 §5 の 3）を書く。
- 実機の確認（§11）は `user-needed` の issue にする。D-2b のあと、D-2c と一緒に VM で見る。

## 14. 結果

- 「通信するのは `ikari-data` だけ」が、依存の検査（`ikari-data` の下に限る）、ソースの検査（呼び出しの 1 か所）、OS の規則（設定画面にはかかる）、実機の記録の 4 つで言える。
- ネットワークから来たものを解釈するのは、Windows は低い整合性の `transfer`、macOS は Sandbox の中の `ikari-data`、それと檻の中の LM の子だけになる。中の整合性の `ikari-data` とエンジンはバイト列を数えて写すだけ。
- 失うもの: 写す分のディスク（モデルの大きさの 1 倍、一時に）、続きからの取り直し、HTTP の crate を ikari の依存として追うこと（`ureq` の更新と脆弱性の知らせ。0070 の対応の範囲に入る）、Windows の OS のプロキシの設定（環境変数だけ）。

## 15. ユーザーの決定（2026-11-17。すべて推奨どおり）

- **Q1**: ファイルからのモデルの取り込みを `ikari-data` から外す（§7。0060 §6 の `ikari-data model import --file` を改める）。
- **Q2**: 通信は OS の curl の起動をやめ、`ureq` 3.5.x ＋ `default-features = false, features = ["native-tls-no-default"]` ＋ 実行時に `RootCerts::PlatformVerifier` と `TlsProvider::NativeTls` を明示する（§4.4、§9）。ring、rustls、openssl、webpki、ISC、CDLA を入れない。依存は `ikari-data` の crate だけ（`deny.toml` と `supply.rs` の例外は、その crate の下に限る。`webpki-roots`、`webpki-root-certs`、`ring` の禁止を足す）。Windows のプロキシは環境変数だけ（OS の登録値は読まない。README と設定画面の文言に限界を書く）。Windows 10 の TLS 1.2 止まりは許す。低い整合性・App Sandbox の下で動くかは、先にスパイクで確かめる（§16。動いた）。
- **Q3**: 仮置きの値（転送 5 回、接続 30 秒、無通信 60 秒、版の確認の上限 64KiB、ディスクの余裕 256MiB）は、仮置きのまま実装し、VM の確認（§11）で見直す。モデルの大きさの上限は 0060 §4 の値（未定）に従う。
- 残りだった「最新の Release」がデータのタグを指す件（§6）は、同じ日の 2 回目の決定で決めた（§18.2）。

### （記録）決定の前の問い
- Q1. ファイルからの取り込みを `ikari-data` から外す。推奨: 外す。
- Q2.（初版）OS の curl を続け、通らなければ Windows だけ `ureq`＋`rustls`＋`ring`＋OS の証明書の確かめにする。推奨: この形。→ 決定は上のとおり、OS の TLS の `ureq` に改めた。
- Q3. 仮置きの値を、仮置きで実装して VM で見直す。推奨: そうする。

## 16. 実測（2026-11-17 のスパイク。コードはリポジトリに入れない）

- 作り: `ureq` 3.5.2（`default-features = false, features = ["native-tls-no-default"]`）、§4.4 の設定（`https_only`、転送 5、接続 30 秒、無通信 60 秒、UA `ikari-data`、`NativeTls`、`PlatformVerifier`）の最小の exe。Windows は Mac から `cargo xwin build --target x86_64-pc-windows-msvc`。低い整合性の起動は `lm/src/host/os/windows.rs` と同じ作り（トークンの複製と `WinLowLabelSid`、Job Object の制限、`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`）。
- 環境: Windows 11 Pro（26H2、ビルド 26300）の VM に、管理者の SSH から。macOS 27.2.1（arm64）。プロキシなし。
- Windows の 5 つの形（直に起動、(a) `CreateProcessW` で通常、(b) 低い整合性、(c) 低い整合性＋Job Object、(d) (c) に加えて継承するハンドルを標準出力のパイプだけに絞る）で、結果は同じだった:
  - 整合性: (a) は 0x3000（高。管理者の SSH のため。中の整合性のログオンでは確かめていない。§11 の 2）、(b)〜(d) は 0x1000（低）。低では `%LOCALAPPDATA%` への書き込みが拒まれ（エラー 5）、`AppData\LocalLow` への書き込みは通った。
  - Hugging Face の公開ファイル: 小さいファイル（665 バイト）を最後まで取れた。古い名前の URL からの転送も追えた。大きいファイル（548,105,171 バイト。別のホストへの転送あり）の先頭 4KiB を約 0.5 秒、先頭 8MiB を 1.4〜1.5 秒で取れた。`max_redirects(0)` では 302 が返った。
  - `limit(100)` で 665 バイトの本文を読むと `BodyExceedsLimit(100)` で止まった。
  - GitHub の `releases/latest`（`max_redirects(0)`）: 302 と `Location` を読めた。本文は 0 バイト、約 40 ミリ秒。このリポジトリの転送先はデータのタグ（`…/releases/tag/webwords-20261101`）だった（§6）。
  - 拒まれたもの: 期限切れの証明書、自己署名、ホスト名の不一致、信頼されない根、TLS 1.0 だけのサーバ（どれも native-tls の失敗）、`http://` の URL と、https から http への転送（どちらも `RequireHttpsOnly`）。
- macOS: App Sandbox（`app-sandbox` と `network.client`）を付け、ad-hoc で署名した exe で、Windows と同じ結果だった（`APP_SANDBOX_CONTAINER_ID` が入り、Sandbox の中で動いたことを確かめた）。`network.client` を外すと、名前の解決から失敗した（https のすべての要求）。Info.plist を埋めない exe は、ad-hoc でも Apple Development の署名でも、Sandbox の初期化（`_libsecinit_appsandbox`）で SIGTRAP で落ちた。
- 失効の確かめの扱いは、OS で違った（中身の真正性は、どちらも sha256 で受ける）。Windows の OS の TLS が失効した証明書を通すことの扱いは §18.1。
- 依存（`cargo metadata --filter-platform`）: Windows は `ureq`、`ureq-proto`、`native-tls`、`schannel`、`http`、`httparse`、`der`、`rustls-pki-types` など 19、macOS は `security-framework`、`core-foundation`、`tempfile`、`rustix` など 29。ライセンスはどれも MIT か Apache-2.0（`rustix` は Apache-2.0 WITH LLVM-exception の選択肢つき）。`Cargo.lock` には、Linux 向けの `openssl`、`openssl-sys`、`openssl-probe` も載った（§9 の検査の書き方の理由）。
- Windows の exe が取り込んだ DLL に、`WS2_32.dll`、`secur32.dll`、`crypt32.dll` があった。

## 17. 子の絞った起動の共通化（`ikari-sandbox`。2026-11-17）

- **今**: `windows/sandbox`（`ikari-sandbox`）に、止めた状態で作り → Job Object に入れ → 渡すハンドルを限って動かす起動を置いた。使うのは `transfer` の起動（`fetch/src/spawn_windows.rs`。AppContainer（`Launch::container`。起こせなければ低）、子 1、環境はプロキシと OS の分だけ）と、設定画面の核の `ikari-data` の起動（`windows/settings/src/fetch_os.rs`。中、子 2、環境は受け継ぐ）。どちらも UI の制限をかけ、標準出力のパイプを渡す。ほかに渡すハンドルは、呼ぶ側が `Launch::inherit` で明示したものだけ（2026-11-19。0078 §4.1。今は `transfer` の書く先の追記だけの写しだけ）。取り消しは Job の中をすべて止める（`TerminateJobObject`）。
- 確かめたこと（VM、管理者の SSH。`ikari-sandbox` のテスト）: 子が Job に入っている、低の子の整合性が 0x1000、渡さない標準入力と標準エラーは null か子の新しいコンソール、親が継承の印を付けたままの別のハンドルは子に届かない、子 1 の Job では子がさらにプロセスを作れない（子 2 では作れる）、Job を持つ親が終わると子も終わる、渡したハンドルの継承の印は起動のあとに外れる。
- **入れ子の Job と UI の制限**（VM で見つけた）: UI の制限つきの Job の中のプロセスは、UI の制限つきの別の Job に子を入れられない（`AssignProcessToJobObject` が `ERROR_NOT_SUPPORTED`。外と内のどちらにも、どの 1 つの制限をかけても同じ。外だけ、内だけなら入れられる）。そのため `ikari-sandbox` は、自分の Job がすでに UI の制限を全部かけていれば、新しい Job にはかけない（外の Job の制限が入れ子の中にもかかる前提）。一部だけなら、かけて、入れるところで失敗にする（起動しない側に倒す）。この形で、設定画面の核 → `ikari-data`（子 2、UI の制限）→ `transfer`（子 1、低）の入れ子から、`transfer` が https のファイル（11,358 バイト）を最後まで取れた（`transfer_runs_inside_both_jobs`、手で流すテスト）。
  - **推測（確かめていない）**: 外の Job の UI の制限が、入れ子の内の Job の孫にもかかること（Microsoft の入れ子の Job の説明による）。管理者の SSH（セッション 0）では、確かめの手段にしたクリップボード（整合性でも拒まれ、区別できない）とグローバルなアトム（親が足せない）が使えなかった。ふだんのログオンで確かめる（§11 の 2 と一緒に）。
- **共通化の順の改め（2026-11-19。0078 §3.2 の 3）**: 取得の子の段 2（AppContainer）では、AppContainer の部品（プロファイル、SID、読み取りの許可）を `lm` から `ikari-sandbox` に移し、`lm` は `use` を替えるだけにする。LM の子の起動の本体は触らないので、下の移行を済ませるまで、属性の組み立ては 2 か所になる。この移動のあとは、下の「`lm` の `appcontainer` に残す」を `ikari_sandbox::appcontainer` と読む。
- **LM の子（`lm/src/host/os/windows.rs`）はまだ移していない**。段 2（AppContainer。`lane/d3-win`）が同じ関数（起動の本体と属性の一覧）を書き換えていて、先に移すと衝突するため。段 2 を統合したあとで、次の形で移す:
  - `ikari-sandbox` の `Integrity` に、段 2 の値（AppContainer の SID を借りる形。SID の作成と許可は `lm` の `appcontainer` に残す）を足し、属性の一覧に `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES` を足す（今の `AttrList` の中身）。
  - `lm` の `spawn` は、整合性の決め方（`low_integrity_enabled`、`Cage`、`CageState`）と、渡す 3 つのハンドル（標準エラーは `NUL`）を決めて `ikari_sandbox::spawn` を呼ぶだけにする。`Proc` は `Confined` を包む（`id`、`reap`）。`cmdline.rs` は `ikari_sandbox::cmdline` に替える。
  - 変えないもの: LM の子の起動の入口が `stack/src/lm.rs` の `host_loader(` の 1 か所であること（`xtask` の `lm_spawn_single`。`ikari-sandbox` は `host_loader(` を持たない）、開発用の口が `dev-knobs` の中だけにあること（`ikari-sandbox` は環境変数を読まない。整合性を選ぶのは呼ぶ側）。

## 18. 2 回目の決定（2026-11-17。ユーザー（Fable の指摘つき）。リードの推奨どおり）

### 18.1 Windows の OS の TLS が、失効した証明書を通すこと
- 分かったこと: 失効の確かめの扱いは OS で違った（§16）。Windows の OS の TLS は、失効した証明書を通しうる。
- **決定: 許す。** 自前の失効の確認（OCSP、CRL）は採らない。理由: 通信の行き先と、外から来たものを解釈するコードが増える（「通信してよいのは 1 つ、解釈は低い整合性の子だけ」を細くしておくため）。
- プリセットは表の sha256 で必ず照らす（§4.2）ので、失効の扱いに左右されない。影響を受けるのは自由指定で sha256 なしのときだけ。
- 自由指定（sha256 なし）の警告の文に、このことを書く。文は OS で分けず、「sha256 なしでは、通信の経路だけが頼り」と書く（macOS なら安全と読ませないため）。文は核が作る（§8.1 と同じ置き場所）。
- sha256 なしで取ったときは、取得のあとに数えた sha256 を確認の画面に出す（利用者が配布元の値と比べられるように）。manifest の `source` に残すのは §4.3 のとおり（決定済み）。

### 18.2 「最新の Release」がデータのタグを指すこと（§6）
- **決定: データの Release は「最新」にしない運用にする。アプリの Release は、作るときに明示して最新にする。** チェックリストの文だけにせず、作るスクリプトと手順書に書く。
  - データの Release を作るスクリプト（`cargo xtask webwords publish`。`xtask/src/webwords/publish.rs` の `release_args`）は、`gh release create` に `--latest=false` を付ける（実装の課題。§18.3）。手順書は [webwords-update.md](../webwords-update.md) §5 に書いた。
  - アプリの Release は `--latest` を明示して作る（[publish-checklist.md](../publish-checklist.md) §4 に書いた）。
- 版の確認は `v<数>.<数>.<数>` の形のタグだけを見る。ほかの形（データのタグ、`v` のない形、`-rc` などの付いた形）は `bad-reply` にする（「最新です」と出さず、確認できない側に倒す）。
- 版の各数字に桁の上限を置き、超えたら `bad-reply`（比べるときの桁あふれを避ける）。上限の値は契約のレーンで決め、PR に書いてユーザーに確かめる（今は決めていない）。
- 公開前チェックリスト（[publish-checklist.md](../publish-checklist.md) §4）に足した: アカウントの 2 要素認証（パスキーか物理キー）、`v*` と `webwords-*` のタグを守る ruleset、immutable releases（GitHub 側にこの機能があるか、どう設定するかは未確認なので「要確認」）、データの Release を最新にしないこと、アプリの Release を最新にすること。
- **既知の限界**: アプリの Release を作れるアカウントを取られると、偽の `v9.9.9` を出せて、設定画面が「新しい版があります」と勧めうる。署名なしの配布では、版の確認の側でこれを防げない。
  - 推測: 版の確認は表示だけで、取得と導入に結びつけない（§6）ので、害は「偽の Release のページに誘う」まで。そこから先は、利用者が ADR 0069 の証拠（作者の鍵の署名など）で確かめるかに懸かる。アカウントを取られた場合に、どの証拠が偽物を見分けられるかは確かめていない。

### 18.3 実装の課題（issue はリードが切る）
1. `publish.rs` の `release_args` に `--latest=false` を足し、テストで引数を確かめる。
2. `version-check` の形の検査（`v<数>.<数>.<数>` だけ、桁の上限）と、`bad-reply` のときの設定画面の表示。
3. 自由指定の警告の文（OS で分けない）と、sha256 なしで取ったあとの sha256 の表示（D-2c、D-4）。
