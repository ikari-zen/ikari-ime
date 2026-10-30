# ADR 0036: bundle ID を `<旧 ID の頭>.…` から `io.github.ikari-zen.…` に移す（識別子の頭とチームID を 1 か所にまとめ、実機の入れ直しの手順を決める）

- 状態: 承認・実装済み（2026-10-25。§11 の 3 点はユーザーが推奨どおりに決めた。PR-1 で一元化、PR-2 で値を切り替えた。実機の入れ直しは未了）
- 日付: 2026-10-25
- 対象: macOS の殻・エンジン・設定アプリ・smoke の識別子（`macos/scripts/lib.sh`、`macos/scripts/templates/*.plist`、`macos/Resources/*.lproj/InfoPlist.strings`、`macos/Sources/{IkariShellCore,IkariXPC,IkariIME,IkariSettings}`、`macos/tools/select-input-source.swift`、`macos/Tests/*`、`scripts/diag-report.py`）と文書。Rust の core、エンジンのロジック、辞書、学習ファイルの形式は変えない
- 前提: main = ec634b9。ADR 0004 §5.1・§15.1（署名とチームID、Bundle ID の決定）、ADR 0005 §6.2・§6.3（設定アプリの identifier とブリッジの署名の条件）、ADR 0026 §5.1・§12（日本語と英語のモードの ID）、ADR 0034（診断ログの `app=`）
- タスク: #187（依存: #126 アカウントの移行、#115 個人情報のチェック）
- ユーザー決定（2026-10-25）: GitHub を ikari-zen に移す（リポジトリは履歴なしの新規）。ドメインは持たない。bundle ID の頭は `io.github.ikari-zen`。入力メソッドは `<頭>.inputmethod.<製品名>` の形（azooKey は `dev.ensan.inputmethod.azooKeyMac`、Google 日本語入力は `com.google.inputmethod.Japanese` で確認済み）

---

## 0. 現状の把握

### 0.1 コードで確かめたこと（事実。main = ec634b9 の `git grep`）
- 識別子は **2 つの定義（`lib.sh` と `IkariXPC/Identifiers.swift`）と、それを写した多数の文字列**でできている。Info.plist は**テンプレートに値を直書き**しており、`build.sh` が書き換えるのは版、`IkariSigningMode`、`IkariPeerRequirement` だけ。つまり今は「頭を変える」と、plist 3 つ、InfoPlist.strings 2 つ、Swift 7 ファイル、テスト 3 ファイル、ツール 1 つ、Python 1 つ、文書が一緒に動く。
- 署名の条件の文字列は `IkariXPC.SigningRequirements` が唯一の出どころ（`build.sh` は `ikari-smoke --print-requirement` で受け取り、`codesign --verify -R` で実際の署名と突き合わせる）。**Swift の識別子と `lib.sh` の識別子がずれると、ビルドの検査で落ちる**（ずれを黙って通す道はない）。
- チームID（`lib.sh` の `TEAM_ID`）も `lib.sh` と `Identifiers.swift` の 2 か所。`scripts/setup.sh` は `lib.sh` を `sed` で読む（`readonly TEAM_ID="…"` の形に依存）。
- **UserDefaults は使っていない**（`UserDefaults`、`@AppStorage`、`suiteName` はコードにない）。設定は `~/Library/Application Support/ikari-ime/settings.tsv`（ADR 0005 §7。書き手は殻）。
- **学習ファイルとユーザーデータの置き場所は bundle ID に依存しない**: Rust は `core/src/user/dir.rs` の `APP_DIR_NAME = "ikari-ime"`、エンジンは `IkariEngineService/main.swift` の `dataDirectoryComponents`（`getpwuid` で実際のホームを取る。Sandbox のコンテナではない）、Sandbox の一時例外は `engine.entitlements` の `/Library/Application Support/ikari-ime/`（固定の文字列）。ブリッジのソケットも `ikari-ime/run/bridge.sock`。
- launchd のラベルを持つ plist（LaunchAgents）はない。同梱の XPC サービスは launchd がその bundle ID（= サービス名）で起こす。
- プロセス名（`ikari`、`IkariEngine`、`IkariSettings`、`ikari-smoke`）とアプリの置き場所（`~/Library/Input Methods/ikari.app`）は bundle ID と別。`install.sh` は `killall` をプロセス名で行う。
- IkariModeSpike（`<旧 ID の頭>.inputmethod.ikarispike`）は ADR 0026 の記述にだけ残り、コードはもうない。

