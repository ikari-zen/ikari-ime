# ADR 0076: 取得する部品 `ikari-data` の契約（プロセスの形、権限、入力と出力、版の確認、失敗、設定画面との口）

- 状態: 提案
- 日付: 2026-11-16
- 対象: 新しい crate `ikari-data`（置き場所 `fetch/`。lib と bin `ikari-data`）、`resource`（束を作る関数と取得の報告の型）、`windows/settings`（設定画面の核。起動と進み具合）、`windows/settings-ui`（ボタンと表示だけ）、`engine`（ファイルからのモデルの取り込み。Windows）、`shell-ffi`（同。macOS の設定アプリ）、`xtask`（検査）、インストーラ。この ADR ではコードを変えない
- 前提: origin/feat/security-impl2 = 0bd7822。[ADR 0071](0071-settings-ui-and-fetcher.md) §3（`ikari-data` の範囲。Q2 で OS の curl、Q3 で 1.0.0 まではモデルだけ）、[ADR 0070](0070-update-and-vulnerability-response.md) §3 の 3 と Q3（「新しい版を確認」のボタン）、[ADR 0066](0066-security-premises.md) §6（通信しないの定義と L1〜L3）、[ADR 0058](0058-offline-resource-install.md)（inbox と束、検証、入れ替え）、[ADR 0060](0060-user-selectable-lm-model.md)（自由指定のモデル、sha256 なしは警告で許可、試しの採点、`.loading`）、[ADR 0068](0068-ipc-endpoints.md) §4（設定の口、パスだけを流す）、[ADR 0072](0072-lm-cage-levels.md)（檻）、[security-design.md](../security-design.md) §8.5 の D-2
- 判断の置き場所: すべて Rust。取得の判断（どこから、何を、どの引数で、何を確かめるか）は `ikari-data`、束の形と sha256 は `resource`、取り込みの判断はエンジン。設定画面（C#、Swift）は、ボタン、ファイルを選ぶ窓、核が作った文の表示だけ（UI であるため）
- ADR 0071 との関係: 0071 §3 は「何をしてよいか」を決めた。この ADR は、それを実装できる形（型、引数、終了の仕方、置き場所、検査）にする。0071 の本文は書き換えない。0060 §6 の「`ikari-data model import --file`」だけを §7 で改める（Q1）

---

## 1. 分かったこと（2026-11-16）

- `Cargo.lock` に HTTP・TLS・ソケットの crate は無い。`deny.toml` の `[bans]` と `xtask/src/check/supply.rs` の `NET_CRATES` が同じ一覧で止めている。例外の欄 `NET_CRATES_ALLOWED` は空。`Cargo.lock` はワークスペースで 1 つなので、この検査は「どの exe に入ったか」までは見分けない。
- `xtask/src/check/product.rs` には、「LM の子の起動は `stack/src/lm.rs` の 1 か所だけ」をソースで確かめる検査（`lm_spawn_single`）がある。同じ形で「curl の起動は 1 か所だけ」を書ける。
- `scan-product`（`xtask/src/scan.rs`）は、開発用の文字列、llama.cpp の印、動的な C ランタイムを見る。通信の DLL（`ws2_32` など）の検査は、まだ入っていない（ADR 0066 §6.2 にはある）。
- 低い整合性で子を起動するコードは `lm/src/host/os/windows.rs` にある（LM の子のため）。
- 設定画面の核（`windows/settings`）は、書き出しと取り込みのパスを `shell-core::bridge::check_transfer_path`（絶対パスだけ、ネットワークとデバイスの場所を拒む）で確かめてから、パスだけを設定の口に送る（B-2）。
- `resource` には、manifest の型（`Origin` は `Export`、`Release`、`Local`）、束の検証 `verify_bundle`、既知のモデルの表 `KNOWN_MODELS` がある。`Origin::User` はまだ無い（ADR 0060 §6 で足す予定）。
- リポジトリの直下の `data/` は辞書などのデータの置き場なので、crate のディレクトリには使えない。
- TLS の crate（手元の crates.io の写しで見た。§9 の別の案のための事実）:
  - `ureq` 3.5.1 の既定は `rustls`（暗号は `ring`）と `webpki-roots`（Mozilla の根の証明書を埋め込む）。`rustls` 0.23 だけを入れたときの既定の暗号は `aws-lc-rs`（C と、ビルドの道具が増える）。
  - ライセンス: `ring` は `Apache-2.0 AND ISC`、`webpki-roots` 1.0 は `CDLA-Permissive-2.0`、`rustls-platform-verifier` は `MIT OR Apache-2.0`。今の `deny.toml` の許可には ISC と CDLA が無い。
  - `rustls-platform-verifier` は、macOS では `security-framework`（今の禁止の一覧にある）を、Windows では `windows-sys` を使う。
  - 大きさ（macOS arm64、release、LTO、strip。空の main との差）: `ureq` 既定で約 1.4MB 増えた（302KB → 1.71MB）。Windows では測っていない。

