# ADR 0040: Windows で動かす（殻・エンジン・候補窓・設定画面・配布の方針と、段階の切り方）

- 状態: 承認（2026-11-11。ユーザー決定。§9）
- 日付: 2026-10-30（初版）。2026-11-11 改訂（ユーザーの決定を反映。優先順位、1.0 の定義、進め方、段階の切り方、設定画面、ファイルの形式、門）
- 対象: 方針と段階の切り方だけ（コードは変えない）。採ったときに触るのは、`core/src/user/`、`engine/`、`runmark/`、テストの symlink、`shell-core/src/settings/`、`xtask/`、ファイルを読む箇所（§2.9）、新しい `textfile/`（§2.9）、`pipe/`（名前付きパイプの層）と `windows/`（TIP、常駐のエンジン、候補窓、設定画面）
- 前提: main = e3360ec。spec「OS別の実装メモ」の Windows（TSF）、NFR-02（Space から 1 ページ目まで p99 16ms 以下）、NFR-14（エンジンは別プロセス）、NFR-40（Windows 11）、プロセス間通信の表（Windows は名前付きパイプ）、NFR-41（Windows 版が配れたら 1.0.0）・NFR-42（署名）。ADR 0005（設定アプリ）、ADR 0041（`shell-core`。承認、移行は完了）、ADR 0042（技術選定の方針）、ADR 0043（`cargo xtask`）、ADR 0051（`runmark`）、ADR 0058（リソースの配置）
- タスク: #81。#128（開発環境）。#99（配布）。#138（Rust への移行。完了）

---

## 0. 現状の把握

### 0.1 リポジトリの中（確認済み。main = e3360ec）
- 殻の OS に依存しない処理は、すべて `shell-core`（Rust）にある（ADR 0041 の移行は完了。#138）。入口は「OS のイベント → `Shell::dispatch` → 動作の列」の 1 本で、共通のシナリオ（`shell-core/scenarios/*.json`）で振る舞いを守っている。候補窓の配置の計算（`panel`）、設定の検証・整形・書き換え（`settings/`）、設定アプリとのブリッジの判断（`bridge/`）も `shell-core` にある。macOS の設定アプリ（`IkariSettings`）は画面だけを Swift で持つ。
- ホストの口は `shell-core/src/host.rs` の `TextAccess`（周辺テキスト、選択）と `EngineTransport`（`connect`、`request`、`post`、`close`、`pid`。意味を知らないバイト列の運び手。複数のスレッドから同時に呼ばれる）。Windows で新しく要るのは、この 2 つの TSF と名前付きパイプによる実装と、パイプのサーバーになる常駐のエンジンの exe（macOS の XPC サービスの Swift 約 260 行に当たる）。
- 辞書は合計約 644MB（`vectors.bin` 337MB、`system.dict` 242MB、`grammar.bin` 72MB）を mmap する。アプリごとのプロセスに入る DLL には置けないので、エンジンは別プロセスにする（NFR-14 のとおり）。
- Windows では通らない箇所:
  - `core/src/user/util.rs:5`（`DirBuilderExt`、`OpenOptionsExt`、`PermissionsExt`。0700 / 0600）、`core/src/user/learning.rs` のテスト（`PermissionsExt`）
  - `engine/src/lock.rs:7`、`engine/src/log.rs:18`、`engine/src/lib.rs:480`、`engine/src/rss.rs`（macOS の libc）
  - `runmark`（`signal` が `cfg(unix)` だけ。Windows ではクラッシュの記録が働かない。ADR 0051）
  - `core/src/user/dir.rs`（macOS と XDG だけ。`%APPDATA%` / `%LOCALAPPDATA%` の分岐がない）
  - テストの `std::os::unix::fs::symlink`: `engine/src/tests.rs`、`engine/src/tests/model_check.rs`、`engine-ffi/src/lib.rs`、`import/src/io.rs`
  - `shell-core/src/settings/store.rs` と `eval/src/rss.rs` は `cfg(unix)` で分けてある
- ファイルの改行と BOM: BOM を外しているのは `dict-builder/src/town.rs` だけ。`split('\n')` で行に分ける箇所があり、CRLF だと行末に CR が残る（`xtask/src/eval/snapshot.rs:613`、`shell-core/src/settings/file.rs:19`。後者は行末の CR を保って書き戻す）。`.lines()` は CRLF を扱える。
- キー割り当てのプリセットの TSV は `macos/Resources/keymap/` にある（`shell-core/src/keymap.rs` がテストで読む）。
- `lm` は `llama-cpp-2` の CPU 版で、ビルドに cmake が要る（feature なしならビルドしない）。
- CI（`.github/workflows/ci.yml`）は `macos-latest` だけ。`scripts/check.sh` は `cargo xtask check` の包み（ADR 0043）。
- spec の「リポジトリ構成」は「`windows/`（TSFの殻、C++）」のまま（§4 で直す）。

