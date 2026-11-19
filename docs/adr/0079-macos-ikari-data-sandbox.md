# ADR 0079: macOS の `ikari-data` を App Sandbox で動かす形（置き場所、署名、entitlement は通信と `fetch/`・`inbox/` だけ、検査、設定アプリからの起動）

- 状態: 承認（2026-11-19 ユーザー決定。§10 の Q1〜Q3 はすべて推奨どおり）。スパイク (a)〜(d)（§5）は 2026-11-20 に済み（§8。決定は変えない。§3.5 は既定の形に確定）
- 日付: 2026-11-19
- 対象: `macos/scripts/build.sh`・`lib.sh`・`templates/`（`ikari-data` のビルド、Info.plist の埋め込み、署名、`Contents/Helpers/` への配置、検査）、`fetch/`（macOS のデータの場所の決め方、親の終わりの見張り）、`shell-ffi`（設定アプリから起動する C の ABI）、`windows/settings/src/fetch.rs`・`text.rs` の OS に依存しない部分（共有の場所へ移す。§3.6）。この ADR ではコードを変えない
- 前提: origin/main = fe5f90c。[ADR 0076](0076-ikari-data-contract.md)（取得の契約。§3.1 起動の連鎖、§3.3 macOS の権限、§13 の D-2e、§16 の実測）、[ADR 0078](0078-fetch-child-handle-and-appcontainer.md) §3.4 と Q2（macOS の例外を `fetch/` と `inbox/` だけにし、別の issue にする。2026-11-19 ユーザー決定）、[ADR 0077](0077-macos-lm-cage.md)（檻役の束、署名、`verify_cage` の型）、[ADR 0005](0005-macos-settings-and-indicator.md) §6.2（設定アプリは Sandbox なし）、[ADR 0069](0069-distribution-authenticity.md) §6（ad-hoc の配布）、[ADR 0070](0070-update-and-vulnerability-response.md) §5、#489、#99
- 判断の置き場所: Rust。起動するか、どの exe を、どの引数で起こすか、行の読み、文は Rust（`shell-ffi` と共有の核）。`ikari-data` の中の判断は今までどおり `fetch/`。Swift は、ボタンから ABI の関数を呼び、返った文と数を描くだけ（UI であるため）。権限の絞り（entitlement）は署名のときに付ける OS の設定で、その守りは `build.sh` の検査で見る
- ほかの ADR との関係: 0076 §3.3 の「データの場所の例外」を「`fetch/` と `inbox/` の 2 つ」と具体にする（0078 Q2 の決定の実装の形）。0076 §13 の D-2e の macOS の分を、この ADR のレーン（§7）に置き換える。0076・0078 のほかの決定（子を作らない、通信は `ikari-data` 自身、Info.plist の埋め込み、引数は Rust）は変えない

---

## 1. 分かったこと（2026-11-19。researcher とコードで確かめた）