## 2. 決定の要約

| 項目 | 決定 |
| --- | --- |
| 形 | 別の実行ファイル `ikari-data`（Windows は `ikari-data.exe`、インストール先の固定の場所。macOS は `ikari.app/Contents/Helpers/`）。1 回の起動で 1 つの仕事をして終わる。常駐しない |
| 起こすもの | 設定画面の核（Rust）だけ。利用者が手で打つコマンドとしても動く。エンジン、TIP、殻、LM の子は起こさない |
| 通信の手段 | OS の curl を絶対パスで起動する（0071 Q2）。`ikari-data` 自身は通信のコードを持たない。新しい crate を依存に足さない |
| 仕事 | (a) プリセットのモデルの取得、(b) 自由指定の URL のモデルの取得、(c) アプリの版の確認。ファイルからの取り込みは持たない（§7。Q1） |
| 書く先 | 自分の作業の場所（`fetch/`）と、`inbox/<名前>.partial/` → `inbox/<名前>/` の rename だけ |
| 確かめ | `ikari-data` は、写すときに数えた sha256 と大きさを確かめる。取り込みの判断（検証、試しの採点、入れ替え）はエンジン（0058、0060） |
| 版の確認 | GitHub の「最新の Release」の転送先のタグを読むだけ。取り込みも導入もしない。古い版を勧めない |
| 失敗 | 全部の失敗で、作業の場所と `.partial` を消し、理由の語を 1 つ返す。自動では取り直さない |
| 設定画面との口 | 設定画面の核の C の ABI（開始、進み具合、取り消し）。核は `ikari-data` の標準出力の行を読み、文は核が作る |

## 3. プロセスの形と権限

### 3.1 起動の連鎖
- 設定画面の核（`ikari_settings_core.dll`。設定画面のプロセスの中の Rust）が、自分のモジュールの場所から決めたインストール先の `ikari-data.exe` を、絶対パスで起動する（R2。PATH から探さない。シェルを通さない）。引数は全部核が作る。
- `ikari-data` は、OS の curl を絶対パスで起動する（Windows は `GetSystemDirectoryW` の下の `curl.exe`、macOS は `/usr/bin/curl`）。
- 連鎖は「設定画面 → `ikari-data` → curl」の 1 本だけ。ほかの経路（エンジン、スケジューラ、Run キー、ファイルの関連付け）では起きない。
- 同時に走る取得は 1 つ。作業の場所の錠（`fetch/.lock`。排他で開く）で受ける。取れなければ `busy` で終わる。