### 0.2 Swift のターゲットの依存（事実。`macos/Package.swift`）
`IkariShellCore` がいちばん下。`IkariXPC` → `IkariShellCore`。`IkariSettings` は `IkariXPC` に依存しない。英語のモードの ID（`InputModeID`）は `IkariShellCore/EnglishMode.swift` にある。したがって**頭の定義を `IkariXPC/Identifiers.swift` に置くと、`IkariShellCore` と `IkariSettings` から使えない**（依存を逆にするか、写しが残る）。

---

## 1. 決定の要約（提案）

| 項目 | 提案 |
| --- | --- |
| 新しい頭 | `io.github.ikari-zen`（ユーザー決定） |
| 製品の ID | `io.github.ikari-zen.inputmethod.ikari`。他はこの後ろに足す（§2 の表） |
| 一元化 | 頭（`IKARI_ID_PREFIX`）とチームID を、**シェルは `lib.sh`、Swift は `IkariShellCore/Identity.swift`（新設）の 1 か所ずつ**に置く。他の識別子は全部そこから組み立てる。テンプレートの plist と InfoPlist.strings は置き場所の印（`@IKARI_ID_PREFIX@`）にして、`build.sh` が埋める。2 か所の値がずれたら `scripts/check.sh` で落とす（§3） |
| 他の人の上書き | 上の 2 か所（4 行）を書き換えれば、全部が自分の値になる。環境変数で上書きする形は入れない（§3.3、§11-1） |
| 移行 | 実機の入れ直しは 1 回。古い入力ソースを外す → 新しいものを入れる → 入力ソースを足す → 英語のモードの履歴を作り直す。学習と設定はそのまま引き継がれる（§4） |
| PR | 2 つに割る。PR-1 = 一元化（値は今の `<旧 ID の頭>` のまま。挙動は変わらない）、PR-2 = 値の切り替えと文書（§6） |
| 文書 | 証明書の ID（CN の括弧内）と証明書の名前は文書から外す。チームID の値は文書に書かず「`lib.sh` の `TEAM_ID`」と指す（§7） |

---

## 2. 識別子の一覧（今の値 → 新しい値）

頭: `<旧 ID の頭>` → `io.github.ikari-zen`。以下 `P` = `io.github.ikari-zen.inputmethod.ikari`。