### 0.2 前例（researcher の調査。2026-10-30、2026-11-11）
- **azooKey-Windows**（MIT、Rust）: client（cdylib の TIP）、server（変換）、ui（候補窓の別 exe）。TIP とサーバーは名前付きパイプの上の gRPC。`windows` crate 0.58、登録は `ITfInputProcessorProfileMgr::RegisterProfile`、配布は Inno Setup、候補窓は tao + wry（WebView2）。成熟度は低い（「開発中で安定性の保証なし」）。開発は VM を勧めている（IME が落ちると Windows が固まるため）。
- **Mozc**（C++、`src/win32/tip`）: 最も成熟。候補窓は別プロセスの renderer。`ITfUIElement` で窓を管理する。周辺テキストは `ITfEditSession`（`TF_ES_SYNC | TF_ES_READ`）と `ITfRange`。オンとオフは `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`。
- Microsoft の「IME requirements」: 新しい IME は TSF で作る。DLL は各アプリのプロセスに読み込まれ、app container の中ではネットワークが使えない。モードの表示は Input Indicator（`GUID_LBI_INPUTMODE`）。
- Azure Artifact Signing の公開の信頼の証明書は、個人は米国とカナダだけ（日本は組織なら対象）。
- 未調査: 署名と SmartScreen、MSIX、使用中の DLL の置き換え（§6 の S7、S9）。

## 1. 問題の定義
- 1.0.0 は「Windows 版が配れたら」（NFR-41）。Windows の実装が全タスクで最優先になった（2026-11-11）が、旧版の本 ADR は「Windows は Rust への移行の後」「1.0 に含めない」のままで、段階が issue にできる粒度になっていない。
- 作者は Windows を日常で使わず、検証は miniPC の上の VM だけ。**実機で確かめられる回数が少ない**ので、Mac で確かめられる部分を先に厚くし、実機の確認は段階ごとにまとめて後で行う必要がある。
- 候補窓と設定画面の方式は、見た目と遅延とアプリへの影響のトレードオフで、実機で試さないと決められない。
- 設定画面を 2 つの OS で作ると、機能がずれやすい。

## 2. 決定

### 2.1 進め方: 「作るだけ作って、Windows 向けは後で実機で確かめる」（2026-11-11 ユーザー決定）
- 各段階の作業を、次の 3 つに分ける。
  - **Mac で検証**: `cfg` の分岐、`cargo check --target x86_64-pc-windows-msvc`（LM なし。`scripts/check.sh` に入れる。§2.6）、共通のシナリオ、OS に依存しない層（枠、多重化、キーの変換表、動作の変換、ファイルの形式）の単体テスト。
  - **CI（`windows-latest`）で検証**: ジョブは W1 の名前付きパイプが通ったあと（W1-d）に作る。それまでは無い。作ってからは `cargo test --workspace`（LM なし）、名前付きパイプの層の単体テスト、COM の登録の成否。GUI のセッションは要らないもの。最初は必須にしない（§2.6）。
  - **実機（VM）が要る**: キー入力、合成中の文字列、候補窓、設定画面、周辺テキスト、アプリごとの挙動、遅延の実測。**段階ごとに「実機の確認待ち」の一覧**（§7 の表）にし、`user-needed` の issue に手順書つきで残す。段階は Mac（あれば CI）の検証が通れば完了とし、実機の確認を待たずに次に進む。CI のジョブができる前に書いた Windows だけのテスト（W1-b など）は、ジョブができたときか実機で初めて流れる。
- **すべての「実機の確認待ち」を閉じることと、全体のセキュリティレビュー（W6）が、1.0.0 の前提**。
- **コードを Mac で検証できる形に分ける**: Windows の crate は、OS に依存しない純粋な部分（`cfg` なし。Mac でテストが回る）と、`#[cfg(windows)]` の薄い層（COM、Win32 の呼び出しだけ）に分ける。薄い層に判断を置かない。

