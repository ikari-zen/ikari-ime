# ADR 0078: 取得の子（Windows の `ikari-data transfer`）に書く先のハンドルを渡し、そのあと AppContainer に入れる（2 段）

- 状態: 承認（2026-11-19 ユーザー決定。案 B（段階）と、§10 の Q1〜Q3 はすべて推奨どおり）。仮置きのまま残るもの: 大きさの見張りの間隔（§3.1 の 6。契約のレーンで値を置き、PR でユーザーに確かめる）、`privateNetworkClientServer` を足すか（段 2 の VM の確認 3 の結果を見て聞く）
- 日付: 2026-11-19
- 対象: `fetch/`（`ikari-data`。`run.rs`、`child.rs`、`spawn_windows.rs`、`workspace.rs`、`job.rs`）、`windows/sandbox`（`ikari-sandbox`）、`lm/src/host/os/appcontainer.rs`（段 2 で移す。中身は変えない）、[ADR 0076](0076-ikari-data-contract.md) §3.2・§5.1・§11・§17、[security-design.md](../security-design.md) §4。この ADR ではコードを変えない
- 前提: origin/main = ef3befd。[ADR 0076](0076-ikari-data-contract.md)（取得の契約）、[ADR 0072](0072-lm-cage-levels.md) §3・§12（AppContainer の実測）、[ADR 0067](0067-windows-install-layout-and-launch-chain.md)（Program Files の ACL）、[ADR 0070](0070-update-and-vulnerability-response.md) §5（弱い点の詳細を公開の場所に書かない）、#449、#99
- 判断の置き場所: すべて Rust。何を渡し、どの檻で起こし、いつ止めるかは `ikari-data`（`fetch/`）、起動の作り（ハンドルの継承、AppContainer の属性）は `ikari-sandbox`。Swift と C# は触らない
- ほかの ADR との関係: 0076 §3.2（`transfer` の権限と作業の場所）、§5.1 の 1・2、§17（共通化の順）を改める。0072 の LM の段 2 は変えない（AppContainer の部品の置き場所だけ移す）

---

## 1. 分かったこと（2026-11-19。researcher とコードで確かめた）