### 3.2 Windows の権限
| プロセス | 整合性 | ほかの絞り | 理由 |
| --- | --- | --- | --- |
| `ikari-data` | 中（設定画面と同じ。`asInvoker`） | 高い権限で起動されたら何もせずに終わる（A-3 と同じ規則）。設定画面の核が Job Object に入れる（閉じたら終わる。子の数の上限）。渡すハンドルは標準出力のパイプだけ（S5） | `inbox` に書くため。受け取ったバイト列の解釈（TLS、HTTP）をしないので、中の整合性で動かしても、外から来たものを読むのは自分の作業の場所のファイルを写すところだけ |
| curl | **低**（`lm/src/host/os/windows.rs` と同じ作り） | `ikari-data` の Job Object の中（`ikari-data` が終われば終わる）。渡すハンドルは標準出力・標準エラーだけ | ネットワークから来たものを解釈するのは curl。利用者のファイルに書けないようにする |

- 低い整合性の curl が書けるのは、低い整合性の場所だけ。作業の場所は `%USERPROFILE%\AppData\LocalLow\ikari-ime\fetch\`（OS が低い整合性に用意している場所。ikari が ACL を変えない）。
- **作業の場所のファイルは信じない**（低い権限のほかのプログラムも書けるため）。`ikari-data` は、作業の場所のファイルを rename せず、`inbox/<名前>.partial/` に**写しながら sha256 を数える**。確かめるのは、写した先のバイト列の sha256。rename にしない理由は、低い整合性の印がファイルについたまま `models/` まで移ってしまうため。
- 写す分だけ、ディスクを一時に 2 倍使う（§6.3）。
- ファイアウォールの送信ブロックの規則は、`ikari-data` と curl にはかけない（0071 §3.1）。設定画面の exe にはかける（0066 §6.2 の L2）ので、設定画面のプロセス自体は通信できないまま。
- 低い整合性の curl が、Windows の証明書の確かめ（Schannel）と、プロキシの環境変数で動くかは推測（§11）。動かなければ、curl を中の整合性にし、Job Object とハンドルの限定だけにする（守りは 1 段下がる。結果の確かめ方は同じ）。

### 3.3 macOS の権限
- `ikari-data` は App Sandbox に入れ、`network.client` と、データの場所の例外だけを付ける（0071 §3.5）。`/usr/bin/curl` は Sandbox を引き継いで動く（推測。§11）。作業の場所は `<データの場所>/fetch/`。
- 設定アプリ（Swift）は、`shell-ffi` の関数を通して `ikari-data` を起動する。起動のコード（引数の組み立て）は Rust。Swift は関数を呼ぶだけ。

### 3.4 環境変数
- `ikari-data` が curl に渡す環境は、プロキシの変数（`https_proxy`、`HTTPS_PROXY`、`no_proxy`、`NO_PROXY`、`all_proxy`、`ALL_PROXY`）と、OS が動くのに要るもの（Windows は `SystemRoot`、`TEMP`、`TMP`。macOS は `TMPDIR`）だけ。ほかは消す（`CURL_HOME`、`SSLKEYLOGFILE`、`CURL_CA_BUNDLE`、`SSL_CERT_FILE` などを効かせない）。プロキシの変数を通すのは、R5 の文書に書いた例外（0071 §3.4）。
- `ikari-data` 自身は、環境変数で挙動を変えない（R5）。

## 4. 入力（何を取りに行くか）

### 4.1 引数（`ikari-data` のコマンドの形。人が打っても、核が起動しても同じ）
```
ikari-data model fetch --preset <id>
ikari-data model fetch --url <https の URL> --format <形式> [--sha256 <64 桁の 16 進>] [--name <表示名>]
ikari-data version-check
ikari-data cleanup          # 落ちたあとに残った作業の場所と .partial を消すだけ（通信しない）
```
- データの場所を変える引数は持たない（製品は `--data-dir` を受けない。0067）。
- 知らない引数、重なった引数は、何もせずに `usage` で終わる。

### 4.2 プリセット（既知のカタログ）
- 表はアプリに埋めたもの（`resource::MODEL_PRESETS`。ADR 0060 §5。今の `KNOWN_MODELS` を広げる）。1 行に、ID、ファイルの名前、sha256、大きさ、URL（https）。
- **表は遠くから取らない**。新しいプリセットは、アプリの更新で届く（0060 §5）。そのため、表の署名と、表の巻き戻しの問題は起きない（表は実行ファイルの中にあり、実行ファイルの真正性は ADR 0069 で守る）。
- プリセットの取得は、表の sha256 と大きさに合わなければ、必ず捨てる（警告で通す道は無い）。

### 4.3 自由指定の URL（ADR 0060 の段階 5）
- Rust で厳しく読む（新しい crate は使わない）: `https://` で始まる。ホストは ASCII だけ（国際化ドメインは `xn--` の形で入れてもらう）。利用者名とパスワードを含まない（`@` を拒む）。長さは 2048 文字まで。空白と制御文字を拒む。
- 確認の画面に出すホストは、Rust が読んだ ASCII のホストそのもの（0060 の「確認の画面でホストを出す」。見た目の似た文字にだまされないため ASCII に限る）。
- sha256 を指定したら照らす。無ければ、核が警告の文を出し、利用者が確かめてから起動する（0060 のユーザー決定）。`ikari-data` は、sha256 なしで取得したことを manifest の `source` に残す。
- `--format` は 0060 の `PromptFormat` の語。`ikari-data` は GGUF の中身を読まない（形式を推定しない。読むのは檻の中の子だけ）。