- macOS の `ikari-data` は、今はビルドも署名も配置もされていない。`macos/scripts/build.sh` に `ikari-data` が無く、`Contents/Helpers/` には `IkariSettings.app` だけがある。`ikari-data` 用の entitlement と Info.plist のテンプレートは無く、`fetch/` に build.rs も無い。`shell-ffi` に起動の関数も無い。設定アプリ（Swift）に取得の画面も無い。
- エンジンの entitlement（`templates/engine.entitlements`）は、`app-sandbox` と、`home-relative-path.read-write` の一時例外 `/Library/Application Support/ikari-ime/`（データの場所の全体）。`build.sh` の検査（`verify_bundle`）はこの文字列があることを見る。エンジンは学習、設定、`models/`、`inbox/` を読み書きするので、この範囲は変えない。
- `ikari-data` が触れるのは、作業の場所 `fetch/`（`fetch/.lock`、`download.bin`）、`inbox/<名前>.partial/` への写しと `inbox/<名前>/` への rename、空きの確認（`statvfs`）、ネットワークだけ（0076 §2、§5.1）。
- 作業の場所の作り方（`fetch/src/workspace.rs` の `hold_work_chain`）は、根（macOS はデータの場所そのもの `…/ikari-ime/`）に `create_dir_all` をかけ、根から `fetch/` まで 1 段ずつ作って確かめる。
- データの場所は `core/src/user/dir.rs` の `UserDir::resolve` が、環境変数 `HOME` から決める。エンジンの Swift（`IkariEngineService/main.swift` の `dataDirectory`）は、Sandbox の中で `NSHomeDirectory()` がコンテナを指すので、`getpwuid` で本当のホームを取っている。
- Info.plist を埋めない Sandbox の exe は、起動の直後に落ちる（0076 §16。ad-hoc でも Apple Development でも）。`app-sandbox` と `network.client` を付けた ad-hoc の exe で、取得が動いた（同。置き場所は Helpers ではない。一時例外は付けていない）。
- 檻役（0077）には、署名のあとに「identifier」「Hardened Runtime」「entitlement のキーの数」を見る検査（`verify_cage`）がある。設定アプリには「ikari.app の署名が Helpers の中を封印している」検査がある。
- LM の子（`lm/src/host/serve.rs`）は、`getppid()` の変化を見て、親が終わったら自分も終わる。
- Windows の設定画面の核（`windows/settings/src/fetch.rs`）は、OS に依存しない部分（`FetchRequest`、`model_fetch_args`、行の読み、`FetchTask`、`Launcher` の trait）と、OS の起動（`fetch_os.rs`）に分かれている。文は `text.rs`。ABI は `fetch_abi.rs`（`ikari_settings_fetch_start`・`_poll`・`_sha256`・`_cancel`・`_free`、`ikari_settings_version_check_start`）。

## 2. 問題

- #99（最初の公開）の前に、macOS の `ikari-data` が触れる場所を `fetch/` と `inbox/` に限りたい（0078 Q2）。今はそもそも macOS の `ikari-data` が配られていないので、絞る対象が無い。絞りを実現するには、ビルド、署名、配置、起動（0076 §13 の D-2e の macOS の分）を一緒に作ることになる。
- 絞りは、署名のときの entitlement 1 つで決まる。間違えて広げても、ビルドは通り、取得も動く（気づけない）。だから検査で守る必要がある。

## 3. 決定

### 3.1 置き場所と形
- **`ikari.app/Contents/Helpers/ikari-data`**（束にしない単体の exe。0076 §2 のとおり）。`IkariSettings.app` と同じ階層。
- 1 回 1 仕事、常駐しない、子を作らない（0076 §3.1）。
- **今は通常のビルドに必ず入れる。** オフライン版（ADR 0060 の 2026-11-19 の更新）は `ikari-data` を持たないが、その版のビルドはまだ無い。作るときに `build.sh` に版の切り替えを足し、オフライン版では「`ikari-data` が無いこと」と「どの署名にも `network.*` が無いこと」を検査にする（そのときの課題。今は作らない）。

### 3.2 署名の単位、identifier、Info.plist
- 署名の単位は `ikari-data` の exe 1 つ。identifier は **`$BUNDLE_ID.data`**（`io.github.ikari-zen.inputmethod.ikari.data`。`lib.sh` に `DATA_ID` として足す。ほかの `*_ID` と同じ並び）。Hardened Runtime（`--options runtime`）を付ける。
- 署名の順は、内側から: `ikari-data` → 設定アプリ → `ikari.app`（`ikari.app` の署名が Helpers の中を封印する）。
- **Info.plist は `build.sh` が埋める。** テンプレート `templates/ikari-data.Info.plist`（`CFBundleIdentifier` は `@IKARI_ID_PREFIX@.inputmethod.ikari.data`、版は build.sh が埋める。`CFBundleExecutable` は `ikari-data`）を `$OUT` に埋めたものを作り、`cargo rustc --release --target $RUST_TARGET -p ikari-data --bin ikari-data -- -C link-arg=-Wl,-sectcreate,__TEXT,__info_plist,<埋めた plist>` でリンクする。エンジンの FFI を `cargo rustc` で作っているのと同じ型。
  - `fetch/` に build.rs を足して埋める案は採らない（§4 の 2）。そのため、手元の `cargo build -p ikari-data` の exe には Info.plist が無い。これは Sandbox を付けずにテストで使うだけで、配る exe ではない。
