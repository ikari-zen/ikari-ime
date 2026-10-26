# ikari (macOS)

ikari-ime の macOS 版。InputMethodKit の殻（`ikari.app`、Swift）と、同梱の XPC サービス（`IkariEngine.xpc`、Rust のエンジンを App Sandbox で動かす）と、設定アプリ（`Contents/Helpers/IkariSettings.app`、「ikari 設定」）でできている。設計は `docs/adr/0004-phase2-macos.md`（殻とエンジン）と `docs/adr/0005-macos-settings-and-indicator.md`（設定アプリ、ユーザー辞書と学習の管理）、識別子と署名要件は `docs/phase2-contract.md`。

## 必要なもの

- Xcode 27 / Swift 6.4（SwiftPM、codesign、xcodebuild）、Rust（`aarch64-apple-darwin`）、cmake（エンジンに LM を入れるとき）
- 初回の `swift build` は SwiftProtobuf を GitHub から取得する（ビルド時だけ。IME 自体は通信しない）
- `team` 署名にはキーチェーンの `Apple Development` 証明書（subject.OU = チームID。値は `lib.sh` の `TEAM_ID`。CN の括弧内の ID はチームIDではない）（期限 2027-05-23。切れたら Xcode で更新する）

## ビルドとインストール

手順は [README の「インストール」](../README.md#インストール) が唯一の場所です（辞書の用意から、`scripts/setup.sh` の 1 コマンドまで）。
ここには、このディレクトリのスクリプトが何をするかだけを書きます。引数は各スクリプトの `--help` が正です。

- `build.sh`、`install.sh`、`smoke.sh` が出力先を共有する。出力は `target/macos/<release|debug>-<team|adhoc>/ikari.app`。
- `build.sh` は、cargo（`ikari-shell-ffi`、`ikari-engine-ffi`）、XCFramework（`macos/Frameworks/`、gitignore 済み）、`swift build`、バンドルの組み立て、内側から順の署名、検証までを行う。検証は次の内容。
  - `codesign --verify --deep --strict`
  - エンジンの entitlement に `app-sandbox` があり、`com.apple.security.network.*` がなく、データ用の一時例外（`~/Library/Application Support/ikari-ime/`）だけがあること
  - 殻に app-sandbox が付いていないこと
  - `SigningRequirements` の文字列（殻→エンジン、エンジン→殻）が、実際の署名に当てはまること
  - `team` のときは `TeamIdentifier` が `IkariIdentifiers.teamID` と一致すること
  - 設定アプリ（`IkariSettings.app`）: `codesign --verify --deep --strict`、identifier が `io.github.ikari-zen.inputmethod.ikari.settings`、Hardened Runtime（`runtime`）、`app-sandbox` と `com.apple.security.network.*` がないこと（Sandbox に入れず、通信の権限も持たない。ADR 0005 §6.2）、`SigningRequirements.settingsApp` に当てはまること、ikari.app の署名の封印（`_CodeSignature/CodeResources`）に含まれること、`team` のとき `TeamIdentifier` が一致すること
- `--no-lm` は llama.cpp を入れない（cmake 不要で速い）。`--skip-rust` は既にある XCFramework を使う。XCFramework がないと、`build.sh` は止まる（FFI なしの動かない「スタブのエンジン」の ikari.app ができるのを防ぐため）。スタブでよいときだけ `--allow-stub` を付ける（インストールしないこと）。
- 設定アプリは `ikari.app/Contents/Helpers/IkariSettings.app` に置く。署名は内側から（smoke、XPC サービス、設定アプリ、最後に ikari.app）で、`--deep` で署名しない。設定アプリの Resources は、アプリのアイコン（`macos/Resources/settings/AppIcon.icns`）、`THIRD_PARTY_NOTICES.md`（リポジトリ直下のもの。「このソフトについて」に出す）、`keymap/`（殻と同じプリセット）。版（`CFBundleShortVersionString`）と `IkariSigningMode` は `build.sh` が埋める。
- `install.sh` は、動いている `ikari` / `IkariEngine` / `IkariSettings` を止め、`~/Library/Input Methods/ikari.app` に置き換え、また止める。**入力ソースの設定は変えない。** 今 ikari を使っているときだけ、別の入力ソースを経由して選び直す。`--register` は「候補に出す」だけ、`--enable` はシステム設定の入力ソースに追加する（どちらも明示したときだけ）。
- 署名の方式は Info.plist の `IkariSigningMode` に入り、殻、smoke、XPC サービスが実行時に読む。XPC サービスが相手に求める署名要件は `IkariPeerRequirement` に入る（`ikari-smoke --print-requirement` の出力。出どころは `SigningRequirements` だけ）。

## 設定アプリ「ikari 設定」

Bundle ID `io.github.ikari-zen.inputmethod.ikari.settings`。Sandbox なし、Hardened Runtime。**エンジンには直接つながない**: 殻が待つ Unix ソケット（`~/Library/Application Support/ikari-ime/run/bridge.sock`）に、`bridge.proto` でつなぎ、設定の取得と更新、管理の要求（ユーザー辞書、学習、エクスポート、インポート、属性プロファイル）を殻の中継で送る（`Sources/IkariBridge`、docs/phase2-contract.md §5）。通信のコードを持たない（ソケットの記述子を読み書きするだけ）。

- 画面（SwiftUI の 1 つのウィンドウ）: 入力、変換、ユーザー辞書、学習とデータ、このソフトについて。タブの名前は `SettingsTab` の rawValue（`input` / `conversion` / `dictionary` / `learning` / `about`）で、起動の引数 `--tab <名前>`（または `--tab=<名前>`）と、殻からの `OpenTab` の通知で開く。
- 切れたとき: 接続が切れた表示にして操作を無効にする。つなげないときは 5 秒おきに 1 分まで自動で試し、そのあとは「再接続」の操作。
- ファイルの受け渡し: エクスポートとインポートのファイルは、設定アプリが NSSavePanel / NSOpenPanel で選び、読み書きする。エンジンにはバイト列で渡す（通常の要求は 256 KiB、インポートだけ 64 MiB まで）。インポートは先に `dry_run` で件数を確かめ、確認のダイアログ（上書きされる件数）のあとで実行する。学習のリセットも確認のダイアログを出す。実行の前に、エンジンが `backups/` に自動でバックアップを書く。
- 画面だけを確かめる: `IkariSettings --fake-bridge`（`swift run IkariSettings --fake-bridge` など）は、本物の殻とエンジンの代わりに、メモリ上の偽の殻とエンジン（`Sources/IkariSettings/Fake/FakeShell.swift`）につなぐ。全タブが動く。単体テストも同じ偽の殻を使う（`FileDescriptorTransport.socketPair()` の片側）。
- アプリのアイコン（錨）は `swift macos/tools/make-app-icon.swift` で `macos/Resources/settings/AppIcon.icns` を作り直す（生成物はコミットする）。モードのアイコン（`macos/Resources/icons`、`make-icons.swift`）とは別。

## 入力ソースの追加（初回だけ、手動）

手順は [README の「入力ソースの追加」](../README.md#入力ソースの追加初回だけ手動) にあります（`install.sh` の完了メッセージからの案内先）。

## テスト

```sh
swift test --manifest-cache none --package-path macos     # IkariShellCore、IkariBridge、IkariSettings（偽の殻）、IkariXPC（モック）、署名要件
macos/scripts/smoke.sh --signing adhoc                     # XPC 経由の E2E（約 25 秒。先に build.sh が要る）
```

`ikari-smoke` の内容: Hello（署名要件を通ること）、文脈つきの変換（「ユーザー辞書に単語を登録するときは」+ ひんし → 1位が「品詞」）、確定、`kill -9` からの復帰（1 秒以内）、版の不一致（Shutdown が働き、プロセスが替わる）、打鍵のシミュレーション（120ms 間隔）での Convert の時間（p99 16ms 以下）。管理の E2E（ADR 0005 §10.1）: `UserDictAdd`（見張りの語「いかりすもーく → IKARI_SMOKE」）→ `Convert` に出る → `UserDictRemove` → 出なくなる、`LearningStatus`、`ProfileGet`、`ExportData` → `ImportData{dry_run}` の件数が一致、壊れた入力を断る。**本物のユーザーデータに触れる**ので、書き込むのは見張りの語の追加と削除だけで、インポートは `dry_run` だけ（学習のリセットとインポートの実行はしない。途中で失敗しても見張りの語は最後に消す）。XPC の `relay` を使うので、殻側の実装（`XPCEngineClient.relay`）とエンジンの管理（`admin.rs`）が揃っているときに通る。文節と再変換の E2E（`SegmentE2E.swift`。ADR 0006 §10.1）: **secure のセッション（ID 2）で行うので学習を汚さない**。`Convert`（はじめてしった。2 文節以上になり、読みの合計が元の読みに戻り、文節の最初の表記をつないだものが全体の第一候補と一致する）→ `SegmentPage`（文節 1。候補の 0 番が表示中の表記。古い版は `STALE_LAYOUT`、知らない変換は `UNKNOWN_CONVERSION`）→ `ResizeSegment`（りぽじとりめい を りぽじとり｜めい に。`layout_rev` が上がる）→ `CommitSegments`（変換が閉じる）、`Reconvert`（見張りの文字列「品詞」。最初の表示が選択と同じ）→ `Cancel`（変換が閉じる）。エンジンが文節に対応していないと FAIL になる。エンジンが `ENGINE_NOT_READY` を返すと FAIL になる。ikari を使っている間は IME のエンジンが `engine.lock` を持つので、smoke は最初にロックを試し、使用中なら「実行できない」と出して終了コード 2 で止まる（ABC などに切り替えてから流す）。

## 知っておくこと

- **Package.swift と manifest のキャッシュ**: `Package.swift` は `Frameworks/*.xcframework` の有無で依存を変える（`swift test` を Rust のビルドなしで流せるようにするため）。SwiftPM はこの結果をキャッシュするので、XCFramework を作り直したあとは `--manifest-cache none` を付ける（`build.sh` は付けている）。
- **XCFramework のヘッダ**: 2つの XCFramework が同じ `include/module.modulemap` を出力して衝突するため、ヘッダと modulemap は `Headers/<モジュール名>/` の下に置く（`build.sh` の `stage_headers`）。
- **launchd の再起動の間隔**: 起動して約 10 秒以内に終わったサービスは、launchd が再起動を遅らせる。smoke は kill の前にエンジンが十分長く動いていることを待つ。
- **版の照合**: `XPCEngineClient.hello(shellVersion:)` は、`HelloReply.engine_version` が `shellVersion` と等しいことを求める。違えば `Shutdown` を送って起動し直し、それでも違えば `unavailable`（殻は単独で動く）。バンドルの版（`CFBundleShortVersionString`）は `Cargo.toml` の `[workspace.package] version`。エンジンはこれと同じ文字列を `engine_version` に入れること。
- **XPC サービスの例外**: サービスは中身を解釈しないが、`Shutdown` だけは返事のあとにエンジンを閉じてプロセスを終える。
- **アイコン**: `macos/Resources/icons/*.pdf` は、バンドルの `Contents/Resources/` の直下にコピーされる。Info.plist が使うのは `ikari.pdf`、`ikari-japanese.pdf`、`ikari-english.pdf`（英語のモード。輪郭の円に「A」。ADR 0026 §5.1）。
- **署名の方式を切り替えたとき（adhoc と team）**: サンドボックスのコンテナ `~/Library/Containers/io.github.ikari-zen.inputmethod.ikari.engine` は署名に結び付く。署名が違うコンテナが残っていると、エンジンが起動時（`_libsecinit_appsandbox`）で止まり、Hello がタイムアウトすることがある。そのときはコンテナを消す（Finder で削除するか、ターミナルにフルディスクアクセスを許可して `rm -rf`）。
