# ADR 0040: Windows で動かす（実現性の調査と、殻・エンジン・配布の方針の案）

- 状態: 提案
- 日付: 2026-10-30
- 対象: 方針だけ（コードは変えない）。採ったときに触るのは、`core/src/user/`（パスと権限）、`engine/`（ロック、ログ、権限）、テストの symlink、CI、新しい `windows/`（TSF の殻）。macOS の殻は変えない
- 前提: main = b36d647。spec「OS別の実装メモ」の Windows（TSF）、NFR-14（エンジンは別プロセス）、NFR-40（Windows 11）、プロセス間通信の表（Windows は名前付きパイプ）、NFR-41・42（配布と署名。#99）。ADR 0004 §3（殻とエンジンの分け方）、ADR 0006（文節の編集。Windows の殻は範囲外）、ADR 0021（LM の先行計算と時間の予算）、ADR 0026（英語のモード）
- タスク: #81

---

## 0. 現状の把握

### 0.1 リポジトリの中（確認済み。行番号は main = b36d647）
- workspace は 13 crate。`dict-format`、`input`、`core`、`proto`、`stack` は純 Rust（fst、memmap2、jiff、prost。`protox` も純 Rust で cmake は要らない）。
- エンジンは `engine-ffi`（staticlib。C ABI で `Envelope` の protobuf のバイト列を受け渡す）。macOS ではそれを XPC サービスの中で動かす。**Rust 側にソケットや名前付きパイプの層はない**（通信は殻の側の責務）。
- Unix 専用で、Windows では通らない箇所:
  - `core/src/user/util.rs:5`（`DirBuilderExt`、`OpenOptionsExt`、`PermissionsExt`。0700 / 0600）
  - `engine/src/lock.rs:7`、`:70`（テスト）、`engine/src/log.rs:18`、`:607`、`engine/src/lib.rs:439`
  - テストの `std::os::unix::fs::symlink`: `engine/src/tests.rs:321`、`:366`、`engine-ffi/src/lib.rs:215`、`import/src/io.rs:87`
  - `eval/src/rss.rs` は `cfg(unix)` で分けてある
- ロックは標準の `File::try_lock`（`engine/src/lock.rs:46`）。Windows でも動く API（LockFileEx）。
- ユーザーのディレクトリ（`core/src/user/dir.rs`）は、macOS は `~/Library/Application Support/ikari-ime`、それ以外は `$XDG_DATA_HOME` か `~/.local/share`。`%APPDATA%` / `%LOCALAPPDATA%` の分岐はない。
- LM は `lm` crate の feature `llama`（CPU）と `metal`。Metal は feature の後ろに隔離済み。
- **殻の中身の大きさ**: `macos/Sources/IkariShellCore`（入力の状態機械、キー割り当て、設定、ショートコード、英語のモードなど）はテストを除いて約 4,800 行の Swift。うち `InputStateMachine.swift` が約 1,500 行。AppKit には依存しないが Swift で、**Windows の殻からそのまま使えるものではない**。`IkariIME`（IMK と候補窓）は約 2,600 行。
- CI は `.github/workflows/codeql.yml`（ubuntu-latest、Rust の CodeQL）だけ。`x86_64-pc-windows-msvc` での `cargo check` はまだ流していない。
- spec の Windows の記述: TSF の COM の DLL を `regsvr32` で登録、x64 と x86 の DLL（ARM 版なら ARM64 も）、ストアアプリには「ALL APPLICATION PACKAGES」の権限、周辺テキストは `ITfRange::GetText`、候補窓の位置は `ITfContextView::GetTextExt`。spec の末尾は「`windows/`（TSF の殻、C++）は Windows 版の段で足す」としている。