- `CFBundleIdentifier` は署名の identifier と同じ（0076 §3.3）。検査で一致を見る（§3.7）。

### 3.3 entitlement（`templates/ikari-data.entitlements`。新しいファイル）
キーはちょうど 3 つ。ほかは持たない。

| キー | 値 |
| --- | --- |
| `com.apple.security.app-sandbox` | true |
| `com.apple.security.network.client` | true |
| `com.apple.security.temporary-exception.files.home-relative-path.read-write` | `/Library/Application Support/ikari-ime/fetch/` と `/Library/Application Support/ikari-ime/inbox/` の 2 つだけ（この順） |

- `network.server`、`files.user-selected.*`、`inherit`、読み取りだけの例外、ほかの一時例外は付けない。
- エンジンの entitlement（`engine.entitlements`）は変えない。`ikari-data` の分を、エンジンのファイルに足さない・共有しない（ファイルを分けるのは、片方を広げたときにもう片方が広がらないようにするため）。
- データの場所が利用者の指定で変わる口は、製品には無い（ADR 0067。`dev-knobs` だけ）。例外は既定の場所だけで足りる。

### 3.4 データの場所の決め方（macOS の `ikari-data`）
- **macOS の `ikari-data` は、データの場所を `HOME` から決めない。** `getpwuid_r(getuid())` の `pw_dir` から決める（エンジンの `dataDirectory` と同じ規則）。理由: 一時例外はホームからの相対で、Sandbox の中の `HOME` がコンテナを指すと、ずれて何も書けなくなる（根の `lstat` すら ENOENT になる）。**Sandbox は `HOME` を必ずコンテナに設定する**（環境を空にして起こしても同じ。§8 の 3）ので、`HOME` からは本当のホームを得られない。また、製品は環境変数で挙動を変えない（R5）。
- 置き場所は `fetch/src/workspace.rs` の macOS の分だけ（`Dirs::resolve`）。`core` の `UserDir::resolve` は変えない（ほかの部品の決め方に波及させないため）。`dev-knobs` の上書きの口は今のまま（製品には入らない）。

### 3.5 ディレクトリの作成の分担
- **決定（スパイク (a) で通った。§8 の 1）: `ikari-data` が `fetch/` と `inbox/` を自分で作る。** データの場所そのもの（`…/ikari-ime/`）は作らない。無ければ `internal` で終わる（データの場所はエンジンと `install-data.sh` が作る。データの場所が無いときは取得しても取り込めない）。
  - そのために `hold_work_chain` の macOS の分を、根に `create_dir_all` をかけず「根があり、ディレクトリで、リンクでないこと」を確かめるだけにする。`inbox/` も同じ規則（根の下に 1 段だけ作る）。
  - 実装の注意（§8 の 2）: 根に対して Sandbox の中から通るのは `lstat` だけで、`open(根)`（一覧）は EPERM。根が無いときは根の `mkdir` が EPERM になる。根を確かめる段で、無い・`lstat` が通らない・根の操作の EPERM を、`internal` に写す（EPERM をそのまま別の種類の失敗として出さない）。仮に根に `create_dir_all` をかけても、根があれば mkdir が EEXIST で返るので失敗しないが、確かめるだけの形にそろえる。
  - こうすると、人が手で打っても、設定アプリから起こしても同じに動く（0076 §4.1）。
- **代わりの案（採らなかった。(a) が通らない場合に備えて書いていたもの。記録として残す）: 親（`shell-ffi` の起動の関数。Sandbox なし）が、起動の前に `fetch/` と `inbox/` を作る。** `ikari-data` は、無ければ作ろうとせず `internal` で終わる。作り方は今の `nofollow` の規則（リンクを辿らない、1 段ずつ）を共有して使う。手で打つときは、先に設定アプリかエンジンが作っている前提になる（README の手で打つ説明に書く）。
- どちらでも、`fetch/.lock` は `ikari-data` が作る（`fetch/` の中なので例外の範囲）。