### 2.2 構成
```
アプリのプロセス（各アプリ）
  └─ ikari_tip.dll（windows/tip。Rust、cdylib）
       ├─ com/（cfg(windows)）  TSF の COM、編集セッション、登録。TsfOp を実行するだけ
       ├─ keys, ops             VK と修飾 → ShellEvent、ShellAction → TsfOp（純粋。Mac でテスト）
       ├─ shell-core            状態機械（macOS と共有。§3.2）
       └─ pipe::Client          EngineTransport の実装
            │ 名前付きパイプ（枠 + Envelope。§5）
ikari-engine.exe（windows/engine-host。Rust、ユーザーのセッションで常駐）
  ├─ pipe::Server → engine（今の Engine::handle をそのまま）
  └─ 設定画面の相手（shell-core の bridge と settings。§2.5）
候補窓（方式は §2.4 のスパイクで決める）
設定画面（方式は §2.5 のスパイクで決める）
```
- **TIP の DLL（Rust、`windows` crate）**: キーの受け取り、合成中の文字列、周辺テキスト、候補窓の位置（`ITfContextView::GetTextExt`）。変換も学習もしない。COM の入口はすべて `catch_unwind` で包み、panic はその回を素通しにする（ADR 0041 §4.2。アプリのプロセスを落とさない）。
- **`pipe`（新しい crate。OS に依存しない部分が中心）**: 枠の符号化、要求と返事の対応付け（多重化）、時間切れ、クライアントとサーバー。名前付きパイプの作成と接続（`FILE_FLAG_FIRST_PIPE_INSTANCE`、`PIPE_REJECT_REMOTE_CLIENTS`、自分のユーザーに絞った ACL、`GetNamedPipeServerProcessId`）だけを `cfg(windows)` に置く。**`shell-core` に依存しない**（`EngineTransport` への適合は TIP の側で行う。依存の向きを `windows/tip → pipe`、`windows/tip → shell-core`、`windows/engine-host → pipe`、`windows/engine-host → engine` の一方向に保つ）。
- **常駐のエンジン（`windows/engine-host`）**: 接続ごとにスレッドを持ち、`Engine::handle` に渡す。受け付けの繰り返しは `pipe::Server` の抽象の上に書き、Mac ではメモリ上の双方向の流れでテストする。起動は「ログオン時に起動（HKCU の Run）」を基本にし、TIP が接続に失敗したら、デスクトップアプリ（app container でも管理者でもない）からに限って起動を試みる（S3 で確かめる）。
- gRPC は使わない。`Envelope` は既に長さつきの枠で運べる単位で、gRPC は依存と境界を太くするだけになる（ADR 0042 §1-3）。

### 2.3 1.0.0 の範囲（2026-11-11 ユーザー決定。§9 の Q1）
- Windows 11 の**デスクトップアプリ**（Notepad、ブラウザ、VS Code など）。**x64 だけ**（開発の VM が x64。§2.7）。
- 機能は macOS と同じ `shell-core` の範囲（変換、文節の編集、学習、英語のモード）。共有なので、Windows で足すのは入口だけ。
- **設定画面を含める。macOS と同じレベルの機能で作る**（§2.5）。
- LM は任意で、既定はオフ（§2.6 の LM）。
- 1.0.0 の後: ARM64、ストアアプリ（app container）、管理者権限のアプリ（UIAccess）、タッチキーボード、x86（32bit アプリ）、IMM32 しか持たない古いアプリ。

### 2.4 候補窓（本 ADR では方式を決めない。比較とスパイクの手順）
配置と内容は `shell-core` の `panel` が計算し、窓は描くだけ（ADR 0042 §1-4）。方式は W3 のスパイクで実測して決め、その結果を本 ADR に追記する。

| 軸 | C-a: WebView2（tao + wry）の別プロセス | C-b: 自前描画（Direct2D / DirectWrite） | C-c: `ITfCandidateListUIElement` |
| --- | --- | --- | --- |
| 何か | HTML と CSS で描く別の exe。TIP から位置と内容を送る | Win32 の窓に自前で描く。TIP の DLL の中か、別の exe | TSF の UI 要素の口。アプリが自前で描く（UILess）場合に候補を渡す |
| 見た目 | 作りやすい（CSS） | 手間がかかる（文字の組版、影、角丸を自前で） | アプリ次第 |
| 遅延 | プロセス間の往復と WebView の描画が乗る（未測定） | 最も短い見込み（DLL の中なら往復なし。未測定） | アプリ次第 |
| メモリ | WebView2 のプロセス群が常駐（未測定） | 小さい | なし |
| アプリへの影響 | 窓の不具合がアプリを巻き込まない | DLL の中なら巻き込む。別 exe なら巻き込まない | なし |
| 前例 | azooKey-Windows | Mozc（別プロセスの renderer） | Mozc（UI 要素で窓を管理） |
| 位置づけ | 単独で成り立つ | 単独で成り立つ | **C-a・C-b と組み合わせる口**。多くのアプリは自前で描かないので、単独では窓が出ない |

- スパイクの手順（捨てるコード。main に入れない）:
  1. 同じ候補の固定の列（9 件、注釈あり）を、C-a、C-b（DLL の中）、C-b（別 exe）で表示するだけの最小の TIP を作る。C-c は C-b（DLL の中）に `ITfCandidateListUIElement` を足して、UI 要素を受け取るアプリでの扱いを見る。
  2. 測る: キーを受けてから窓が描き終わるまでの時間（`QueryPerformanceCounter` はプロセスをまたいで比べられる）の中央値と p99、初回の表示（コールドスタート）、常駐のメモリ。
  3. 見る: Notepad、Chrome、Edge、VS Code で、ちらつき、最前面、フォーカスを奪わないか、150% の高 DPI、2 枚のモニタ。窓の側を強制終了したときにアプリが生きているか。
  4. 判断の目安: NFR-02（Space から 1 ページ目まで p99 16ms 以下）に窓の描画を足しても体感で遅れないか。満たす案のうち、見た目とアプリへの影響で選ぶ。結果とユーザーの判断を本 ADR に追記する。