### 0.2 調査時点の観察（Web。2026-10-30。researcher が curl で取れた範囲。各リンクの中身は変わりうる）
- Microsoft の「IME requirements」: 新しい IME は TSF で作る。IMM32 で作った IME はシステムが止める。IME の DLL は各アプリのプロセスに読み込まれ、app container（ストアアプリなど）の中ではネットワークが使えない。辞書の更新や共有は、別のデスクトップのプロセスに任せる。ストアアプリか否かは `ITfThreadMgrEx::GetActiveFlags` の `TF_TMF_IMMERSIVEMODE`。モードの表示は Input Indicator（`GUID_LBI_INPUTMODE`）。<https://learn.microsoft.com/en-us/windows/apps/design/input/input-method-editor-requirements>（Windows 11 専用の新しい規約は見つからなかった）
- Mozc の TIP（`src/win32/tip`、C++ 約 60 ファイル）: 周辺テキストは `ITfEditSession`（`TF_ES_SYNC | TF_ES_READ`）と `ITfRange` で取る。オンとオフは `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`。<https://github.com/google/mozc/tree/master/src/win32/tip>
- azooKey-Windows（有志、MIT、Rust と Swift）: TSF の DLL（Rust、`windows` crate の `implement`）、変換のサーバー（gRPC）、候補窓（別プロセス）、起動役に分かれる。学習と辞書の登録は未実装。「開発中で安定性の保証なし」。開発は VM を勧めている（IME が落ちると Windows が固まるため）。<https://github.com/fkunn1326/azooKey-Windows>
- Azure Artifact Signing（旧 Trusted Signing）: 公開の信頼の証明書は、**個人は米国とカナダに住む人だけ**。日本は組織なら対象。<https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart>。料金（Basic 月 9.99 ドル）は researcher の報告で、本 ADR では一次情報を確かめていない。
- GitHub Actions に `windows-latest`（x64）と `windows-11-arm` のランナーがある（researcher の報告）。
- 推測（確かめていない）: TIP の DLL は使用中だと置き換えられないので、更新には再ログオンか再起動が要る。署名があっても、配り始めは SmartScreen の警告が出うる。MSIX は TIP の登録に向かない。

## 1. 問題の定義
- spec は macOS → Windows → Linux の順を決めているが、Windows でどこまで今の資産が使えるか、何を作り直すかが分かっていない。
- 作り直すのは「OS の殻」だけ、というのが NFR-14 と ADR 0004 の前提だった。しかし実際には、**入力の状態機械とキー割り当ても Swift の殻の側にある**（§0.1）。Windows の殻は TSF の COM だけでなく、この 4,800 行分の振る舞いも持つ必要がある。ここが Windows 版の費用の大半になる見込み（推測）。
- 配布（#99）の署名は、macOS と違って個人では取りにくい（§0.2）。
- 作者の手元に Windows の実機がない前提で、検証の方法を決める必要がある。

## 2. 決定（提案）

### 2.1 段階を分ける（W0 → W1 → W2）
1. **W0: コアを Windows でビルドとテストを通す**（LM なし）。§0.1 の Unix 専用の箇所を `cfg` で分け、Windows ではユーザーのディレクトリを `%APPDATA%\ikari-ime`（学習と設定）にする（辞書とモデルを `%LOCALAPPDATA%` に分けるかは §6 の S5 で決める）。権限は Windows では既定の ACL（自分のユーザーのプロファイルの下）に任せ、0600 に相当する処理は持たない。CI に `windows-latest` で `cargo test --workspace`（LM なし）を足す。**macOS の振る舞いは変えない**。
2. **W1: スパイク（捨てるコード）**。Rust の薄い TIP の DLL と、別プロセスのエンジン（`engine` をそのまま使う常駐の exe）を名前付きパイプでつなぎ、Notepad とブラウザで「読みを打って Space で 1 位を確定」までを通す。§6 の S1〜S4 を測る。
3. **W2: 本実装**。W1 の結果で §2.2〜§2.4 を確かめてから、別の ADR（契約）にする。