### 3.6 親の起動の契約（`shell-ffi`。実装は §7 のレーン）
- 設定アプリ（Swift）は `shell-ffi` の関数だけを呼ぶ。exe の場所と引数は Rust が決める。
- **OS に依存しない核を、Windows と共有する。** `windows/settings/src/fetch.rs`（`FetchRequest`、`model_fetch_args`、`Snapshot`、`Phase`、`Running`、`Launcher`、`FetchTask`、行の読み）と、`text.rs` の取得の文（`fetch_error`、`fetch_snapshot` など）を、両方の核が依存できる共有の場所へ移す。置き場所は `shell-core` の `settings::fetch`（`shell-core` に `ikari-resource` の依存を足す）。Windows の `fetch_os.rs`（`ikari-sandbox` での起動）と `fetch_abi.rs` は今の場所に残す（§10 の Q2）。
- `shell-ffi` に足す関数（名前と引数の並びは Windows の `fetch_abi.rs` と同じ。文の入れ物は `IkariShellBuf`、失敗の値は `shell-ffi` の値。ヘッダは `shell-ffi` のヘッダ）:

```c
int32_t ikari_settings_fetch_start(int32_t kind,
    const uint8_t *a, size_t a_len, const uint8_t *format, size_t format_len,
    const uint8_t *sha256, size_t sha256_len, const uint8_t *name, size_t name_len,
    IkariSettingsFetch **out, IkariShellBuf *msg);
int32_t ikari_settings_version_check_start(IkariSettingsFetch **out, IkariShellBuf *msg);
int32_t ikari_settings_fetch_poll(const IkariSettingsFetch *fetch,
    IkariSettingsFetchStatus *status, IkariShellBuf *text);
int32_t ikari_settings_fetch_sha256(const IkariSettingsFetch *fetch, IkariShellBuf *out);
int32_t ikari_settings_fetch_cancel(const IkariSettingsFetch *fetch);
void    ikari_settings_fetch_free(IkariSettingsFetch *fetch);
```

  - `kind` の値と `IkariSettingsFetchStatus` の形は Windows と同じ（`phase`、`received`、`total`、`has_total`）。`version_check_start` の引数は Windows の `fetch_abi.rs` に合わせる（違えば Windows に合わせる）。
- macOS の `Launcher`（`shell-ffi` の中。`std::process::Command` で起こす）:
  - exe の場所: 自分の exe（`current_exe` を正規化）が `<X>/ikari.app/Contents/Helpers/IkariSettings.app/Contents/MacOS/` にあることを確かめ、`<X>/ikari.app/Contents/Helpers/ikari-data` を絶対パスで起こす。形が違えば起こさない（`tool`）。PATH を使わない。シェルを通さない。
  - 標準入力と標準エラーは `/dev/null`、標準出力はパイプ。環境は 0076 §3.4 と同じく、プロキシの変数だけに絞る（`HOME` も渡さない。データの場所は §3.4 で決める）。`HOME` を渡さなくても、Sandbox の初期化がコンテナを `HOME` に設定し、作成と通信（TCP の接続と https の HEAD）が動くことは確かめた（§8 の 3、8）。ureq（`NativeTls`、証明書の確かめ）での実際の取得は、§7 の 5 の実機の確認で見る。そこでほかの変数が要ると分かれば、最小を足す。
  - 取り消しは、その子に `SIGKILL`（子は孫を作らないので、木はその 1 つ）。待ちは `wait`。
- `ikari-data`（macOS）は、親が終わったら自分も終わる。`lm/src/host/serve.rs` と同じく `getppid()` の変化を見る（Windows の Job Object の「閉じたら終わる」に当たる）。