### 2.5 設定画面（1.0.0 に含める。画面の方式は本 ADR では決めない）
- **中身は `shell-core/src/settings/`（と `bridge/`）の Rust をそのまま使う**。検証、整形、ファイルの読み書き、変更の組み立ては Rust。Windows で別に作るのは**画面だけ**（ADR 0042 §1-4、ADR 0041 §2.3 と同じ線引き）。
- 画面の相手（macOS では殻の `IkariBridge`）は、Windows では `windows/engine-host` にする。TIP の DLL はアプリごとにあり、画面が話す相手を 1 つに決められないため。管理の要求（ユーザー辞書、学習の消去など。ADR 0005 §8.3）は engine-host の中でエンジンに渡す。TIP が設定の変更を知る方法（ファイルの変更を見るか、engine-host から知らせるか）は W4-s の issue で決める。
- **機能を両 OS でそろえる守り方**: 新しい設定は、まず Rust（`shell-core/src/settings/` の既定値・検証、必要なら `ikari.settings.v1`）とシナリオ（またはテスト）に足し、両 OS の画面がそれに追従する。画面に出す設定の一覧（キー、種類、節）を Rust に持ち、両 OS の画面のテストで「一覧のキーが全部画面にある」ことを確かめる（一覧の形は W4-s の契約コミットで決める）。片方の OS の画面だけに足す設定は作らない。
- 画面の方式の候補（W4-s のスパイクで比べる）:

| 軸 | U-a: WebView2（HTML と CSS） | U-b: WinUI 3（C#。Windows App SDK） | U-c: Rust の GUI の crate（Slint、egui など） |
| --- | --- | --- | --- |
| 見た目・Windows らしさ | CSS で自由。Windows らしさは自前 | 最も Windows らしい（Fluent） | crate による。Windows らしさは弱い見込み |
| 言語とビルド | Rust ＋ HTML。候補窓が C-a なら部品を共有できる | C# と .NET のビルドが増える | Rust だけ |
| Rust の設定の中身との境界 | 画面 ↔ Rust（同じプロセス、またはパイプ） | C# ↔ Rust の FFI か、パイプ | なし（同じ言語） |
| 前例 | azooKey-Windows（候補窓） | Windows の標準の設定 | 少ない |

  - スパイクの手順（捨てるコード）: 同じ 2 画面（キー割り当ての一覧と、ユーザー辞書の表）を各案で作る。見る: 見た目、**画面の入力欄で ikari 自身を使って日本語が打てるか**、高 DPI、ダークモード、キーボードだけでの操作、配布物の大きさ、ビルドに要る道具。結果とユーザーの判断を本 ADR に追記する。

### 2.6 門、CI、LM、開発の環境
- **`scripts/check.sh` に `cargo check --target x86_64-pc-windows-msvc`（LM なし）を入れる**（W0-c から。2026-11-11 ユーザー決定。§9 の Q3）。ターゲットがなければ `rustup target add` を案内して失敗にする。
  - 注意（未確認の推測）: C や C++ をビルドする build script を持つ crate（feature つきの `llama-cpp-2` など）は、Mac からの msvc ターゲットの `check` で失敗しうる。W0-c で実測し、失敗するなら対象から外し、外したことを `check` の出力に出す。
- **`windows-latest` の CI のジョブは、ある程度機能ができてから作る**（2026-11-11 ユーザー決定）。目安は W1 で名前付きパイプが通ったあと（W1-d）。最初は必須にしない。必須にする時期は、ジョブが安定してから改めてユーザーに聞く。それまでの Windows の検証は、手元の `cargo check`、Mac で流せるテスト、実機の確認待ちだけ。
- **LM**: Windows 版は LM なしを基準にし、LM（CPU、`llama` feature。cmake が要る）は任意で既定はオフ。既定を見直すかは S4 の実測のあと。
- **開発環境**（2026-10-30 ユーザー決定。2026-11-11 再確認）: 作者の miniPC の上の x64 の Windows 11 の VM（#128）。TIP が落ちると Windows が固まりうるので、VM に限る。#81 の本文にある Mac の Parallels（ARM）の案は採らない。

### 2.7 配布（#99 との関係）
- **Windows 版が配れたら 1.0.0**（NFR-41。2026-11-05 ユーザー決定）。
- **署名なしで配り始める**（2026-11-11 ユーザー決定。§9 の Q2）。SmartScreen の警告と回避の手順を README に書き（S7 で実際の表示を記録）、パイプの相手の確認はインストール先のパスで行う。署名の費用と方法は、日本の個人で取れる手段が分かったときに改めて聞く。
- インストーラ（Inno Setup か WiX の MSI）、データの配置（ADR 0058 の束を `%LOCALAPPDATA%` に）、登録、更新のときの DLL の置き換え（S9）は W5 で決める。

### 2.8 判断の置き場所
すべて Rust（`shell-core`、`pipe`、`windows/engine-host`、`windows/tip` の `keys`・`ops`、`textfile`）。TIP の `com/` は TSF の呼び出しと `TsfOp` の実行だけ、候補窓と設定画面は描くだけで、どれも判断を持たない。