### 2.2 殻の構成（案。W1 で確かめる）
- **TIP の DLL（Rust、`windows` crate）**: キーの受け取り、合成中の文字列（composition）、周辺テキスト、候補窓の位置。変換も学習もしない。
- **エンジン（Rust の exe、ユーザーのセッションで常駐）**: 今の `engine` と `proto`（`Envelope`）をそのまま使う。通信は spec の表どおり名前付きパイプ（`FILE_FLAG_FIRST_PIPE_INSTANCE`、`PIPE_REJECT_REMOTE_CLIENTS`、ACL、`GetNamedPipeServerProcessId`）。
- **候補窓**: 別プロセス（エンジンの exe の中か、別の exe）に置く案を第一にする。TIP の DLL はアプリのプロセスに入るので、UI の不具合がアプリを巻き込まないようにするため（azooKey-Windows と同じ分け方）。DLL の中で描く案は、W1 で遅延とちらつきを比べてから決める。
- **入力の状態機械**: Windows 用に Rust で書く（§3.2 の案 S-b）。どこに置くか（TIP の DLL の中か、エンジンの側か）は、キーごとの往復の時間（S1）で決める。

### 2.3 初期の範囲
- Windows 11 の**デスクトップアプリ**（Notepad、ブラウザ、VS Code など）。x64 を基準にし、ARM64 は開発の VM で動かすために同時に作る（§2.5）。
- 後回し: ストアアプリ（app container）、管理者権限のアプリへの表示（UIAccess）、タッチキーボード、x86（32bit アプリ）、IMM32 しか持たない古いアプリ。
- 機能は macOS のフェーズ2 相当（変換、文節の編集、学習、英語のモード）から。設定アプリは後。

### 2.4 LM
- Windows 版は **LM なしを基準**にし、LM（CPU、`llama` feature）は任意にする。LM の時間は macOS の Metal で中央値 28ms（ADR 0021 の冒頭の要約 4）、先行計算の上限は 300ms。**Windows の CPU での時間は測っていない**ので、既定でオンにするかは S4 の実測のあとで決める。GPU（Vulkan など）は `llama-cpp-2` の feature を確かめてから。

### 2.5 開発と検証の環境
- 作者の手元は Apple Silicon の Mac なので、**ARM 版 Windows 11 の VM**（UTM か Parallels）で動かす。ARM 版 Windows では x64 のアプリが x64 の DLL を読むので、VM では ARM64 と x64 の両方の DLL を入れる（推測。S6 で確かめる）。
- CI は `windows-latest` でビルド、テスト、COM の登録まで。キー入力と候補窓は GUI のセッションが要るので、VM の手動の確認にする。
- VM の費用と Windows のライセンスは作者が決める（§9 の Q5）。

### 2.6 配布（#99 との関係）
- **1.0 に Windows を含めない**案を推す（§9 の Q1）。Windows は 1.0 のあとの版で出す。
- インストーラは MSI（WiX）か exe を候補にし、W2 の ADR で決める。署名は、個人では Artifact Signing の公開の信頼が使えない（§0.2）ので、費用と方法を別に調べる（S7）。署名なしで配るかは NFR-42 と同じくユーザーが決める。

## 3. 検討した案

### 3.1 TIP の言語
- **案 T-a（推す）: Rust（`windows` crate）**。コアと同じ言語、同じビルド、メモリの安全。azooKey-Windows が同じ形で動いている（§0.2）。COM の実装は `windows` crate の `implement` で書ける。
- 案 T-b: C++（spec の記述、Mozc）。資料と前例は一番多い。ただし、リポジトリに 3 つ目の言語が入り、ビルド（MSVC、ATL）と保守の費用が増える。spec の「C++」は TSF の前例から書いたもので、比較した記録はない。
- 案 T-c: C#（.NET）。全アプリのプロセスに CLR を載せることになり、TIP には向かない（推測）。
- 案 T-d: Swift on Windows で macOS の殻を流用する。Swift の実行環境を全アプリのプロセスに載せることになり、IMK と TSF の違いで流用できる部分は IkariShellCore に限られる。ツールチェーンの安定性も未知。