| 用途 | 今の値 | 新しい値 | 出どころ（今） |
| --- | --- | --- | --- |
| 殻の Bundle ID（`CFBundleIdentifier`、署名の identifier、`IMKServer` の bundle、os_log の subsystem） | `<旧 ID の頭>.inputmethod.ikari` | `P` | `lib.sh` `BUNDLE_ID`、`Identifiers.swift` `shell`、`ikari.Info.plist`、`AppContext.swift`（`Logger(subsystem:)` は `shell` を使う） |
| エンジン（XPC サービス）の Bundle ID = サービス名 = launchd が使う名前、署名の identifier | `<旧 ID の頭>.inputmethod.ikari.engine` | `P.engine` | `lib.sh` `ENGINE_BUNDLE_ID`、`Identifiers.swift` `engine`、`engine.Info.plist` |
| エンジンの Sandbox のコンテナ | `~/Library/Containers/<旧 ID の頭>.inputmethod.ikari.engine` | `~/Library/Containers/P.engine`（初回の起動で作られる） | macOS が bundle ID から作る |
| smoke の署名の identifier、smoke のセッションの `clientBundleID`（診断ログの `app=`） | `<旧 ID の頭>.inputmethod.ikari.smoke` | `P.smoke` | `lib.sh` `SMOKE_ID`、`Identifiers.swift` `smoke`、`IkariSmoke/{main,SegmentE2E}.swift`、`scripts/diag-report.py` `SMOKE_APP`、`scripts/testdata/diag/engine.log` |
| 設定アプリの Bundle ID、署名の identifier | `<旧 ID の頭>.inputmethod.ikari.settings` | `P.settings` | `lib.sh` `SETTINGS_ID`、`Identifiers.swift` `settings`、`IkariSettings.Info.plist` |
| IMK の接続名（`InputMethodConnectionName`） | `<旧 ID の頭>.inputmethod.ikari_Connection` | `P_Connection` | `Identifiers.swift` `inputMethodConnection`、`ikari.Info.plist` |
| 日本語のモード（`ComponentInputModeDict` のキー、`TISInputSourceID`、`tsVisibleInputModeOrderedArrayKey`、表示名の strings のキー） | `<旧 ID の頭>.inputmethod.ikari.Japanese` | `P.Japanese` | `lib.sh` `INPUT_SOURCE_JAPANESE`、`EnglishMode.swift` `InputModeID.japanese`、`ikari.Info.plist`（3 か所）、`{ja,en}.lproj/InfoPlist.strings`、`select-input-source.swift` |
| 英語のモードの `TISInputSourceID` | `<旧 ID の頭>.inputmethod.ikari.English` | `P.English` | `EnglishMode.swift` `InputModeID.englishInputSource`、`ikari.Info.plist`、`select-input-source.swift` |
| 英語のモードのキー | `com.apple.inputmethod.Roman` | 変えない（macOS の標準のキー。ADR 0026） | `EnglishMode.swift` |
| 入力ソースの履歴を調べる bundle ID（`prime-english`） | `<旧 ID の頭>.inputmethod.ikari` | `P` | `select-input-source.swift` 104 行（直書き） |
| DispatchQueue のラベル（デバッグ用の名前） | `…ikari.engine-admin`、`…ikari.emoji-table`、`…ikari.bridge-relay`、`…ikari.settings.bridge.send` | `P.engine-admin` など | `AppContext.swift`、`BridgeServer.swift`、`BridgeClient.swift`（直書き） |
| 署名の条件（`team` / `adhoc`） | identifier の部分が上の値 | 上の新しい値で組み立てる（形は変えない） | `SigningRequirements`（`IkariXPC`） |
| チームID | （`lib.sh` の `TEAM_ID`） | **変えない**（Apple Developer のチームは GitHub のアカウントと別） | `lib.sh` `TEAM_ID`、`Identifiers.swift` `teamID` |
| 表示名、プロセス名、`ikari.app` の置き場所、`ikari-ime` のデータのディレクトリ | ― | 変えない | ― |

テストの期待値（`SigningRequirementsTests`、`EnglishModeTests`、`BridgeServerTests` の 319 行）は、新しい値の**文字列を直書きのまま**にする（期待値を `Identity` から組み立てると、組み立ての誤りをテストが見逃す）。頭を上書きした人は、この 3 ファイルの期待値も直す（§3.3 に書く）。

bundle ID に使える文字は英数字、`-`、`.`（Apple の決まり）。`ikari-zen` の `-` はこの範囲。IMK の接続名と XPC のサービス名に `-` を含めても動くことは、まだ確かめていない（推測では問題ない。§5.2 の実機の確認で見る）。

---

## 3. 一元化