### 2.9 ファイルの形式を共通にする（W0-d）
- **書くときは UTF-8、改行は LF**（BOM を付けない。`\r\n` を書かない）。**読むときは LF でも CRLF でも、BOM つきでも読める**ようにする。Windows のメモ帳などで編集されたファイルを、そのまま読めるようにするため。
- BOM を外す共通の関数を 1 つ置く。置き場所は新しい小さな crate `textfile`（`strip_bom` と、BOM を外して読む関数）。`core`、`shell-core`、`dict-builder`、`xtask` のどれからも使えて、依存の向きを増やさないため。`dict-builder/src/town.rs` の BOM の処理もこれに替える。
- 行に分けるのは `.lines()`（CRLF を扱える）にする。直す点: `xtask/src/eval/snapshot.rs:613` の `split('\n')`、`shell-core/src/settings/file.rs:19` の `split('\n')`。後者は行末の CR を保って書き戻しているので、今の挙動（BOM つきのとき最初のキーが読めるか、CRLF の行の値に CR が残らないか、足した行の改行）を先にテストで固め、書き戻しは上の「LF で書く」に合わせる（CRLF のファイルを書き換えると LF になる）。
- **読むファイルごとに「LF」「CRLF」「BOM つき」の 3 通りの共通テスト**を足す。対象: 設定（`settings.tsv`）、学習、プロファイル、ユーザー辞書、取り込み（`ingest_*.tsv`、取り込みの JSONL）、キー割り当てのプリセット、ほかにユーザーが手で編集しうるもの（W0-d の最初に一覧を作る）。3 通りを作る補助はテスト用に `textfile` に置く。

## 3. 検討した案

### 3.1 TIP の言語
- **案 T-a（採る。2026-10-30 ユーザー決定）: Rust（`windows` crate）**。コアと同じ言語とビルド、メモリの安全。azooKey-Windows が同じ形で動いている。
- 案 T-b: C++（Mozc、spec の旧記述）。資料と前例は最多だが、3 つ目の言語とビルド（MSVC、ATL）が増える。
- 案 T-c: C#。全アプリのプロセスに CLR を載せることになる。
- 案 T-d: Swift on Windows。実行環境を全アプリに載せ、流用できる部分はもうない（判断は `shell-core` に移った）。

### 3.2 状態機械の置き場所
- **案 S-a（採る）: TIP の DLL の中で `shell-core` を使う**。macOS と同じ形（殻の中の `shell-core`、エンジンは別プロセス）で、キーごとのプロセス間の往復がなく、エンジンが落ちても入力の状態は残る（ADR 0004 §7.5 の休止がそのまま効く）。共通のシナリオがそのまま Windows の振る舞いを守る。
- 案 S-c（採らない）: エンジンの側に置き、TIP はキーを全部送る（Mozc の形）。`OnTestKeyDown` の同期の応答に毎キーの往復が乗り、macOS と形が分かれる。S1 で名前付きパイプの往復が想定外に遅いと分かっても、S-c ではむしろ悪化するので、採る理由がない。

### 3.3 TIP とエンジンの間の通信
- **案 P-a（採る）: 独自の枠（長さ + 種類 + 通し番号 + `Envelope`）を名前付きパイプに流す**。`EngineTransport` の意味を知らない運び手にちょうど合う。依存が増えない。
- 案 P-b（採らない）: gRPC（azooKey-Windows）。HTTP/2 と tonic などの依存が TIP の DLL（全アプリのプロセス）に入る。得るものは多重化だけで、それは数十行の対応付けで足りる。

### 3.4 BOM を外す関数の置き場所
- **案 F-a（採る）: 新しい小さな crate `textfile`**。使う crate のどれにも意味の合う共通の親がない。
- 案 F-b（採らない）: `input` か `dict-format` に置く。依存の向きは足りるが、ローマ字の変換や辞書の形式と関係のない責務が混ざる。

### 3.5 段階の順
- **案 O-a（採る）: W0（コアとファイルの形式）→ W1（パイプとエンジン）→ W2（TIP の最小）→ W3（候補窓）→ W4（機能の残りと設定画面）→ W5（配布）→ W6（セキュリティレビュー）**。各段階が前の段階の Mac の検証の上に乗り、実機の確認は後でまとめられる。
- 案 O-b（採らない）: 実機で 1 本通るスパイクを先に作る（旧版の W1）。動くものは早いが、実機の確認を先に要し、「作るだけ作って後で確かめる」と合わない。

## 4. 結果（採ったときに変わること）
- W0 で、Windows のビルドが手元の `check` で守られる。macOS の振る舞いは変わらない（`cfg` の分岐とパスの追加、ファイルの読み方の緩和だけ。設定の書き戻しが CRLF を保たなくなるのは例外で、W0-d で明示する）。
- spec の「`windows/`（TSFの殻、C++）」を「`windows/`（TSF の TIP、常駐のエンジン、候補窓、設定画面。Rust と、画面は §2.5 で決めた方式）」に改める（W0-c の文書の変更で行う）。`docs/architecture.md` に crate と Windows のプロセスの構成を足す（その段階の PR で）。
- 設定の機能は Rust と両 OS の画面のテストで同時に守られ、片方の OS だけの設定は作らない。
- 失うもの: 実機の確認と CI のジョブを後に回すので、W2 以降で TSF の前提の誤りが見つかったとき、上に積んだ分を直す手戻りがありうる（§8）。