### 4.4 curl に渡す引数（Rust が全部決める）
- 先頭に `-q`（`.curlrc` を読まない）。`--proto =https --proto-redir =https`、`--location --max-redirs <N>`、`--fail`、`--silent --show-error`、`--max-filesize <上限>`、`--connect-timeout <秒>`、`--speed-limit 1 --speed-time <秒>`（止まった通信を切る）、`--user-agent ikari-data`（版を付けない）、`--output <作業の場所のファイル>`、URL は `--url <u>`（`-` で始まる URL を引数と取り違えないため）。
- 渡さないもの: `--insecure`、`--cacert`、`--cookie`、`--header`（UA のほか）、`--config`、`--netrc`、再開（`--continue-at`）。検査（§8）で止める。
- 値（N、秒、上限）は仮置き（Q3）。大きさの上限は、0060 §4 のモデルの上限と同じ値を使う（未定。それまでは仮置き）。
- `--max-filesize` は、curl の版によっては、大きさの宣言がない応答で効かない（推測）。そのため `ikari-data` は、作業の場所のファイルの大きさを見回り、上限を超えたら curl を止める。

## 5. 出力（どこに置き、だれが取り込むか）

### 5.1 流れ（モデル）
1. `ikari-data` が作業の場所に curl で落とす（§4.4）。進み具合は、作業の場所のファイルの大きさを見て、標準出力の行で知らせる（§6.1）。
2. curl が成功したら、作業の場所のファイルを `inbox/<名前>.partial/<ファイル名>` に写しながら sha256 と大きさを数える。プリセットは表と、自由指定は `--sha256` があれば照らす。合わなければ消して終わる。
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

- `ikari-data version-check` は、`https://github.com/ikari-zen/ikari-ime/releases/latest` に、転送を追わずに 1 回だけ要求し（`--max-redirs 0`、`-w '%{redirect_url}'`、本文は捨てる、受ける量の上限は小さく）、転送先の URL の末尾のタグを読む。URL は実行ファイルに埋めた固定のもの。
- タグは `v<数>.<数>.<数>` だけを受ける（ほかは `bad-reply`）。`latest 0.3.1` の行を出して終わる。
- 比べるのは設定画面の核（自分の版は `CARGO_PKG_VERSION`）。表示は 3 つだけ: 「新しい版があります（0.3.1）」、「最新です」、「この版のほうが新しい（開発版）」。**取った版が今より古くても、古い版を勧めない**（巻き戻しの拒否。0070 §3 の 4 で、古い版を入れるのはインストーラの確認つきだけ）。
- 取得も導入もしない。「Releases を開く」のリンクは固定の URL で、押したときだけブラウザで開く（応答の中身を URL にしない）。真正性の確かめ方は ADR 0069 の案内に任せる。
- 署名は付けない。表示するのは版の番号だけで、だまされても起きるのは「表示が違う」までのため（取得と導入に結びつけない限り）。TLS は curl が確かめる。
- 自動では確認しない（起動のとき、定期、設定画面を開いたとき、のどれもしない）。