### 3.1 決定: 頭とチームID を、シェルと Swift に 1 か所ずつ置く
- **シェル**: `lib.sh` に `readonly IKARI_ID_PREFIX="io.github.ikari-zen"` と `readonly TEAM_ID="…"` を置き、`BUNDLE_ID` などはそこから組み立てる（`"$IKARI_ID_PREFIX.inputmethod.ikari"`）。`IKARI_ID_PREFIX` が `^[A-Za-z0-9.-]+$` に合わなければ `die`。
- **Swift**: `IkariShellCore/Identity.swift`（新設）に `idPrefix` と `teamID` を置き、`productID = "\(idPrefix).inputmethod.ikari"` を組み立てる。`IkariXPC.IkariIdentifiers` は**今の公開の名前（`shell`、`engine`、`smoke`、`inputMethodConnection`、`settings`、`teamID`）を保ったまま**、中身を `Identity` から組み立てる。`InputModeID.japanese` / `englishInputSource` と DispatchQueue のラベルも `Identity` を使う。置き場所を `IkariXPC` ではなく `IkariShellCore` にするのは §0.2 の依存のため（`IkariShellCore` と `IkariSettings` から使える、依存の向きが変わらない）。
- **テンプレート**: `ikari.Info.plist`、`engine.Info.plist`、`IkariSettings.Info.plist`、`{ja,en}.lproj/InfoPlist.strings` の頭の部分を `@IKARI_ID_PREFIX@` にする。`build.sh` はコピーのときに置き換え、置き換えたあとに `@IKARI_` が残っていれば `die`。`ComponentInputModeDict` のキーのように「キーそのもの」が識別子の箇所があるので、`plutil -replace` ではなく文字列の置き換えにする（頭は §3.1 の文字の範囲に限るので、置き換えで壊れない）。
- **`select-input-source.swift`**（`swiftc` で単体でコンパイルする道具で、パッケージに属さない）: 識別子を引数か環境変数で受け取り、`install.sh` が `lib.sh` の値を渡す。直書きをなくす。
- **`scripts/diag-report.py`**: `SMOKE_APP` を頭に依存しない判定（`.inputmethod.ikari.smoke` で終わるか）にする。
- **ずれの検査**: `scripts/check.sh` に「`lib.sh` の `IKARI_ID_PREFIX` と `TEAM_ID` が `Identity.swift` の値と一致する」検査を足す（`sed` で両方を読んで比べる。swift test が流せない環境でも動く）。加えて、§0.1 のとおり `build.sh` の `codesign --verify -R` が実際の署名とのずれを落とす。
- `scripts/setup.sh` の `TEAM_ID` の読み方（`sed`）は、`lib.sh` を `source` する形に変える（`lib.sh` の書き方に依存しない）。

### 3.2 チームID も同じ場所に置く理由
チームID は「誰が署名したか」の識別子で、頭（誰の製品か）と同じく、他の人が自分のものに変える値。変える理由が同じなので同じ場所に置く。今もすでに `lib.sh` と `Identifiers.swift` にあり、置き場所を `Identity.swift` に移すだけ。

### 3.3 他の人が自分の値で使うとき（2026-10-26 に変更: 環境変数で上書きできる形）
- 値を書く場所は `lib.sh` の 2 行だけ。`IKARI_ID_PREFIX` と `TEAM_ID` は環境変数があればそれを使い、なければ既定値（ikari-zen と作者のチームID）を使う。
- Swift の `Identity.swift` は手で書かず、`macos/scripts/gen-identity.sh` が `lib.sh` の値から生成する（`build.sh` が `swift build` の前に呼ぶ）。`swift test` はスクリプトを通らないので、生成したファイルをリポジトリに置く。`scripts/check.sh` は、既定値で生成し直した結果とファイルが同じかを調べる（§3.1 の「ずれの検査」はこれに置き換わった）。
- 他の人は `IKARI_ID_PREFIX=io.github.<名前> TEAM_ID=<自分のチームID> macos/scripts/build.sh …` で作れる。環境変数を与えると `Identity.swift` が書き換わる（作業ツリーが変わる）。テストの期待値は既定値の直書きなので、上書きしたときは swift test の一部（署名の条件の文字列など）が合わない。
- 決めた経緯: 当初は定数の 2 か所（§8 の案 C は見送り）としたが、ユーザーが「ビルドスクリプトが環境変数を読めばよい」と指摘した（2026-10-26）。Swift が実行時に環境変数や Info.plist を読む必要はなく、ビルドのときに生成すれば足りる。