- `ProductFetcher::fetch`（`fetch/src/run.rs`）は、Windows で自分を `transfer` として `spawn_low`（`fetch/src/spawn_windows.rs`）で起こす。起動は `ikari-sandbox::spawn`（止めて作る → Job → `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` は標準出力のパイプだけ）。整合性は `Integrity::Inherit` と `Low` だけで、AppContainer の値と `SECURITY_CAPABILITIES` は無い。
- 書く先は、引数 `--out <パス>` を子が `transfer_to_file` で `open_plain_file(create_new)` して自分で開く。置き場は `<LocalLow>\ikari-ime\fetch\`（低い整合性が書ける場所）。親は子が終わったあと、同じパスを読み取りで開き、`stage_model_bundle`（`resource/src/stage.rs`）で `inbox/<名前>.partial/` に写しながら sha256 を数える。sha256 の無い自由指定は、写す前に一度数えて名前を作り、写したあとの値と照らす（`job.rs`）。
- D-01 の直し（リンクを辿らない開き方。`fetch/src/nofollow.rs`、`workspace.rs` の `lock()`・`hold_work_chain`）は main にある。
- AppContainer の部品は `lm/src/host/os/appcontainer.rs`（プロファイル、SID、読み取りの許可と取り消し）と `lm/src/host/os/windows.rs`（`SECURITY_CAPABILITIES`。能力 0 個）にある。どちらも `lm` の中だけで見える（`pub(super)`）。LM の子は `ikari-sandbox` にまだ移っていない（0076 §17）。
- macOS の `ikari-data` は子を作らず、自分で通信する。App Sandbox 版の entitlement と、例外の場所の検査（`build.sh`）はまだ無い（0076 §13 の D-2e が未着手）。
- 同じファイルに手を入れている別のブランチ: `fix/low-security-450-fp6`（#450 FP-6。`windows/sandbox/src/os.rs` と `lm/src/host/os/windows.rs` に、子のプロセスとスレッドの DACL を足す）。`lane/d3-win` には、main に入っていない LM の段 2 のコミット（`HostCage`）が残る。

## 2. 問題

- 取得の子は、外から来たバイト列（TLS、HTTP）を解釈する唯一の ikari のプロセス。今の形では、(1) 書く先が低い整合性の書ける場所なので、ほかの低い整合性のプロセスと場所を共有している（妨害の筋が残り、それを D-01 の検査で一つずつ塞いでいる）。(2) 子は低い整合性なので、同じ利用者のファイルを読める（0076 §3.2 と security-design §4 に限界として書いた）。
- (2) を止めるには AppContainer に入れる必要があり、AppContainer の子は、利用者が許した場所しか開けない。子がパスでファイルを開く形のままでは、AppContainer に入れても書く先の許可を別に作ることになる。**子がファイルの場所を一つも要らない形（ハンドル渡し）が、AppContainer の前提になる。** だから段 1 → 段 2 の順にする。

## 3. 決定

### 3.1 段 1: ハンドル渡し（整合性は低のまま）

1. **作業の場所を、Windows でもデータの場所の下（`<データの場所>\fetch\`。中の整合性）にする。** macOS と同じ置き場所になる。`LocalLow` を解決するコード（`local_low`、`work_root` の Windows 版）は消す。古い `LocalLow\ikari-ime\` は移さない・消さない（作者以外の利用者がまだいない。開発機は手で消す）。
2. **`download.bin` は親（`ikari-data`、中）が作る。** `HeldDir` を持ったあと、`open_plain_file(read + write + create_new)` で開き、このハンドル（以下「親のハンドル」）を最後まで持つ。親はこのファイルをパスで開き直さない（読むのも消す前の確かめも、親のハンドルで行う）。
3. **子に渡すのは、親のハンドルから `DuplicateHandle` で作った、`FILE_APPEND_DATA | SYNCHRONIZE` だけの写し**（継承の印なし。`ikari-sandbox` が起動の直前に付け、起動のあとに外す）。子は読めず、切り詰められず、位置を選べない（書いた分だけ大きくなる）。
4. 子への知らせ方は引数 `--out-handle <十進の数>`（継承したハンドルの値は子でも同じ）。Windows の `transfer` は `--out` を受けない（`EXIT_USAGE`）。
5. **親は、写しながら数える今の形（`stage.rs`）を変えない。** 子が書いたファイルを rename で inbox に入れない。理由: 子に渡したハンドルの写しが、子が終わったあとも残りうる（推測。§6）ので、子が書いたファイルは数え終わるまで動かないとは言えない。親が読んで新しいファイルに写したバイト列だけを数え、照らす。
6. **大きさの上限は親が見張る。** 子の進み具合の行とは別に、親が自分のハンドルでファイルの大きさを一定の間隔で読み、`max_size` を超えたら Job を閉じて `too-large` で終わる。間隔は**仮置き**（契約のレーンで値を置き、PR に書いてユーザーに確かめる）。上限の値そのものは今の `max_size`（0060 §4。未定）のまま。
7. 子が終わったら（`wait` のあと）、親は親のハンドルで `FlushFileBuffers` してから先頭に戻して写す。子は `sync_all` を呼ばない（追記だけの権利では失敗する見込み。推測）。最後に `download.bin` は親が消す（`HeldDir` の `cleanup` 経由）。
8. `transfer` が自分の整合性を確かめる規則（低でなければ `refused`）は変えない。

**段 1 だけで消える筋**
- 作業の場所をほかの低い整合性のプロセスと共有すること（リンクや先回りの作成による妨害。D-01・FD-1 の前提）。nofollow の開き方は、同じ利用者の中の整合性の者に対する保険として残す（範囲外の P3 だが、消す理由も無い）。
- `LocalLow` の解決（環境変数で根を向け替えられる筋。D-03）。
- 子が書く先と大きさを選べること。

**段 1 のあとも残る筋（段 2 で止める）**
- 子が同じ利用者のファイルを読めること（security-design §4 の限界の行は残す）。
- 子が、ほかのアプリの低い整合性のプロセスを開けること（ikari の子は FP-6 の DACL で守る）。渡したハンドルの写しを外へ持ち出せること（§3.1 の 5 で、害は「取得の失敗」に留める）。

### 3.2 段 2: AppContainer（能力は `internetClient` だけ）

1. `transfer` を、取得の子だけの AppContainer で起こす。**プロファイルは LM の子と分ける**（名前の案: `io.github.ikari-zen.data-transfer`。LM のプロファイルと同じにすると、同じ AppContainer のプロセスどうしが開き合え、LM に許したモデルのファイルも読めてしまうため）。プロファイルは `ikari-data`（中）が無ければ作る。消すのはアンインストールのとき（LM のプロファイルと同じ扱い）。
2. 能力は `internetClient` の 1 つだけ（`WinCapabilityInternetClientSid`）。`privateNetworkClientServer` は付けない（LAN の中のプロキシが使えなくなる見込み。§6 と VM の確認 3）。読み取りの許可（`grant_read`）は 1 つも付けない。書く先は §3.1 の継承したハンドルだけ。
3. 部品の置き場所: `lm/src/host/os/appcontainer.rs` を `ikari-sandbox` に移し（`ikari_sandbox::appcontainer`。プロファイルの名前を引数にする）、`lm` はそれを呼ぶ形に `use` を替えるだけにする。**LM の子の起動の本体（`lm/src/host/os/windows.rs` の `SECURITY_CAPABILITIES` の組み立て）には触らない。** `ikari-sandbox` にも同じ属性の組み立てを書くので、LM の起動を `ikari-sandbox` に移す（0076 §17）までは、属性の組み立てが 2 か所になる。§17 の移行は別の issue のまま。
4. `transfer` の自己確認: AppContainer で起こしたときは、`TokenIsAppContainer` が真であることも確かめる（偽なら `refused`）。AppContainer のトークンの整合性は低なので、低の確かめはそのまま通る。
5. AppContainer で起こせないとき（プロファイルが作れない、`CreateProcess` が失敗する）は、**段 1（低い整合性）に落として取得を続け、報告の行と設定画面に『檻なし』に相当する印を出す**（2026-11-19 ユーザー決定。Q3）。印の語と文は契約のレーンで決める（報告の行は `resource::fetch_report`、文は核）。
6. 段 2 が済んだら、security-design §4 の「取得の子が利用者のファイルを読める」の限界を消し、「AppContainer がすべてのアプリのパッケージに開いている場所（Program Files、HKCU\Software の一部など）は読める」に替える（0072 §12.4 の LM の段 2 と同じ書き方）。

### 3.3 順序と、#99 に間に合わないとき

- 順序: **FP-6（`fix/low-security-450-fp6`）→ 段 1 → 段 2**。段 1 は段 2 を待たずに main に入れる。段 1 だけでも配れる形にする（段 2 は上に足すだけで、段 1 の形を変えない）。
- 段 2 が #99 の前に VM の確認を通らなければ、段 1 の形で配り、security-design §4 の「約束しないこと」の行（低い整合性の取得の子は利用者のファイルを読める）を残したまま出す。書き方は今の行を使い、「#449 の段 2 で狭める予定」の括弧を残す（2026-11-19 ユーザー決定。Q1）。

### 3.4 macOS の分

0076 §3.3 の `ikari-data` の「データの場所の例外」を `fetch/` と `inbox/` の 2 つだけにし（学習、ユーザー辞書、設定、`models/` を入れない）、`build.sh` の検査で守る。**この件（#449）には入れず、別の issue にして D-2e（`ikari-data` の App Sandbox 版を作るところ）に付け、#99 の前の条件にする**（2026-11-19 ユーザー決定。Q2。比べた案は §5 の 4）。#449 の完了の条件から macOS の行を外す。

## 4. 契約（段 1 の最初のコミットで入れる。シグネチャとスタブ）

### 4.1 `ikari-sandbox`（`windows/sandbox/src/os.rs`）

```rust
pub struct Launch<'a> {
    pub program: &'a Path,
    pub args: &'a [OsString],
    pub environment: Option<&'a [u16]>,
    pub integrity: Integrity,
    pub job: JobLimits,
    pub stdio: Stdio<'a>,
    /// 標準入出力のほかに、子に継承させるハンドル（段 1 で追加）。
    /// `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` に標準入出力と合わせて入れる。継承の印は起動の直前に付け、起動のあと
    /// （失敗でも）に外す。値は子でも同じなので、呼ぶ側が引数で知らせる（ここは引数を作らない）
    pub inherit: &'a [BorrowedHandle<'a>],
    /// AppContainer（段 2 で追加）。`Some` のときは `integrity` が `Low` でなければ `InvalidInput`
    pub container: Option<&'a Container>,
}
```

- エラー: 継承の印を付けられない、`inherit` に同じハンドルが 2 度ある、`stdio` と重なる → `io::Error`（`InvalidInput` か OS のエラー）で、子は作らない。今の「どれか 1 つでもかけられなければ子を止めて失敗」と同じ。
- 段 2 で足すもの（`ikari_sandbox::appcontainer`。`lm` から移す）:

```rust
pub struct ContainerProfile { pub name: &'static str, pub display_name: &'static str }
pub enum Capability { InternetClient }
pub struct Container { /* AppContainer の SID と能力の SID。Drop で解放 */ }
impl Container {
    /// プロファイルが無ければ作り、あれば名前から SID を求める。能力の SID を作る
    pub fn ensure(profile: &ContainerProfile, capabilities: &[Capability]) -> io::Result<Container>;
}
pub fn delete_profile(profile: &ContainerProfile) -> io::Result<()>;
pub fn grant_read(path: &Path, sid: &Container) -> io::Result<()>;   // LM のまま
pub fn revoke_read(path: &Path) -> io::Result<()>;                   // LM のまま
```

- `Integrity` は OS に依存しない `Copy` の列挙のまま残す（今の 2 つの呼ぶ側は `inherit: &[]`、`container: None` を足すだけ）。

### 4.2 `ikari-data`（`fetch/`）

```rust
// spawn_windows.rs
/// `out` の追記だけの写しを作って継承させ、`--out-handle <値>` を引数の末尾に足して起こす
pub fn spawn_transfer(program: &Path, args: &[String], out: &File, confine: Confine)
    -> io::Result<(Confined, PipeReader)>;