## 7. 自由指定のモデルの境界（ADR 0060 の段階 3〜5）と、ファイルからの取り込み

- `ikari-data` が持つのは URL からの取得だけ。**ファイルからの取り込みは `ikari-data` に置かない**（0060 §6 の `ikari-data model import --file` を改める。Q1）。理由: 通信してよい部品に、利用者の任意のファイルを読む力を足さない（権限を 1 つに絞る）。B-2 の書き出しと取り込み（パスを流す）と同じ流儀にそろう。
  - Windows: 設定画面の核が、OS のファイルを選ぶ窓で得たパスを `check_transfer_path` で確かめ、設定の口に `ImportModelFile { path, format, sha256 }`（`bridge.proto` に足す。パスだけ）を送る。engine-host が同じ検査をして読み、`resource::stage_model_bundle` で inbox に束を作る。
  - macOS: エンジンは Sandbox で利用者のファイルを読めないので、設定アプリ（Sandbox の外）が `shell-ffi` の関数を呼び、同じ `stage_model_bundle` で inbox に束を作る。
- そのあとは URL のときと同じ（§5.1 の 5、6）。sha256 なしは警告で許可（0060）。試しの採点に通らなくても、警告を出して使えるようにする（0060 Q2）。
- 檻（0060 §4.1、0072）: モデルの中身を読むのは `ikari-lm-host` だけ。`ikari-data` と engine と設定画面は、モデルのバイト列を写して数えるだけで、解釈しない。自由指定のモデルを出すのは段 2 のある OS だけ（0072 Q1）。設定画面の核は、段 2 が無いときは自由指定のボタンを出さない（判断は核）。
- モデルが自分で通信したりファイルを触ったりできないことは、檻（段 2）で受ける。`ikari-data` の側では何もしない。

## 8. 失敗のふるまい

### 8.1 標準出力の行と終了コード（`resource::fetch_report` に型と読み書きを置く。核と `ikari-data` が共有する）
```
progress <受けたバイト> <全体のバイト|->
done <束の名前> <sha256>
latest <版>
error <理由の語>
```
- 1 行は 256 バイトまで。核は、知らない行、長すぎる行、`done`・`latest`・`error` のあとの行を捨てる。終了コードは 0（成功）と 1（`error` を出した）と 2（`usage`）。文は核が理由の語から作る（`windows/settings/src/text.rs`）。

### 8.2 理由の語（`FetchError`）
| 語 | いつ | 片づけ |
| --- | --- | --- |
| `cancelled` | 利用者が取り消した（核が Job Object を閉じる） | 次の起動の `cleanup` か、次の取得の始めに消す |
| `network` | つながらない、名前が引けない、プロキシが断った | 作業の場所を消す |
| `tls` | 証明書の確かめに失敗 | 同じ |
| `http-status` | 成功でない応答（`--fail`） | 同じ |
| `stalled` | 決めた時間、受けるバイトが増えない | 同じ |
| `too-large` | 大きさの上限を超えた | 同じ |
| `checksum` | sha256 か大きさが合わない（壊れた、途中で変わった、改ざん） | 作業の場所と `.partial` を消す。自動で取り直さない |
| `disk-full` | 始める前の見積もり、または書くときに足りない | 同じ |
| `bad-url` | §4.3 に合わない | 何も作らない |
| `bad-reply` | 版の確認の応答が読めない | 同じ |
| `tool` | curl が無い、要る引数を受けない | 同じ。実機でこれが出たら 0071 Q2 の別の案（§9） |
| `busy` | ほかの取得が走っている | 何もしない |
| `refused` | 高い権限で起動された | 何もしない |
| `internal` | 上のどれでもない | 作業の場所と `.partial` を消す |