### 3.7 `build.sh` の検査（`verify_data`。署名のあとに必ず流す）
1. `Contents/Helpers/ikari-data` がある、実行できる、`codesign --verify --strict` が通る。
2. 署名の identifier が `$DATA_ID`。Hardened Runtime（`runtime`）が付いている。
3. 埋めた Info.plist がある（`codesign -dv` の `Info.plist entries=` が出る）。その `CFBundleIdentifier` が `$DATA_ID`。
4. entitlement のキーがちょうど 3 つ（§3.3）。`app-sandbox` と `network.client` が true。
5. 一時例外の配列が、`fetch/` と `inbox/` の 2 つの文字列に**厳密に一致**する（要素の数が 2、並びも同じ）。`ikari-ime/` そのもの、末尾の `/` の無い形、ほかの場所が 1 つでもあれば失敗。
6. **束の中で `network.*` を持つのは `ikari-data` だけ。** `ikari.app` の中の署名されたコード（殻、エンジン、檻役、LM の子、設定アプリ、smoke、`ikari-data`）を全部見て、`ikari-data` のほかに `com.apple.security.network.` で始まるキーがあれば失敗。`ikari-data` も `network.server` を持てば失敗。新しい部品が増えても漏れないよう、一覧を手で並べず `Contents/` の下を見て回る。
7. `ikari.app` の署名が `Helpers/ikari-data` を封印している（`CodeResources` に名前がある。設定アプリと同じ検査）。
- 検査の名前と文は、檻役（`verify_cage`）と同じ書き方にする。`check.sh` の `build.sh` の検査（macOS）でも流れる。

## 4. 検討した案

1. **エンジンの例外の下に `ikari-data` を置く（entitlement を共有する）**: 採らない。触れる場所がデータの場所の全体（学習、ユーザー辞書、設定）になり、0078 Q2 の決定に反する。
2. **Info.plist を `fetch/` の build.rs で埋める**: 手元のビルドでも Info.plist が付く利点がある。採らない理由: identifier は `IKARI_ID_PREFIX`（`lib.sh`）で変わり、版も build.sh が決めるので、二つの場所で同じ値を持つことになる。`fetch/` は Windows でもビルドされ、macOS の署名の知識を Rust の crate に入れたくない。
3. **`ikari-data` を `.app` の束にする（`Contents/Helpers/IkariData.app`）**: Info.plist は束に置けるが、利用者が Finder から起こせてしまい、Dock に出る心配も増える。0076 §2・§3.3 の決定（単体の exe に埋める）を変える理由が無い。
4. **起動の核を `shell-ffi` に写す（Windows と共有しない）**: 早いが、行の読み、取り消し、文の同じものが 2 つになり、片方だけ直す事故が起きる。共有して、OS の起動だけを分ける（§3.6。CLAUDE.md の「OS に依存しないものは Rust で 1 つ」）。
5. **ディレクトリを常に親が作る**: `ikari-data` の単体での動き（人が打っても同じ）が崩れる。(a) が通るなら要らないので、代わりの案に置く（§3.5）。
6. **例外にデータの場所そのものの読み取りを足す（`read-only` で `ikari-ime/`）**: 例外は下の全部に効くので、学習やユーザー辞書が読める。採らない。

## 5. スパイク（実装の前に、このリポジトリの macOS で確かめる。コードはリポジトリに入れない）

**約束**: ユーザーの ikari を止めない。`smoke` は流さない。インストール（`install.sh`）もしない（検証用の exe を `~/Library/Input Methods` に置かない）。ユーザーデータに書かないため、データの場所は**検証用の別のホーム**を使う（下の手順）。結果は §8 に書く。