## 5. 契約
- W0 には契約は要らない（中のパスと `cfg`、ファイルの読み方だけ）。
- **W1 の契約コミット**（レーンの前に入れる）: `pipe` crate の骨組み。
  - 枠: `u32`（LE。以降のバイト数）、`u8` の種類（`Request`、`Post`、`Reply`、`Error`）、`u64`（LE）の通し番号（クライアントが付け、`Reply` は同じ番号を返す）、本体（`Envelope` のバイト列。`pipe` は中を見ない）。枠の上限（定数）を持ち、超えたら接続を切る。
  - 型と trait のシグネチャ: `Frame`、`FrameKind`、`Duplex`（読み書きの双方向の流れ）、`Client`（`request(bytes, timeout)`、`post(bytes)`、`close()`、`server_pid()`）、`Server`（受け付けと、接続ごとの `FnMut(&[u8]) -> Option<Vec<u8>>`）、パイプの名前を作る関数（ユーザーの SID とセッションを含める）。
  - `windows/engine-host` の骨組み（空の `main`）と workspace への追加。
- **W2 の契約コミット**: `windows/tip` の骨組みと、純粋な部分と COM の層の間の `TsfOp`（合成の開始・更新・確定・終了、素通し、候補窓の表示・移動・閉じる）。
- **W4-s の契約コミット**: 画面に出す設定の一覧（§2.5）の型と、engine-host が受ける設定画面のパイプ（名前、`ikari.settings.v1` を運ぶか）。
- `proto/ikari/engine/v1/engine.proto` と `shell-core` の公開の型は、上の追加のほかは変えない（変える必要が出たら、別の issue で）。

## 6. 未検証の事項
| # | 問い | 方法 | 判断に使う所 |
| --- | --- | --- | --- |
| S0 | LM なしの workspace が `x86_64-pc-windows-msvc` で通るか。§0.1 以外にないか。C/C++ の build script が Mac から通るか | Mac の `cargo check --target`（W0-c）。テストは W1-d の CI か実機 | W0 の範囲、§2.6 の対象から外す crate |
| S1 | キー 1 回と Space の往復の時間（名前付きパイプ）。中央値と p99 | W1-d の CI で概算、W2 の実機で本測定 | NFR-02 を満たすか |
| S2 | Notepad、Chrome、Edge、VS Code で周辺テキストと候補窓の位置が取れるか | W2 の実機 | §2.3 |
| S3 | 自分のユーザーに絞った ACL で、デスクトップアプリの TIP から接続できるか。TIP からエンジンを起動してよい場合の見分け | W1-d の CI（接続）、W2 の実機（起動） | §2.2 |
| S4 | LM（CPU）の 1 回の時間と、先行計算の 300ms の中で効く割合 | W4。`ikari-eval` を VM で | §2.6 の LM |
| S5 | mmap 中の辞書を入れ替えられないこと（推測）の確認と、ADR 0058 の入れ替えをどう実現するか | W5 の CI と実機 | W5 |
| S6 | ARM 版 Windows で x64 の DLL が読まれるか、ARM64EC が要るか | ARM64 に着手するとき（1.0.0 の後） | — |
| S7 | 署名なしの DLL とインストーラが Windows 11 でどう扱われるか（SmartScreen、Smart App Control） | researcher の一次情報と VM | W5、§2.7 の README |
| S8 | パスワード欄など secure な入力欄の見分け方。英語のモード（ADR 0026）で `OnTestKeyDown` が FALSE を返せば素通しになるか | W4 の実機 | W4 |
| S9 | 更新のときに使用中の DLL をどう置き換えるか（再ログオンの要否） | W5 の実機 | W5 |
| S10 | 候補窓の方式（§2.4） | W3 のスパイク | W3 |
| S11 | 設定画面の方式（§2.5） | W4-s のスパイク | W4-s |

## 7. 段階と issue の単位（1 ブランチ 1 タスク）
各段階は親 issue 1 つと、下の行ごとの子 issue にする。「並行」と書いた行は、契約コミットのあと、`isolation: worktree` の別レーンで同時に進める。実機の確認は段階ごとに 1 つの `user-needed` の issue（手順書つき）にまとめる。