### 3.2 入力の状態機械（IkariShellCore 相当）
- 案 S-a: Swift の IkariShellCore を Rust に移し、macOS と Windows で共有する。二重の保守がなくなるのが最大の利点。ただし、毎日使っている macOS の殻の中心を、Windows の見込みが立つ前に書き換えることになり、回帰の危険が大きい。
- **案 S-b（推す）: Windows 用に Rust で書く。共有にするかは W2 の後の別 ADR で決める**。W1 の段では最小の状態機械（読み、Space、確定）だけ。Rust の新しい crate として、OS に依存しない形で書いておけば、あとで macOS から使う道も残る。二重の保守の期間ができる（利子）のは受け入れる。
- 案 S-c: 状態機械をエンジンの側（Rust、別プロセス）に置き、TIP はキーを全部送るだけにする（Mozc の形）。殻が一番薄くなる。ただし、キーごとにプロセスをまたぐので、`OnTestKeyDown` の同期の応答に往復の時間が乗る（S1 で測る）。S-b の置き場所の案として W1 で比べる。

### 3.3 着手の順
- **案 O-a（推す）: W0（コア）→ W1（スパイク）→ W2**。W0 は小さく、macOS にも害がなく（`cfg` の分岐だけ）、CI で Windows の回帰を止められる。
- 案 O-b: いきなり TIP を作る。動くものは早いが、コアの移植の問題と TSF の問題が混ざって切り分けにくい。

## 4. 結果（採ったときに変わること）
- W0 で、Windows のビルドが CI で守られる。macOS の振る舞いは変わらない（`cfg` の分岐とパスの追加だけ）。
- Windows 版の殻は Rust になり、spec の「`windows/`（TSF の殻、C++）」を改める（Q2 の承認のあと）。
- 状態機械が Swift と Rust の 2 つになる期間ができる。同じ評価のシナリオ（`eval` のシナリオ）を両方に流せるようにするのが保守の条件になる（推測。W2 の ADR で決める）。
- 失うもの: 1.0 で Windows の利用者には届かない（Q1 を推す案どおりなら）。

## 5. 契約
W0 には契約は要らない（中のパスと `cfg` だけ）。W2 に入るときに、名前付きパイプの上の枠（長さ＋`Envelope`）と、TIP とエンジンの間の追加のメッセージ（あれば）を別の ADR の契約コミットにする。`proto/ikari/engine/v1/engine.proto` は W1 では変えない。

## 6. 未検証の事項（スパイクで確かめること）
| # | 問い | 方法 | 判断に使う所 |
| --- | --- | --- | --- |
| S0 | LM なしの workspace が `x86_64-pc-windows-msvc` でビルドとテストを通るか。§0.1 以外に通らない所があるか | CI の `windows-latest` | W0 の範囲 |
| S1 | TIP からエンジンへの、キー 1 回の往復の時間（名前付きパイプ）。`OnTestKeyDown` の中で待ってよい長さか | W1 のスパイクでログを取る（中央値と 99%） | §3.2 の S-b と S-c |
| S2 | ブラウザ（Chrome、Edge）と Notepad で、周辺テキストと候補窓の位置が取れるか | W1、VM で手動 | §2.3 |
| S3 | 名前付きパイプの ACL を自分のユーザーに絞ったとき、デスクトップアプリの TIP から接続できるか。ストアアプリ（AppContainer の SID）から接続するには何が要るか | W1 | §2.3 の後回しの解除 |
| S4 | LM（CPU）の 1 回の時間と、先行計算の 300ms の中で効く割合 | `ikari-eval` を Windows（VM と CI）で流す | §2.4 |
| S5 | mmap 中の辞書を Windows で入れ替えられないこと（推測）の確認と、ADR 0033 の「原子的に入れ替える」をどう実現するか（版ごとのディレクトリなど） | W0 のテストと小さな実験 | §2.1 のディレクトリの分け方 |
| S6 | ARM 版 Windows の VM で、x64 のアプリに ikari の x64 の DLL が読まれるか。ARM64EC が要るか | W1、VM | §2.5 |
| S7 | 個人で取れるコード署名の方法と費用、署名なしの DLL と MSI が Windows 11 でどう扱われるか（SmartScreen、Smart App Control） | researcher の一次情報の確認と VM | §2.6、#99 |
| S8 | パスワード欄など secure な入力欄の見分け方。英語のモード（ADR 0026）で `OnTestKeyDown` が FALSE を返せば素通しになるか | W1 | W2 の ADR |
| S9 | 更新のときに使用中の DLL をどう置き換えるか（再ログオンの要否） | W2 の前に VM | §2.6 |