**作るもの**（スクラッチの場所。`target/` の外でよい）:
- 0076 §16 の最小の exe に、(1) `getpwuid_r` と `HOME` の両方を出力する、(2) 与えたデータの場所で `fetch/` を作り `fetch/.lock` を開いて `try_lock`、`inbox/x.partial/f` を書き `inbox/x/` へ rename、`statvfs`、の 3 つを足したもの。Info.plist を `-sectcreate` で埋める。
- entitlement は §3.3 の 3 キー。ただし一時例外のパスは、検証用のホームでも効くよう、ホームからの相対の `/Library/Application Support/ikari-ime-spike/fetch/` と `…/inbox/` にする（本物のデータの場所に触れないため。例外の効き方は同じ）。

**手順**:
1. 署名: ad-hoc と Apple Development（team。`macos/scripts/lib.sh` の `TEAM_ID`）の 2 通りで、`--options runtime` と識別子 `…ikari.data.spike` を付ける。
2. **(a) 作成**: `~/Library/Application Support/ikari-ime-spike/` を作ったうえで、`fetch/` と `inbox/` が**無い**状態から起こす。`fetch/` の作成、`.lock` の作成と錠、`inbox/` と `.partial` の作成、rename、`statvfs` の各段の成否を出す。次に、根（`ikari-ime-spike/`）に対する `create_dir_all`（既にある場合）と、根の `lstat` が通るかを見る。最後に、例外の外（`ikari-ime-spike/other`）への書き込みが拒まれることを見る（絞りが効いていることの確認）。
3. **(b) ホーム**: Sandbox の中での `HOME` と `pw_dir` を出す。`HOME` がコンテナ（`~/Library/Containers/…ikari.data.spike/Data`）なら §3.4 の決定どおり。ついでに、作ったファイルに `com.apple.quarantine` の拡張属性が付くかを `xattr -l` で見る（§9 の推測）。
4. **(c) team 署名**: 2・3 を team の署名でも流し、結果が ad-hoc と同じかを見る。
5. **(d) 置き場所**: 検証用の束（`/tmp` の外。スクラッチの場所に `Spike.app/Contents/Helpers/` を作り、外側を ad-hoc で署名する）の Helpers に置いた exe を、束の外の Terminal から起こす場合と、束の中の別の exe（設定アプリの代わりの最小の exe。Sandbox なし、Hardened Runtime）から `posix_spawn` で起こす場合の 2 通りで、2〜3 が同じかを見る。`/Library/Input Methods` 相当の場所（0077 §8 の 6）は、配布の確認（#241）と一緒に見るので、ここでは見ない。
6. 通信: 2 の exe で、0076 §16 と同じ公開の小さなファイル（https）を取れることを 1 回だけ見る（`network.client` と一時例外を一緒に付けても通信が通ること）。
7. 片づけ: `ikari-ime-spike/` とコンテナ（`~/Library/Containers/…ikari.data.spike`）を消す。

**結果で変わるもの**（2026-11-20 に閉じた。§8）: (a) は通ったので、§3.5 は既定の形（`ikari-data` が作る）に確定した。代わりの案は記録として残すだけ。(b) は `HOME` がコンテナだったので §3.4 のとおり。(c) は team も同じだった。(d) は置き場所で違いが無かった。

## 6. 推測（確かめていない）
- （確かめた。§8 の 1）一時例外のパスに書いたディレクトリそのもの（`fetch/`）を、Sandbox の中から新しく作れる。
- （確かめた。§8 の 3）Sandbox の中の `HOME` はコンテナを指す。
- Sandbox の exe が作ったファイルには隔離の印（`com.apple.quarantine`）が付くことがあるが、エンジンの取り込み（読む、rename、LM の子に fd で渡す）は印に左右されない。→ (b) で印の有無だけ見る。印が付くことは確かめた（§8 の 4）。取り込みが左右されないかは、§7 の 5 の実機の確認で見る。

## 7. 実装のレーンと順序（1 タスク 1 ブランチ。#489 の下）