pub enum Confine { Low, AppContainer(&'static ContainerProfile) }   // 段 2 で AppContainer を足す

// child.rs
pub struct TransferArgs { pub url: String, pub max_size: u64, pub expect_size: Option<u64> }  // out を外す
// Windows の transfer の引数: `transfer --url … --max-size … [--expect-size …] --out-handle <十進>`

// run.rs（子の側。Windows）
/// 値を HANDLE にし、`GetFileType == FILE_TYPE_DISK` でなければ失敗。所有は子が持つ
fn take_out_handle(value: &str) -> Result<File, FetchError>;
```

- 子の失敗: `--out-handle` が無い・数でない → `EXIT_USAGE`（行を出さない）。ディスクのファイルでない → `internal` の行と終了 1。書き込みの失敗は今の対応のまま。
- 親の見張り: `fn watch_size(file: &File, limit: u64, child: &Confined, stop: &AtomicBool) -> SizeWatch`（別のスレッド。超えたら `child.terminate()` して印を立てる）。`fetch` の結果は、印が立っていれば子の行より先に `too-large`。
- 作業の場所: `Dirs::resolve` の `work` は両 OS で `<データの場所>/fetch`。`Dirs::create_download(&WorkLock) -> io::Result<File>`（`HeldDir` の下に `create_new`）を足し、`download_path()` は Windows の取得の経路から使わない。
- 写す側（`job.rs`）: 親のハンドルを `seek(0)` して読む（sha256 の無い自由指定の 2 回とも）。`stage.rs` は変えない。

## 5. 検討した案

1. **段 2 だけを一度に作る**: 作業は減るが、AppContainer の中の通信が VM で通らなければ、何も入らない。ユーザー決定（案 B）で採らない。
2. **子の出力をファイルに書かず、パイプで親に流す**（親がそのまま写しながら数える）: 追加のハンドルの口も、作業の場所のファイルも、ディスクの 2 倍も要らず、大きさも親が正確に数えられる。採らない理由: 進み具合の行とデータを分ける口（2 本目のパイプか枠）が要り、sha256 の無い自由指定の名前の付け方（数えてから名前を作る）と `stage.rs` を作り直すことになる。データの終わりがパイプの閉じに頼れない（写しが残ると閉じない。推測）ので、子の終了で切る規則も要る。macOS と流れが分かれる。段 1 の目的（場所を共有しない）には、ハンドル渡しで足りる。ディスクの 2 倍が問題になったら見直す。
3. **段 2 で LM の起動も `ikari-sandbox` に移す（0076 §17 を同時に済ませる）**: 属性の組み立てが 1 か所になる。採らない理由: LM の起動は `lane/d3-win` の残りと FP-6 とも重なり、取得の段 2 が LM の確認（GPU、#424）待ちになる。部品（プロファイルと SID）だけ移し、起動の本体は触らない。
4. **macOS の例外の絞りを、この件に入れる / 別の issue にする**:
   - この件に入れる: 利点は #449 の完了の条件（例外が `fetch/` と `inbox/` だけ）をそのまま満たせること、Fable の指摘が 1 つの issue で閉じること。欠点は、`ikari-data` の App Sandbox 版（entitlement、Info.plist の埋め込み、Helpers への配置）がまだ無く（D-2e）、それを丸ごと作ることになること。Windows 最優先の中で、別の OS・別のファイル（`macos/`）のレーンが #449 の完了を待たせる。
   - 別の issue: 利点は、entitlement を作る D-2e と同じ時に、最初から 2 つの例外で書けること（今は直すものが無い）。#449 は Windows だけで閉じられる。欠点は、#449 の完了の条件を書き換えることと、忘れないように #99 の前の条件として結び付ける手間。
   - 決定: 別の issue（D-2e に付け、#99 を待たせる側に置く。2026-11-19）。
5. **AppContainer で起こせないとき、取得を止める**（段 1 に落とさない）: いちばん守りが固い。欠点は、その機械ではモデルを取れず、LM が使えない（LM は入る前提）。採らない（Q3）。

## 6. 推測と、確かめること

**推測（確かめていない）**
- 低い整合性の子は、継承したハンドルの写しを、同じ利用者のほかの低い整合性のプロセスに作れる。AppContainer の子は、ほかの AppContainer やほかのプロセスを開けないので作れない。
- `FILE_APPEND_DATA` だけのハンドルへの `WriteFile` は、常にファイルの末尾に書かれ、`SetEndOfFile` と `FlushFileBuffers` は拒まれる。
- AppContainer（`internetClient` だけ）の中で、SChannel の証明書の確かめ（中間の証明書の取りに行きを含む）、名前の解決、インターネット側のプロキシが動く。LAN の中のプロキシ（`privateNetworkClientServer` が要る）と、同じ機械のプロキシ（AppContainer のループバックの隔離）は動かない。
- AppContainer の子に渡す環境（`TEMP`、`TMP`）は、子からは書けない場所を指すが、取得には要らない。
- 開発用の配置（`%LOCALAPPDATA%\ikari-ime-dev\install`）では、AppContainer の子が自分の実行ファイルを読めない（Program Files だけが、すべてのアプリのパッケージに開いている）。LM の段 2 の開発時の扱いに合わせる。

**段 1 の完了の条件**
- Windows の CI のテスト: (a) `--out-handle` で渡したファイルに子が書け、親が数えた sha256 が合う。(b) 子は渡されたハンドルで読めない・切り詰められない。(c) ディスクのファイルでないハンドル（パイプ）を渡すと `internal`。(d) `max_size` を超えて書く子を、親が止めて `too-large`。(e) 写したあとに元のファイルに追記されても、束の中身と sha256 が変わらない。(f) `ikari-sandbox`: `inherit` に入れないハンドルは子に届かない、入れたハンドルの継承の印は起動のあとに外れる、重なりは `InvalidInput`。
- ソースの検査（`xtask`）: `fetch/src/` に `LocalLow`、`FOLDERID_LocalAppDataLow` が無い。Windows の `transfer` の引数の組み立てに `--out` が無い。
- 0076 §3.2・§5.1 の改め（作業の場所、子は場所を持たない）。security-design §4 の行は残す。
- VM（ふだんのログオン）: プリセットのモデルを最後まで取り、エンジンが取り込む。取り消しで `transfer` が終わり、`download.bin` が次の起動で消える。

**段 2 の完了の条件（VM。ふだんのログオン（セッション 1）で。管理者の SSH だけでは足りない）**
1. Hugging Face のプリセットと GitHub の版の確認が、AppContainer の子から最後まで通る（別のホストへの転送を含む。§16 と同じ拒否の 6 種も拒む）。
2. 名前の解決が通る。
3. プロキシの環境変数: インターネット側のプロキシ、LAN の中のプロキシ、同じ機械のプロキシ、`NO_PROXY`。動かない形は README と設定画面の限界の文に書く（`privateNetworkClientServer` を足すかは、結果を見てユーザーに聞く）。
4. 継承した追記だけのハンドルに書ける。読めない。
5. 子から、利用者のファイル（文書のフォルダ、`<データの場所>` の学習）、`LocalLow`、LM の子とほかのアプリのプロセスが開けない。
6. 入れ子の Job（設定画面の核 → `ikari-data` → `transfer`）が AppContainer でも動く（0076 §17 の UI の制限の扱いのまま）。
7. ファイアウォール: 規則のない `ikari-data.exe` の AppContainer の子が送信でき、設定画面の exe の送信の禁止は変わらない。送信の記録（0076 §11 の 4）で、行き先が表のホストと転送先だけ。
8. プロファイルの作成（中の整合性の `ikari-data` から）と、アンインストールでの削除。
9. Program Files に置いた `ikari-data.exe` が AppContainer の子として起動できる。
10. プロファイルを作れない状態で、段 1 に落ちて取得が続き、報告の行と設定画面に印が出る（Q3）。

**この件の完了の条件に入れないもの**: macOS の `ikari-data` の例外（`fetch/` と `inbox/` だけ）と `build.sh` の検査は、別の issue（D-2e に付ける。#99 の前の条件）で行う（Q2）。

## 7. 実装の進め方（1 タスク 1 ブランチ。レーンと持つファイル）

| 段 | 中身 | 持つファイル | モデル |
| --- | --- | --- | --- |
| 0（先に） | FP-6 のマージ（`fix/low-security-450-fp6`） | `windows/sandbox/src/os.rs`、`lm/src/host/os/windows.rs` | 実装済み・未マージ（リードがまとめる） |
| 1a 契約 | §4 のシグネチャとスタブ（`Launch::inherit`、`spawn_transfer`、`TransferArgs`、`Dirs::create_download`、`watch_size`） | `windows/sandbox/src/os.rs`、`fetch/src/{spawn_windows,child,workspace,run}.rs` | リード（architect の契約どおり） |
| 1b 実装 | ハンドルの継承、子の受け取り、見張り、作業の場所の移動、テスト、0076 の改め | `windows/sandbox/src/`、`fetch/src/`、`xtask/src/check/product.rs`、`docs/adr/0076-*.md` | Opus（ハンドルの継承とスレッドの見張り） |
| 2a 部品の移動 | `appcontainer.rs` を `ikari-sandbox` に移し、`lm` は `use` だけ替える（中身とテストはそのまま） | `windows/sandbox/src/appcontainer.rs`、`windows/sandbox/Cargo.toml`、`lm/src/host/os/appcontainer.rs`（消す）、`lm/src/host/os/windows.rs` の `use` の行だけ、`lm/Cargo.toml` | Sonnet |
| 2b 実装 | `Launch::container`、能力の SID、`SECURITY_CAPABILITIES`、`Confine::AppContainer`、自己確認、段 1 への落とし方と印（Q3）、プロファイルの削除の口 | `windows/sandbox/src/os.rs`、`fetch/src/`、アンインストールの helper | Opus |
| 2c 確認 | §6 の段 2 の 1〜9（`docs/windows-real-device-check.md` に足し、`user-needed` の issue） | 文書だけ | リード |

- 衝突の注意: `windows/sandbox/src/os.rs` は 0・1b・2b が、`lm/src/host/os/windows.rs` は 0・2a が触る。**この順で 1 本ずつマージし、次のレーンは main を取り込んでから始める**（並行にしない）。2a は `lane/d3-win` の残り（`HostCage`）が入るなら、その後にする（`lm/src/host/os/` が重なる）。
- 1b と 2a は持つファイルが重ならないので、並べてよい（`windows/sandbox/Cargo.toml` だけはリードが統合で直す）。
- レーンへの依頼には「直すまで、弱い点の詳細を公開の issue・PR・コミットメッセージに書かない」（0070 §5 の 3）を書く。

## 8. リスクと戻し方

- 段 2 が VM で通らない（AppContainer の中で TLS かプロキシが動かない）: 段 2 のブランチを入れず、段 1 で配る（§3.3）。段 1 は段 2 に依存しないので、戻すものは無い。2a（部品の移動）だけ入っても害は無い（LM のふるまいは変わらない）。
- 段 2 を入れたあとに、特定の機械で動かないと分かった: Q3 の決定のとおり段 1 に落とし、印を出す。 `transfer` の中身（`--out-handle` で書く）は同じなので、落としても書き方は変わらない。
- LM への影響: 2a は `use` の付け替えと関数の置き場所だけ。LM の起動の本体、整合性の決め方、`lm_spawn_single` の検査は変えない。2a で LM のテスト（Windows の CI。段 2 のテストを含む）が通ることを条件にする。
- 見張りの間隔の分だけ、上限を超えて書かれうる（ディスクの見積もり §8.3 の余裕の中に収まるかは、値を置くときに確かめる）。
- 親のハンドルは取得の間ずっと開いている。取り消し（核が `ikari-data` の Job を閉じる）では、親も子も OS が閉じる。残った `download.bin` は次の起動の `cleanup` で消える（今と同じ）。

## 9. 結果

- Windows の取得の子は、ファイルの場所を一つも持たず、渡された 1 つのハンドルに追記するだけになる。作業の場所は両 OS でデータの場所の下にそろう。
- 段 2 のあと、外から来たものを解釈する子は、通信と、その 1 つのハンドルにしか届かない。
- `ikari-sandbox` に AppContainer が入り、LM の起動を移す（0076 §17）ときの土台になる。それまで属性の組み立ては 2 か所。
- 失うもの: `ikari-sandbox` と `fetch/` の Windows のコードが増える（見張りのスレッド、ハンドルの写し）。段 2 では、LAN の中と同じ機械のプロキシが使えない見込み（推測。VM で確かめる）。

## 10. 決定（2026-11-19。ユーザー。すべて推奨どおり）

- **Q1**: 段 2 が #99 の前に VM を通らなければ、段 1 の形で配り、security-design §4 の「約束しないこと」に「低い整合性の取得の子は利用者のファイルを読める」を残して出す（§3.3）。
- **Q2**: macOS の `ikari-data` の App Sandbox 版（例外は `fetch/` と `inbox/` だけ。`build.sh` で検査）は別の issue にし、D-2e に付け、#99 の前の条件にする。#449 の完了の条件から macOS の行を外す（§3.4）。
- **Q3**: 段 2 のあと、AppContainer で起こせない機械では、段 1（低い整合性）に落として取得を続け、報告の行と設定画面に印を出す（§3.2 の 5）。

### （記録）決定の前の問い
- Q1. 段 2 が #99 に間に合わなければ、段 1 で配り、限界を書いて出すか。推奨: 出す。
- Q2. macOS の例外の絞りを、この issue に入れるか、別の issue にするか。推奨: 別の issue（今は entitlement そのものが無く、直すものが無いため）。
- Q3. AppContainer で起こせない機械で、段 1 に落とすか、取得を止めるか。推奨: 落として印を出す（LM は入る前提。0072 §12.8 の 1 と同じ）。

## 11. 実測（2026-11-20。VM。#505 の確認）

詳細な出力は #505 のコメント。§6 の推測のうち、次の点が外れた、または決まった。

- **(a) LAN への到達**: `internetClient` だけの AppContainer は、Public のネットワークでは、同じサブネットも含めて LAN に届く。Private のときは、同じサブネットだけ届かず（`privateNetworkClientServer` を足すと届く）、別のサブネットには届く。VM のホストの LAN 側とルーターの 80 番に届いた。権限なしの AppContainer は、全部を即座に拒否する（10013）。§3.2 の 2 と §6 の「LAN には届かない」の見込みは外れ。
- **(b) 判断（ユーザー決定 2026-11-20 = 判断 1 の A）: LAN への到達は、ファイアウォールの規則で塞がず、既知の限界として受け入れる。** 理由: 規則は管理者権限が要り、残骸が残りうり、企業の管理下では失敗しうるなど、運用の負担が大きい。取得の子が乗っ取られたときの、LAN への足がかりは残る。段 2 の価値は、乗っ取られた取得の子が、利用者のファイルとほかのプロセスに触れないこと。
- **(c) 開発用の配置**: `%LOCALAPPDATA%\ikari-ime-dev\install` などの、読み取りの許可が無い配置でも、AppContainer の子は起動し、取得が最後まで通った（§6 の推測は外れ。実行ファイルのイメージは親の権限で開かれるためと思われる。推測）。
- **環境変数**: 子に要るのは `SystemRoot` と `LOCALAPPDATA`。`LOCALAPPDATA` だけ足せば起き、`USERPROFILE` だけでは 203 で起きない。`USERPROFILE` は渡さない。
- **同じ機械のプロキシ**は、AppContainer では使えない（ループバックの隔離）。動かないプロキシの変数は、子が受けて使う（`NO_PROXY=*` で外れる）。
- **(d) 未確認**: LAN の中のプロキシ、インターネット側のプロキシ、https から http への転送、設定画面の「隔離なし」の表示、インストーラからの `cage-profile-delete`、Domain のネットワーク、実機の LAN で同じサブネットにいる場合。