| 段階 | issue（ブランチ） | 持つファイル | Mac / CI で検証 | 実機の確認待ち |
| --- | --- | --- | --- | --- |
| **W0 コア** | W0-a パスと権限（並行） | `core/src/user/{util,dir}.rs`、`shell-core/src/settings/store.rs` | `%APPDATA%\ikari-ime` の分岐の単体テスト（`resolve_with` に OS を渡す形）、`cargo check --target` | なし |
| | W0-b エンジンと記録（並行） | `engine/src/{lock,log,lib,rss}.rs`、`engine/Cargo.toml`、`runmark/` | `cargo check --target`。`runmark` は Windows では記録を「不明」にして通す（クラッシュの記録は W4） | なし |
| | W0-c テストと門（W0-a・b の後） | テストの symlink の 4 か所、`core/src/user/learning.rs` のテスト、`eval/src/rss.rs`、`xtask/src/check*`、spec と architecture | `check` に `cargo check --target`。S0 を実測し、通らない crate を対象から外す | なし |
| | W0-d ファイルの形式（W0-a の後。W0-b・c と並行） | 新しい `textfile/`、ファイルを読む箇所（`core/src/user/` の読み込み、`shell-core/src/settings/{file,parse}.rs`、`dict-builder/src/town.rs`、`xtask/src/eval/snapshot.rs`、`import/`） | 読むファイルごとの LF・CRLF・BOM つきの 3 通りのテスト（§2.9） | なし |
| **W1 パイプとエンジン** | W1-0 契約コミット | `pipe/` の骨組み、`windows/engine-host/` の骨組み、`Cargo.toml` | ビルド | — |
| | W1-a 枠と多重化（並行） | `pipe/src/{frame,mux,client,server}.rs` | Mac: 枠の往復、上限、壊れた枠、同時の要求、時間切れ、切断（メモリ上の流れ） | なし |
| | W1-b 名前付きパイプ（並行） | `pipe/src/os/windows.rs` | Mac: `cargo check --target`。単体テスト（作成、最初のインスタンス、リモートの拒否、ACL、相手の PID）は書くが、流れるのは W1-d から | W1-d の前なら、実機で単体テスト |
| | W1-c 常駐のエンジン（並行） | `windows/engine-host/` | Mac: 受け付けの繰り返しと `Engine::handle` の結合（テスト用の辞書） | なし |
| | W1-d Windows の CI のジョブ（W1-a〜c の後） | `.github/workflows/ci.yml` | `windows-latest` で `cargo test --workspace`（LM なし）と W1-b のテスト、パイプの上で `Convert` が 1 往復。必須にはしない | S1 の概算 |
| **W2 TIP の最小** | W2-0 契約コミット | `windows/tip/` の骨組み、`TsfOp` | ビルド | — |
| | W2-a キーと動作の変換（並行。W1 を待たない） | `windows/tip/src/{keys,ops}.rs`、キー割り当てのプリセットを OS 共通の場所へ（今は `macos/Resources/keymap/`） | Mac: VK と修飾、配列によらない文字（どの配列でも動く）、`ShellAction` → `TsfOp` の表のテスト | なし |
| | W2-b COM の層（並行。Opus） | `windows/tip/src/com/`、`EngineTransport` と `TextAccess` の適合 | `cargo check --target`。CI: `DllRegisterServer` と解除 | 登録、Notepad で読み → Space → 1 位の確定、S1 の本測定、S2、S3（起動）、落ちたときに固まらないか |
| **W3 候補窓** | W3-0 スパイク（main に入れない） | 捨てるブランチと手順書 | ビルド | S10（§2.4）。結果を本 ADR に追記し、ユーザーが方式を決める |
| | W3-a 本実装 | 決めた方式の crate（`windows/candidate/` など） | 内容と配置は `shell-core` の既存のテスト | 表示、選択、ページ、高 DPI、2 枚のモニタ |
| **W4 機能の残り** | 英語のモードとモードの表示（Input Indicator、オンとオフの compartment）、secure な欄（S8）、再接続、クラッシュの記録（`runmark`）、LM（S4）。issue は 1 機能 1 つ | `windows/tip/`、`runmark/`（機能ごとに分ける） | 共通のシナリオ（既存）、`cargo check --target` | 機能ごとの手順 |
| **W4-s 設定画面** | W4-s0 スパイク（main に入れない） | 捨てるブランチと手順書 | ビルド | S11（§2.5）。結果を本 ADR に追記し、ユーザーが方式を決める |
| | W4-s1 契約コミット | 画面に出す設定の一覧（`shell-core/src/settings/`）、設定画面のパイプ | Mac: 一覧と既定値の対応のテスト。macOS の画面のテストを一覧に合わせる | — |
| | W4-s2 engine-host の相手（並行） | `windows/engine-host/` の設定の口 | Mac: `bridge` と `settings` の結合 | なし |
| | W4-s3 画面（並行。画面ごとに issue を分けてよい） | 決めた方式の `windows/settings/` | 一覧のキーが全部画面にあるテスト | 全画面の操作、画面の入力欄での ikari、変更が TIP に効くか |
| **W5 配布** | データの配置（ADR 0058 の束、`cargo xtask`）、インストーラ、登録と更新（S9）、署名なしの扱い（S7） | `xtask/`、`windows/installer/` | CI でインストーラを作る | 新規のインストール、更新、削除、SmartScreen の表示 |
| **W6 セキュリティレビュー（全体）** | Windows の実装の全体（パイプの ACL と相手の確認、全アプリに入る DLL、設定画面の口、インストーラの権限、DLL の探索の順、更新） | — | — | 指摘の確認。**すべての実機の確認待ちが閉じてから** 1.0.0 |