### 8.3 決まりごと
- 途中で切れたもの（落ちた、電源が切れた）は、`.partial` で終わるディレクトリのまま残り、エンジンは見ない（0058 §4.3）。次の取得の始めに `ikari-data` が消す（`cleanup` と同じ処理）。
- 続きからの取り直し（Range）はしない。最初から取り直す（単純さと、途中のファイルを信じないため）。
- ディスクの見積もり: プリセットは、表の大きさの 2 倍（作業の場所と写し）と余裕。自由指定は大きさが分からないので、始める前は上限の 1 倍と余裕、写す前にもう一度、受けた大きさと余裕で見る。余裕の値は仮置き（Q3）。
- `checksum` のとき、設定画面は「取ったファイルが期待したものと違いました」とだけ出し、取り直しは利用者のボタンだけ。
- エンジンの取り込みの失敗（束の検証、試しの採点）は、今までどおり `inbox/status.json` と `inbox/rejected/`（0058 §8.3）。設定画面は、取得の結果（`done`）のあと、その状態を設定の口で読んで出す。

## 9. 通信の手段（TLS の選定）

- **推奨: 決定のとおり OS の curl（0071 Q2）を続ける。** 理由に足すこと: (1) 依存の一覧が変わらないので、`NET_CRATES_ALLOWED` を空のままにでき、「通信する crate は製品に無い」を今の検査の文のまま言える。(2) ISC、CDLA の許可を足さずに済む。
- curl が VM の確認（§11）で通らないときの別の案は、**Windows だけ**に、`ureq`（`rustls-no-provider`）＋`rustls`＋`ring`＋`rustls-platform-verifier` を `ikari-data` の bin にだけ入れる（Q2）。
  - 暗号は `ring`（`aws-lc-rs` は C のビルドの道具が増え、大きい）。
  - 証明書は OS の確かめ（`rustls-platform-verifier`。Windows の証明書の置き場を使うので、会社のプロキシが入れた独自の根も通る。失効の扱いも OS）。`webpki-roots`（根を埋め込む）は、根の更新を ikari が配ることになり、プロキシの下で通らないので採らない。
  - 証明書の固定（ピン留め）はしない。配布元（Hugging Face、GitHub）は証明書と配信の先を替えるので、固定すると取得が止まる。中身の真正性は sha256 で受ける。
  - macOS は `/usr/bin/curl` が必ずあるので、別の案は要らない（`security-framework` を禁止の一覧から外さずに済む）。
  - 検査の書き方: `deny.toml` は `[bans]` の該当の行に `wrappers = ["ikari-data"]`（と、間に入る crate）を付ける。`supply.rs` は `Cargo.lock` だけでは exe を見分けられないので、`cargo tree -p <製品の crate> -e normal` の結果で、`ikari-data` 以外に入っていないことを見る形に替える。`scan-product` は、`ikari-data` だけを通信の DLL の検査から名前で外す。
  - 増える大きさは、macOS で約 1.4MB（§1）。Windows は未計測。

## 10. 設計を守る検査（§8 の契約のレーンで入れる）