## 7. 実装の進め方（1 タスク 1 ブランチ）
1. 本 ADR の承認（Q1〜Q5）。
2. W0: `cfg` の分岐、Windows のユーザーのディレクトリ、symlink のテストの分岐、CI の Windows のジョブ（1 PR。S0 の結果で分ける）。
3. W1: スパイクのブランチ（main には入れない）と、手順書。結果を本 ADR の追記にする。
4. W2: 契約の ADR → 契約コミット → レーン（TIP、エンジンの常駐の exe と名前付きパイプ、候補窓、インストーラ）。

## 8. 懸念点
- 状態機械を 2 つ持つ期間の保守（§3.2）。macOS で足した機能が Windows に届かないずれが続く。
- TIP の不具合は、そのアプリや Windows を固めうる（§0.2 の azooKey-Windows の注意）。開発は VM に限る。
- 作者が Windows を日常で使わない前提だと、実際の利用での不具合に気づきにくい。利用者の報告に頼る比率が macOS より高くなる。
- 署名の費用と、日本の個人で取れる方法が決まらないと、配布の時期が決まらない（S7）。
- 費用の目安（Parallels、UTM、Azure の VM、Windows のライセンス）は researcher の推測で、確かめていない。

## 9. ユーザーに聞くこと
- **Q1. 1.0（#99）に Windows を含めるか**。推す: 含めない。1.0 は macOS だけで出し、Windows は W0〜W2 が済んだ後の版にする。理由: 署名（S7）と状態機械の移植（§3.2）の見通しが立っていない。
- **Q2. Windows の殻（TIP）の言語**。推す: Rust（案 T-a）。spec の「C++」を改める。
- **Q3. 入力の状態機械を macOS と共有するか**。推す: 今は共有しない（案 S-b）。Windows 用に OS に依存しない Rust の crate として書き、共有は W2 の後に別 ADR で決める。
- **Q4. 初期の範囲**。推す: Windows 11 のデスクトップアプリ、x64 と ARM64、LM は任意（既定はオフ、S4 の後で見直す）。ストアアプリ、管理者アプリ、タッチキーボード、x86 は後回し。
- **Q5. 開発の環境と費用**。推す: ARM 版 Windows 11 の VM（まず無料の UTM で試し、足りなければ Parallels）と GitHub Actions の Windows のランナー。Windows のライセンスと VM の費用を使ってよいかを決めてほしい。

## 参考
- spec「OS別の実装メモ」「非機能要件」（NFR-14、NFR-40〜42、プロセス間通信の表）: [ikari-ime_spec.md](../ikari-ime_spec.md)
- [ADR 0004](0004-phase2-macos.md)（殻とエンジンの分け方）、[ADR 0006](0006-segment-editing.md)、[ADR 0021](0021-lm-lookahead.md)、[ADR 0026](0026-english-mode-and-key-stats.md)、[ADR 0033](0033-place-data-distribution.md)（データの入れ替え）
- 外部（調査時点の観察。§0.2）: Microsoft「IME requirements」、Mozc `src/win32/tip`、azooKey-Windows、Azure Artifact Signing の quickstart。他に researcher が挙げた参考: khiin-rs、PIME、saschanaz/ime-rs（本 ADR では中身を確かめていない）
- 関連する Issue: #81、#99