- W0 は契約が要らず、W0-a と W0-b は触るファイルが重ならない。W0-c は `check` に Windows のターゲットを入れるので a と b の後。W0-d は W0-a と `core/src/user/` で重なるので、W0-a の後。
- W2-a は `pipe` に依存しないので、W2-0 の後すぐに始めてよい。W4-s は W2 の後ならいつでも始めてよい（W3 と並べてよい）。

## 8. 懸念点
- **実機の確認と CI を後に回す手戻り**: TSF の前提（編集セッションの同期、`OnTestKeyDown` の扱い、起動の方法）が実機で違えば、W2-b と W3 以降を直すことになる。W2-b の実機の確認だけは、W3-a に入る前に済ませるのが安い（ユーザーの都合で前後してよい）。W1-d までは Windows だけのテストが流れないので、W1-b は実機で一度流すと安い。
- `check` に入れる `cargo check --target` は型と借用までしか見ない。リンクと実行の誤りは、W1-d の CI か実機まで見つからない。
- 設定の書き戻しが CRLF を保たなくなる（§2.9）。Windows で CRLF に慣れた利用者がメモ帳で開いても読めるが、差分の道具では全行が変わって見える。
- TIP の不具合は、そのアプリや Windows を固めうる。実機の確認は VM に限る。
- 作者が Windows を日常で使わないので、実際の利用での不具合は利用者の報告に頼る比率が高い。
- 設定画面を 2 つの OS で作るので、画面の不具合は OS ごとに出る。機能のずれは §2.5 の一覧のテストで止めるが、見た目と操作感のずれは止められない。
- `windows` crate の版の上がり方が速い。W1-0 で固定し、上げるときは別の issue にする。

## 9. 決まったこと
- 2026-10-30: **TIP は Rust**（T-a）。**状態機械は macOS と共有**（`shell-core`。ADR 0041）。**開発は miniPC の上の x64 の Windows 11 の VM**（#128。旧 Q5）。
- 2026-11-05: **Windows 版が配れたら 1.0.0**（NFR-41）。旧 Q1 の「1.0 に Windows を含めない」は改めた。
- 2026-11-11:
  - **優先順位**: Windows の実装が全タスクで最優先。変換の質は空いた時間。Rust への移行（#138）は完了。旧版の「Windows は移行の後」（旧 §2.1 の順序）は改めた。
  - **開発環境**: miniPC の上の x64 の VM（再確認）。#81 の本文の Mac の Parallels（ARM）案は採らない。
  - **進め方**: 作るだけ作って、Windows 向けは後で実機で確かめる（§2.1）。
  - **セキュリティレビュー（全体）**: Windows の実装ができた後に、段階の最後（W6）に行う。
  - **候補窓の方式**: 本 ADR では決めず、比較とスパイクの手順にする（§2.4）。
  - **Q1. 1.0.0 の範囲**: Windows 11 のデスクトップアプリ、x64 だけ、LM は任意で既定はオフ。**設定画面は 1.0.0 に含め、macOS と同じレベルの機能で作る**。中身は `shell-core/src/settings/` の Rust を使い、Windows で作るのは画面だけ。画面の方式はスパイクで比べてから決める（§2.5）。ARM64、ストアアプリ、管理者アプリ、タッチキーボード、x86 は 1.0.0 の後。
  - **Q2. 署名**: macOS と同じく署名なしで配り始める（§2.7）。
  - **Q3. 門**: `scripts/check.sh` に `cargo check --target x86_64-pc-windows-msvc`（LM なし）を W0-c から入れる。`windows-latest` の CI のジョブは、ある程度機能ができてから（目安は W1-d）作り、最初は必須にしない（§2.6）。
  - **ファイルの形式**: UTF-8、LF で書き、LF・CRLF・BOM つきのどれでも読む。共通の BOM を外す関数と、読むファイルごとの 3 通りのテスト（§2.9、W0-d）。

## 参考
- spec「OS別の実装メモ」「非機能要件」（NFR-02、NFR-14、NFR-40〜42、プロセス間通信の表）: [ikari-ime_spec.md](../ikari-ime_spec.md)
- [ADR 0004](0004-phase2-macos.md)（殻とエンジンの分け方）、[ADR 0005](0005-macos-settings-and-indicator.md)（設定アプリ）、[ADR 0021](0021-lm-lookahead.md)、[ADR 0026](0026-english-mode-and-key-stats.md)、[ADR 0041](0041-shell-state-machine-in-rust.md)、[ADR 0042](0042-technology-policy.md)、[ADR 0043](0043-xtask.md)、[ADR 0051](0051-crash-exit-record.md)、[ADR 0058](0058-offline-resource-install.md)
- 外部（調査時点の観察。§0.2）: Microsoft「IME requirements」、Mozc `src/win32/tip`、azooKey-Windows、Azure Artifact Signing の quickstart
- 関連する Issue: #81、#99、#128、#138