| 検査 | 置き場所 | 中身 |
| --- | --- | --- |
| curl の起動は 1 か所 | `xtask/src/check/product.rs`（`lm_spawn_single` と同じ形） | OS の curl を起動する関数（`system_curl(`）を呼ぶのは `fetch/src/curl.rs` だけ。ほかの製品のソースに `curl` の起動が無い |
| `ikari-data` の起動は 1 か所ずつ | 同じ | `ikari-data` を起動する関数を呼ぶのは、`windows/settings/src/fetch.rs` と、`shell-ffi` の該当のファイルだけ |
| 渡さない引数 | `fetch/` の単体テスト | 組み立てた引数に `-q` が先頭にあり、`--insecure`・`-k`・`--cacert`・`--config`・`--netrc` が無い。URL は `--url` のあと |
| 通信の crate | `deny.toml`、`supply.rs`（今のまま） | 増やさない。§9 の別の案に下がるときだけ書き換える |
| 通信の DLL | `scan-product`（ADR 0066 §6.2。未実装の分） | 製品の exe と DLL が `ws2_32`、`winhttp`、`wininet` を取り込まない。curl の案なら `ikari-data` も外さない（Rust の std がこれを取り込むかは Windows の CI で確かめる） |
| 設定画面が通信しない | 実機（L3）と、C# のソースの検査（`HttpClient`、`WebRequest`、`Socket`、`Process.Start` を書かない） | `windows/settings-ui` |
| macOS の権限 | ビルドの検査（0071 §3.5） | `network.client` を持つのは `ikari-data` だけ |

## 11. 推測と、確かめること（VM・実機。`docs/windows-real-device-check.md` に足す）

1. Windows 11 の `curl.exe` が §4.4 の引数を全部受けること。`--max-filesize` が大きさの宣言のない応答で効くか。
2. 低い整合性の curl が、Schannel の証明書の確かめ（失効の確かめを含む）、プロキシの環境変数、`LocalLow` への書き込みで動くこと。
3. Hugging Face のプリセットの URL が、転送の回数の上限の中で取れること（転送先のホストが変わる）。
4. 取得の間の送信の記録（L3）: 送信するのは curl だけで、行き先は表のホストと転送先だけ。設定画面、エンジン、`ikari-data` 自身は送信しない。
5. 取り消し（Job Object を閉じる）で、curl も終わり、作業の場所が次の起動で消えること。ディスクが足りないとき、途中で回線を切ったときの表示。
6. macOS: Sandbox の中の `ikari-data` から `/usr/bin/curl` が動くこと（`network.client` を引き継ぐ）。
7. 版の確認: 転送先のタグが読めること。Release が 1 つも無い間（今）の応答で `bad-reply` になり、設定画面が落ちないこと。

## 12. 検討した案

- **設定画面の核が curl を直接起動する（`ikari-data` を作らない）**: 部品が 1 つ減る。設定画面の exe にかけるファイアウォールの規則と、「通信してよいのは 1 つ」の検査の文が崩れる。採らない（0071 §4 と同じ理由）。
- **エンジンが取得を頼まれて `ikari-data` を起こす**: 設定の口に取得の要求が増え、エンジンが通信の連鎖の途中に入る（0066 §6.1 の「エンジンは操作があっても出ない」に近づく）。採らない。
- **作業の場所を `inbox` の下に置き、rename で入れる**: 写す分のディスクが要らない。低い整合性の curl に書かせるには `inbox` の下を低い整合性に開けることになり、エンジンが自動で取り込む場所に、低い権限のプログラムが書けるようになる。採らない。
- **版の確認に GitHub の API（JSON）を使う**: リリースのノートも取れる。認証なしの回数の制限があり、JSON を読むコードと受ける量が増える。表示するのは版の番号だけなので、転送先を読むので足りる。
- **sha256 の表を遠くから取り、署名で守る**: アプリの更新なしにプリセットを足せる。新しい通信と信頼の根（鍵）と巻き戻しの問題が増える。0060 §5 の「遠くから表を取らない」のまま。
- **ファイルからの取り込みも `ikari-data` に置く（0060 §6 のまま）**: コマンドが 1 つにまとまる。通信してよい部品が利用者の任意のファイルを読むことになる。§7 の形を推奨（Q1）。

## 13. 実装の進め方（1 タスク 1 ブランチ。D-1 のあと）