---

## 4. 影響と移行手順

### 4.1 変わるもの・変わらないもの（§0.1 の事実から）
| 項目 | 影響 | 根拠 |
| --- | --- | --- |
| 入力ソース | **macOS からは別の入力メソッドに見える**。新しい ikari を入力ソースに足し直す。古い ID の有効な入力ソースが `com.apple.HIToolbox` の設定に残りうる | `TISInputSourceID` と Bundle ID が変わる |
| 英数キーの行き先（英語のモード） | 入力ソースの履歴（`AppleInputSourceHistory`）は Bundle ID ごとなので、**`prime-english` をもう一度流す必要がある**（ADR 0026） | `select-input-source.swift` の履歴の判定 |
| 設定（`settings.tsv`）、キー割り当て | **そのまま引き継がれる**。UserDefaults を使っていないので、移す作業はない | §0.1 |
| 学習、ユーザー辞書、プロファイル、辞書とモデル、診断ログ | **そのまま**。置き場所は `ikari-ime` の固定の名前 | §0.1 |
| エンジンの Sandbox のコンテナ | 新しく作られる。古いコンテナ（`~/Library/Containers/<旧 ID の頭>.inputmethod.ikari.engine`）は使われなくなって残る。中身は Sandbox の既定のもので、ユーザーデータはない（推測。消す前に中を見る） | macOS の挙動 |
| TCC（アクセシビリティ） | 許可は bundle ID と署名で結び付くので、**新しい ikari は許可がない状態から始まる**（推測。実機で確かめる）。ikari は自分から許可を求めない（NFR-20、ADR 0004 §15.1-6）ので、キャレットの位置の取得が弱いアプリがあれば、ユーザーが許可し直す。古い項目は一覧に残る | ADR 0004 §14-5 |
| 他の UserDefaults の痕跡 | SwiftUI のウィンドウの位置などで `~/Library/Preferences/<旧 ID の頭>.inputmethod.ikari.settings.plist` などができていることがある（推測）。引き継ぐ価値はない | ― |
| os_log | subsystem が変わる。`log show --predicate 'subsystem == "…"'` の値を変える（`docs/handoff-*.md` の例を含む） | `AppContext.swift` 13 行 |
| 診断ログの `app=` | smoke の行が新しい値になる。手元の古い engine.log は旧値のまま（`diag-report.py` は §3.1 の判定で両方を smoke とみなす） | ADR 0034 |

### 4.2 移行の手順（実機。PR-2 のマージの後に 1 回）
**実機に触れる手順は、それぞれユーザーの了解を取ってから行う**（CLAUDE.md「実機に触れるときの約束」）。インストール（`install.sh`、`install-data.sh`）は聞かずに行ってよいが、この移行では入力ソースが入れ替わり、変換できない時間ができるので、**手順全体を始める前に 1 回、ユーザーの了解を取る**。smoke は別にもう一度了解を取る。システムのプロセスを kill しない、入力ソースの設定をエージェントが変えない（入力ソースの追加と削除はユーザーがシステム設定で行う）、ユーザーデータと `data/raw/` を書き換えない。