0. **スパイク**（§5。Opus でも Sonnet でもよい。結果を §8 に書く PR は文書だけ）。結果で §3.5 の形を決める。
1. **契約コミット**（リード。小さい）: `lib.sh` の `DATA_ID`、`templates/ikari-data.entitlements` と `templates/ikari-data.Info.plist`、`shell-ffi` のヘッダの宣言とスタブ（panic しない。`start` は起こせない失敗（`tool` の文）を返す）、共有の核の移動の置き場所（`shell-core` の `settings::fetch` のモジュールの宣言）。ここで §3.6 のシグネチャを固める。
2. **並行実装**（worktree。持つファイルを分ける）:
   - **レーン M（macOS のビルドと検査。Sonnet）**: `macos/scripts/build.sh`（ビルド、Info.plist の埋め込み、配置、署名、`verify_data`）。持つのは `macos/scripts/`。
   - **レーン C（共有の核の移動。Sonnet）**: `windows/settings/src/fetch.rs` と `text.rs` の取得の部分を `shell-core` へ移し、`windows/settings` はそれを使う。中身は変えない（テストも一緒に移す）。持つのは `shell-core/`、`windows/settings/src/fetch.rs`・`text.rs`・`Cargo.toml`。Windows のレーン（#449）が `windows/settings` を触っていれば、その統合を待ってから始める。
   - **レーン F（`ikari-data` の macOS の分。Sonnet）**: §3.4 のデータの場所、§3.5 の作り方、親の終わりの見張り。持つのは `fetch/src/workspace.rs` の macOS の分と、新しい小さなモジュール。
3. **レーン S（`shell-ffi` の起動。Sonnet。C のあと）**: macOS の `Launcher` と §3.6 の ABI。持つのは `shell-ffi/`。
4. **統合**（リード）: `scripts/check.sh`（フル）、`macos/scripts/build.sh --signing team --release` の `verify_data`。
5. **実機の確認**（#489 の完了の条件。smoke なし）: ビルドした束の `Contents/Helpers/ikari-data` を、Terminal から `model fetch` のプリセットで 1 回流し、`inbox/<名前>/` ができることと、`fetch/` が片づくことを見る。これは**ユーザーのデータの場所に束を置く**ので、流す前にユーザーに了解を取る（エンジンが取り込み、`models/` に入る。使うモデルは変えない。0060 §2）。
- 設定アプリの画面（ボタン、確認の窓、進み具合）は、この issue に入れない（§10 の Q1）。

## 8. スパイクの結果（2026-11-20。macOS 27.2.1、arm64。PR #501 の `spike/macos-ikari-data-489/`）

ad-hoc と Apple Development（team）の両方で流し、結果は同じだった（ただし同じ識別子で方式を切り替えた場合を除く。下の 7）。