| 段 | 中身 | 持つファイル | モデル |
| --- | --- | --- | --- |
| D-2a 契約 | `fetch/` の crate の骨組み（引数の型、`FetchError`、`system_curl` のシグネチャとスタブ）、`resource` の `Origin::Preset`・`Origin::User`・`StagePlan`・`stage_model_bundle` のシグネチャ、`fetch_report` の型と読み書き（実装まで。純粋）、設定画面の核の ABI のシグネチャ（`ikari_settings_fetch_start`・`_poll`・`_cancel`・`ikari_settings_version_check_start`）、`bridge.proto` の `ImportModelFile`、§10 の検査（ソースの 2 つ） | `fetch/`、`resource/src/`、`windows/settings/src/abi.rs`・`fetch.rs`（スタブ）、`shell-core/proto/`、`xtask/src/check/product.rs`、`Cargo.toml` | 決めるのは architect、入れるのはリード |
| D-2b 取得 | URL の読み、curl の引数、作業の場所と錠、見回り（大きさ、止まった通信）、写しと sha256、`cleanup`。Windows の低い整合性の起動は、`lm/src/host/os/windows.rs` の起動の部分を、`ikari-data` から使える小さな場所に移して共有する（`ikari-data` が `lm` に依存しないため） | `fetch/src/`、移す先 | Sonnet（低い整合性の起動を移すところだけ Opus） |
| D-2c 設定画面 | 核の起動（Job Object）、行の読み、文、取り消し。画面のボタン、確認の窓（ホスト、sha256 なしの警告）、進み具合、取り込みの状態の表示 | `windows/settings/src/`、`windows/settings-ui/` | Sonnet |
| D-2d 版の確認 | `version-check` と、核の比べ方と表示 | `fetch/src/version.rs`、`windows/settings/src/` | Sonnet |
| D-2e インストーラと macOS | `ikari-data.exe` を置く（規則はかけない）、macOS の Helpers と entitlements とその検査 | `windows/installer/`、`macos/` | Sonnet |

- ファイルからの取り込み（§7）は D-4（自由指定のモデル）に入れる。試しの採点を檻の子に頼む口は D-3・D-4 の契約で決める。
- どのレーンへの依頼にも「直すまで、弱い点の詳細を公開の issue・PR・コミットメッセージに書かない」（0070 §5 の 3）を書く。
- 実機の確認（§11）は `user-needed` の issue にする。D-2b のあと、D-2c と一緒に VM で見る。

## 14. 結果

- 「通信するのは `ikari-data` だけ」が、依存の一覧（変わらない）、ソースの検査（起動の 1 か所）、OS の規則（設定画面にはかかる）、実機の記録の 4 つで言える。
- ネットワークから来たものを解釈するのは OS の curl と、檻の中の LM の子だけになる。`ikari-data` とエンジンはバイト列を数えて写すだけ。
- 失うもの: 写す分のディスク（モデルの大きさの 1 倍、一時に）、続きからの取り直し、OS の curl の版の差への依存（VM で確かめる）。

## 15. ユーザーに聞くこと（推奨つき）

- **Q1. ファイルからのモデルの取り込みを、`ikari-data` から外す（0060 §6 を改める）。** Windows はパスを設定の口でエンジンに渡し、macOS は設定アプリが Rust の関数で inbox に束を作る（§7）。**推奨: 外す。** 通信してよい部品に、利用者の任意のファイルを読む力を足さないため。B-2 と同じ流儀になる。
- **Q2. OS の curl が VM の確認で通らなかったときの別の案は、Windows だけに `ureq`＋`rustls`＋`ring`＋OS の証明書の確かめを `ikari-data` に入れる（§9）。** ライセンスの許可に ISC を足すことになる。**推奨: この形。** 根の証明書を埋め込む案（CDLA の許可も要る）と、証明書の固定は採らない。
- **Q3. 計測なしの仮置きの値**: 転送の回数 5、つなぐまでの締め切り 30 秒、止まったと見なす時間 60 秒、版の確認で受ける量 64KiB、ディスクの余裕 256MiB。**推奨: 仮置きで実装し、VM の確認（§11）で見直す。** モデルの大きさの上限は 0060 §4 の値（未定）に従う。