1. **（リード）事前の控え**: `~/Library/Application Support/ikari-ime/` の一覧と `settings.tsv` の写しを、作業用の場所（scratchpad）に取る（読むだけ）。比べるために使う。
2. **（ユーザー）古い入力ソースを外す**: 入力ソースを ABC か日本語（macOS 標準）に切り替え、システム設定 → キーボード → 入力ソースで「ikari」（日本語と英語の 2 つ）を外す。**新しい ikari を入れる前に行う**。`install.sh` は同じ場所（`~/Library/Input Methods/ikari.app`）に上書きするので、入れた後は古い ID のバンドルがなくなり、システム設定に古い項目が消せない形で残るおそれがある（推測）。
3. **（リード。了解の後）インストール**: `scripts/setup.sh --signing team` のうちビルドとインストール（smoke は除く）。`install.sh` は今の `killall ikari IkariEngine IkariSettings ikari-smoke`（プロセス名。古い ID の殻とエンジンもこの名前で止まる）をそのまま使う。古い ID のプロセスを ID で探して止める処理は足さない（名前で足りる）。
4. **（ユーザー）新しい入力ソースを足す**: ＋ → 日本語 → ikari。出ないときは、ログアウトとログインをし直す（ADR 0004 §14-4）。`select-input-source register` / `enable` は、ユーザーが頼んだときだけ使う。
5. **（リード。了解の後）英語のモード**: `select-input-source prime-english`（入力ソースを一度動かすので了解を取る）。
6. **（ユーザー）確かめる**: 漢字に変換できる、学習が効いている（いつもの語の 1 位）、「ikari 設定」で設定が前と同じ、英数キーとかなキー、メニューのモード表示。開いているアプリは入れ替えの後に再起動が要ることがある。
7. **（ユーザー、任意）TCC**: キャレットの位置がずれるアプリがあれば、アクセシビリティで新しい ikari を許可し、古い項目を消す。
8. **（ユーザーの了解の後）古いものの片付け**: 古いエンジンのコンテナ（`~/Library/Containers/<旧 ID の頭>.inputmethod.ikari.engine`）と古い Preferences の plist。中を見てから消す。ユーザーデータではないが、`~/Library` の下なので了解を取る。HIToolbox の設定に古い ID が残っていれば、システム設定から消えるかを見て、消えなければユーザーに伝える（エージェントは `defaults write` しない）。

---

## 5. 検証

### 5.1 PR ごとの機械の検証（リード）
- `scripts/check.sh`（fmt、clippy、cargo test、swift test、文書のリンク、§3.1 のずれの検査）。
- `macos/scripts/build.sh --signing team --release` と `--signing adhoc`（署名の条件の突き合わせ、identifier とチームID の検査、エンティトルメントの検査が通る）。
- 組み立てた `ikari.app` の Info.plist を `plutil -lint` し、`@IKARI_` が残っていないこと、`CFBundleIdentifier`、`InputMethodConnectionName`、`ComponentInputModeDict` のキー、`TISInputSourceID` が §2 の値であること。
- `git grep -n '<旧 ID の頭>'` が、PR-2 の後は ADR の経緯の記述（§7 で扱うもの）以外で 0 件。
- PR-1 は値を変えないので、組み立てた Info.plist 3 つが main のものと版の行以外で一致すること（`diff`）。

### 5.2 実機の検証（ユーザーの了解の後）
- smoke: `macos/scripts/smoke.sh`（smoke の間、ユーザーの ikari は漢字に変換できない。**了解を取ってから**）。新しいサービス名でエンジンにつながり、署名の条件が通り、engine.log の `begin_session` が `app=P.smoke` になること。
- §4.2 の 6 の確認（入力ソースの追加、IMK の接続名、設定アプリとブリッジ、学習、英語のモード）。IMK の接続名と XPC のサービス名の `-` はここで確かめる。

---

## 6. PR の割り方

**2 つに割る**（どちらも #187 の範囲。PR-2 は PR-1 のマージの後に main から切る。積まない）。