1. **(a) 作成: 通った。** `ikari-ime-spike/` だけがある状態から、`fetch/` の作成、`.lock` の作成と flock、`download.bin` の書き込み、`inbox/` の作成、`inbox/x.partial/` の作成と書き込み、rename、`statvfs` がすべて通った。親が先に `fetch/`・`inbox/` を作った形でも通る（`mkdir` は EEXIST）。例外の外（`ikari-ime-spike/other` の mkdir、根の直下のファイルの書き込み、根の open、ホームの直下の mkdir）はすべて EPERM（絞りが効いている）。→ §3.5 は既定の形に確定。
2. **(a) 根の扱い:** `lstat(根)` は通る（根があり、ディレクトリで、リンクでないことは確かめられる）。`open(根, O_RDONLY)`（根の一覧）は EPERM。根の `mkdir` は、根があれば EEXIST（EPERM ではない）、根が無ければ EPERM で、続く `fetch/` の作成は ENOENT。→ §3.5 に注意を足した。
3. **(b) ホーム:** Sandbox の中の `HOME` はコンテナ（`~/Library/Containers/<識別子>/Data`）。`pw_dir` は本当のホーム。環境変数を空にして起こしても（`env -i`）、`HOME` はコンテナになる（Sandbox の初期化が設定する）。→ §3.4 のとおり。§3.6 の「`HOME` を渡さない」でも動く。
4. **(b) 隔離の印:** Sandbox の exe が作ったファイルとディレクトリ（`inbox/`、`inbox/x/` を含む）に `com.apple.quarantine` と `com.apple.provenance` が付く。エンジンの取り込みが印に左右されないかは見ていない（§6 の推測のまま。§7 の 5 で見る）。
5. **(c) team 署名:** 例外のパスの効き方を含め、ad-hoc と同じ。
6. **(d) 置き場所:** Info.plist を `-sectcreate` で埋めてあれば、`Spike.app/Contents/Helpers/` でもその外でも、Terminal からでも束の中の Sandbox なしの exe の `posix_spawn` からでも、同じに動く。埋めなければどこでも exit 133（SIGTRAP。0076 §16 と同じ）。外側の束の `codesign --verify --deep --strict` が通り、`CodeResources` に Helpers の exe の名前がある（§3.7 の 7 の検査は変えなくてよい）。`/Library/Input Methods` の下は見ていない（#241）。
7. **想定外: 同じ識別子のまま署名の方式（ad-hoc と team）を切り替えると、既にあるコンテナで Sandbox の初期化が止まり、起動が返らない。** 逆向きも同じで、止まったあとは元の方式に戻しても動かなかった。識別子を替えれば動く。原因は確かめていない。→ §9 の限界と #502。
8. **通信:** `network.client` と一時例外を一緒に付けて、`connect(:443)` が通り、子の `curl` の HEAD が 200 を返した。`ikari-data` が使う ureq（`NativeTls` と証明書の確かめ）の通信そのものは見ていない（§7 の 5 で見る）。
9. **片づけ:** スパイクのコンテナ（`~/Library/Containers/` の下）は、`rm -rf` が `Operation not permitted` で消せなかった（データの場所の側の `ikari-ime-spike/` は消した）。→ §5 の手順 7 に足した。

## 9. 結果
- 得るもの: macOS で通信できるのは `ikari-data` 1 つになり、その場所は `fetch/` と `inbox/` だけ、ということを、署名の検査で毎回言える。Windows と macOS の取得の核（行の読み、取り消し、文）が 1 つになる。
- 失うもの・増えるもの: 束の中の署名が 1 つ増え、`build.sh` の検査が増える。`shell-core` が `ikari-resource` に依存する。手元の `cargo build -p ikari-data` の exe は、Sandbox を付けて動かせない（Info.plist が無い）。
- 残る限界: 一時例外は、App Store の審査では認められない種類（ikari は App Store で配らない。0069）。将来 Apple が一時例外を外せば、作り直しになる（エンジンも同じ）。

## 10. ユーザーの判断（2026-11-19 に、Q1〜Q3 すべて推奨どおりで決定）
- **Q1. 設定アプリ（macOS）の取得の画面を、この issue に入れるか。** 推奨: 入れない。この issue は「配る `ikari-data` の権限の絞り」（#99 の前の条件）に絞り、確認は Terminal から行う（§7 の 5）。画面は別の issue（Windows の D-2c に当たる macOS の分）にする。理由: 画面は Swift の UI の作業で量が別で、#99 の前に要るかは別の判断のため。
- **Q2. 取得の核（行の読み、取り消し、文）を、Windows の設定画面の核から `shell-core` へ移して共有するか。** 推奨: 移す（§3.6、§4 の 4）。Windows のコードを動かすので、Windows のレーンが `windows/settings` を触っている間は待つ（§7 のレーン C）。移さない場合は `shell-ffi` に写しを持ち、同じ直しを 2 か所にすることになる。
- **Q3（確認だけ）. 実機の確認（§7 の 5）で、プリセットのモデルを 1 つ、ユーザーのデータの場所の `inbox/` に取ってよいか。** 推奨: よい（エンジンが取り込むが、使うモデルは変わらない。不要なら設定画面か手で消せる）。だめなら、検証用のホームで `ikari-data` だけを流し、取り込みは見ない。

## 参考
- #489、#449、#99。ADR 0076 §3.3・§13・§16、0078 §3.4・Q2、0077 §3.4・§8、0005 §6.2、0060（2026-11-19 の更新。オフライン版）、0067、0069 §6