| PR | 中身 | 挙動 | 確かめ方 |
| --- | --- | --- | --- |
| PR-1 一元化 | `Identity.swift` の新設、`IkariIdentifiers` と `InputModeID` とラベルの組み立て、`lib.sh` の頭と組み立て、テンプレートの印と `build.sh` の置き換え、`select-input-source` の引数化、`diag-report.py`、`setup.sh`、`check.sh` のずれの検査。値は `<旧 ID の頭>` のまま | 変わらない（実機の入れ直しは要らない） | §5.1。Info.plist の `diff` が 0 |
| PR-2 切り替え | 頭を `io.github.ikari-zen` に変える（`lib.sh` と `Identity.swift` の 2 行）、テストの期待値、文書（`phase2-contract.md`、`architecture.md`、`macos/README.md`、ADR 0004・0005・0026 の該当箇所、CLAUDE.md、§7） | bundle ID が変わる | §5.1、§5.2、§4.2 |

1 つにしない理由: 一元化は「挙動が変わらない」ことを Info.plist の `diff` で機械的に確かめられるが、値の切り替えと混ぜると確かめられない。切り替えの PR が 2 行と期待値と文書だけになり、レビューと、実機で問題が出たときの戻しが小さくなる。割る費用は PR が 1 つ増えることだけ。

#126（新しいリポジトリへの移行）との順番: **今のリポジトリで PR-1・PR-2 を終えてから、新しいリポジトリを作る**のを推奨する（§11-3）。新しいリポジトリの最初の内容に `<旧 ID の頭>` が入らない。

---

## 7. 証明書の識別子を文書から外す
- 外すもの: 証明書の ID（CN の括弧内の値。ADR 0004 §15.1-1 にある）、証明書の名前（`Apple Development: <名前> (…)`。個人の名前やメールアドレスを含みうる。#115）。どちらも署名の検証に使っていない（検証はチームID = `subject.OU` だけ）。`build.sh` と `setup.sh` は証明書をキーチェーンから OU で選ぶので、名前を文書に書く必要もない。
- チームID の値は**コード（`lib.sh`、`Identity.swift`）と期待値のテストにだけ置き**、文書（`CLAUDE.md`、`docs/architecture.md`、`docs/phase2-contract.md`、`macos/README.md`、`build.sh` の使い方のコメント、ADR 0004 §15.1・0005 §6.3）では「`lib.sh` の `TEAM_ID`」と指す。チームID は署名されたアプリから誰でも読める値で秘密ではないが、文書に写すと写しが増え、他の人が自分の値に変えるときの直し漏れになる。
- 過去の ADR（0004、0005、0026）の値の書き換え: 新しいリポジトリは履歴なしなので、ADR の本文を直してよい。直すときは「2026-10-25 に ADR 0036 で `io.github.ikari-zen` に移した」と 1 行足し、決定の経緯（`<旧 ID の頭>` を選んだこと）は値を伏せずに残すか、§11-2 でユーザーに聞く。
- `docs/handoff-*.md` の古い値（`log show` の例など）: 過去の引き継ぎなので直さない。次の引き継ぎから新しい値を書く。

---

## 8. 検討した案

| 案 | 捨てた理由（1 行） |
| --- | --- |
| 頭を個人名にする（`dev.<名前>`、`io.github.<個人のアカウント>`） | 製品を個人に結び付けると、今回のようなアカウントの移行のたびに変える。個人情報を出さない方針（#115）にも反する |
| 頭を旧アカウント名のままにする（`<旧 ID の頭>`、`io.github.<旧アカウント名>`） | #187 の発端（旧アカウントへの結び付き）が残る。`<旧 ID の頭>` は持っていないドメインの逆順でもある |
| 独自ドメインを取る（`dev.ikari-zen` など） | ドメインを持たないとユーザーが決めた。取ると更新の費用と手間が続き、失効すると他人が取れる。`io.github.ikari-zen` は GitHub の組織名で一意に決まり、費用がない |
| `inputmethod` を省く（`io.github.ikari-zen.ikari`） | 入力メソッドの bundle ID に `.inputmethod.` を含める慣例（azooKey、Google 日本語入力、今の ikari）から外れる。外したときの挙動を確かめる理由がない |
| 案 C: 頭を環境変数で上書きし、Swift は実行時に Info.plist（`IkariIdentifierPrefix`）から読む | 他の人はファイルを編集せずに済むが、Info.plist にキーがない・誤るときに署名の条件が壊れる道が増え、テストは既定値しか見られない。利用者は作者 1 人で、上書きする人は fork して 4 行を直せば足りる。必要になったら §3 の形から移れる（§11-1） |
| 頭の定義を `IkariXPC/Identifiers.swift` に置く | `IkariShellCore`（英語のモードの ID）と `IkariSettings` が `IkariXPC` に依存しないので、写しが残るか依存が増える（§0.2） |
| 識別子の Swift ファイルを `lib.sh` から生成する | swift test は `build.sh` を通らずに流れるので、生成の手順がテストの前提に入り込む。生成物をリポジトリに入れると 2 か所と同じ |
| 置換の PR を 1 つにする | §6 |
| 古い ID のプロセスを bundle ID で探して止める処理を `install.sh` に足す | プロセス名が変わらないので、今の `killall` で止まる。1 回きりの移行のために恒久のコードを足さない |
| UserDefaults の引き継ぎの処理を入れる | UserDefaults を使っていない（§0.1） |

---

## 9. 結果（提案どおりに入れた場合）
- 得るもの: 製品の識別子がアカウント名から離れ、GitHub の組織に結び付く。頭とチームID を変えるのが 4 行で済み、ずれは `check.sh` とビルドの検査が落とす。文書から証明書の個人に結び付く値が消える。
- 失うもの: テンプレートが直接は読みにくくなる（印が入る）。`build.sh` に置き換えの段が 1 つ増える。
- 波及: 実機の入れ直しが 1 回要る（§4.2）。英語のモードの履歴を作り直す。TCC の許可をやり直すことがある。

## 10. 実装の進め方（承認後）
1. PR-1（code-editor、Sonnet。レーンは 1 つ。契約の変更は `IkariIdentifiers` の中身だけで、公開の名前は変えない）: §3.1。リードが §5.1 で確かめてマージ。
2. PR-2（code-editor、Sonnet）: §6 の表。リードが §5.1 で確かめる。マージの前にユーザーに聞く（bundle ID が変わり、実機の入れ直しが要るため。CLAUDE.md「設計を変えるもの」）。
3. 移行（リードとユーザー）: §4.2。smoke は §5.2。終わったら #187 を閉じ、`docs/user-feedback.md` か引き継ぎに、移行で起きたこと（TCC、HIToolbox の残り）を事実として書く。

---

## 11. ユーザーの決定（2026-10-25。3 点とも推奨どおり）
1. **他の人の上書きの形**: 頭とチームID を `lib.sh` と `Identity.swift` の 2 か所に定数で置き、上書きする人はその 4 行（とテストの期待値）を直す形でよいか。環境変数だけで上書きできる形（案 C）は、Swift が実行時に Info.plist から読む道が増える。**推奨: 定数の 2 か所（§3）**。必要になったら案 C に移れる。
2. **過去の ADR の値**: ADR 0004・0005・0026 の `<旧 ID の頭>…` とチームID と証明書の ID を、新しいリポジトリに移る前に書き換えるか。**推奨: 識別子は新しい値に直し、`<旧 ID の頭>` を選んだ経緯は 1 行で残す。チームID は「`lib.sh` の `TEAM_ID`」に、証明書の ID と名前は消す**（§7）。
3. **順番**: 今のリポジトリで PR-1・PR-2 と実機の移行を終えてから、ikari-zen の新しいリポジトリを作る（#126）でよいか。**推奨: この順**（新しいリポジトリの最初の内容に旧 ID が入らず、問題が出たら今のリポジトリで戻せる）。

追記（M5、#138）: `IkariShellCore` を消したので、`Identity.swift` は `IkariXPC` に移した（`IkariSettings` も今は `IkariXPC` に依存する）。生成は同じ `gen-identity.sh`、検査は `cargo xtask check`。
