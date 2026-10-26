# ADR 0005: macOS の ON/OFF 表示、入力メニュー、設定アプリ、ユーザー辞書と学習の管理

> 注（2026-10-25）: bundle ID の頭は [ADR 0036](0036-bundle-id-migration.md) で `<旧 ID の頭>` から `io.github.ikari-zen` に移した。この文書の識別子は新しい値に直してある。チームIDと証明書のIDは文書に書かず、`macos/scripts/lib.sh` の `TEAM_ID` を参照する。

- 状態: 承認（2026-10-12。§15.1 を参照）
- 日付: 2026-10-12
- 対象: macOS 版。ユーザーの要望（2026-10-12）の 1〜4。spec ではフェーズ3だった「ユーザー辞書の画面」を前倒しする
- 前提: main = 8b57e3f。ADR 0004（§15.1 の決定を含む。チームID（`lib.sh` の `TEAM_ID`）、Bundle ID `io.github.ikari-zen.inputmethod.ikari`、表示名 ikari）。docs/phase2-contract.md。spec の基本原則、UI、表記とアイコン、属性プロファイル、ユーザー辞書・学習・エクスポート/インポート、文章の取り込み、NFR-17〜20

---

## 0. 現状の把握

### 0.1 SDK と手元の環境で確かめたこと（2026-10-12）

| 項目 | 事実 |
| --- | --- |
| SDK | MacOSX 27.0。InputMethodKit のヘッダは ADR 0004 の時点から変化なし |
| `IMKInputController.menu()` | 「メニューを描くたびに呼ばれるので、入力メソッドは現在の状態を反映できる」（ヘッダの説明）。戻り値は `NSMenu` |
| メニューの項目の実行 | `doCommandBySelector:commandDictionary:` が、**コントローラ自身**がそのセレクタに応答すれば `performSelector:withObject:` で呼ぶ。引数は infoDictionary で、`kIMKCommandMenuItemName`（選ばれた `NSMenuItem`）と `kIMKCommandClientName`（クライアント）が入る。**項目の `target` は使われない** |
| `showPreferences:` | 既定の実装は `preferences.nib` を探して開く。メニューに action が `showPreferences:` の項目を置くと、入力メニューから呼ばれる。上書きできる |
| モードの切り替え | `IMKTextInput` の `selectInputMode:`（HIToolbox の `IMKInputSession.h`）。モードの ID は `ComponentInputModeDict` のキー。現在の殻は `setValue(_:forTag: kTextServiceInputModePropertyTag)` でモードの変化を受けている |
| 入力ソースのアイコン | `kTISPropertyIconImageURL` のヘッダ: 「入力モードのアイコンは普通 TIFF。PDF や PNG も使える。TIS は `NSBundle imageForResource:` で読む」 |
| 他の IME の Info.plist | azooKey: 各モードに `TISIconIsTemplate = true`、Menu / Palette / Alternate に同じ TIFF。モードの表示名は `en.lproj/InfoPlist.strings` に、**入力モードの ID をキーにして**書く。Google 日本語入力: テンプレートなしの TIFF。**設定（`ConfigDialog.app`）、辞書ツール、単語登録を、IME とは別のアプリとしてバンドルに同梱している** |
| Apple の日本語 IM | Info.plist に入力モードの定義がない（別の仕組み）。参考にできない |
| 今の ikari.app | `Resources` に `*.lproj` がない（モードの表示名を付けていない）。アイコンは「リングの中の あ / A」の線画で、2つとも同じ系統 |

### 0.2 コードの現状（設計に効く事実）
- **殻の設定は起動時に1回だけ読む**（`AppContext.init`）。`InputStateMachine` は `settings` と `keymap` を `let` で持つ。設定を変えたら `killall ikari` が必要。
- **エンジンは `settings.tsv` を `Engine::open` のときだけ読む**（`engine.lm`、`engine.idle_unload_secs`）。mtime の監視（`DataWatch`）は `user_dict.tsv` と `profile.tsv` だけで、1秒に1回まで。変わると Stack を組み立て直す（学習ストアと LM は `Reuse` で使い回す）。
- **学習はエンジンのメモリが正**。5秒ごとと `EndSession` で保存する。ファイルを外から書き換えても、次の保存でメモリの内容に上書きされる。CLI の `learn reset` と `data import` が `engine.lock` を見てエラーにするのはこのため。→ **インポートとリセットは、エンジンの中でメモリと一緒に変えるしかない。**
- `Reuse::from_stack` は学習ストアを使い回すので、インポートのあとに今の組み立て直しを呼んでも、古いメモリが残る。
- エンジンの要求の上限は `MAX_REQUEST_BYTES = 256 KiB`。学習は最大 `MAX_LEARNING_ROWS = 200_000` 行で、エクスポートは数十 MB になりうる。
- 同梱の XPC サービスは、それを含むアプリ（`ikari.app` を main bundle とするプロセス。smoke もここ）からしか引けない。**別のバンドルのアプリからはエンジンにつながらない。**
- キャレットの矩形（`ClientAccess.caretRect()`）は**変換文字列（marked range）があるときだけ**取れる。モードを切り替えた瞬間は変換文字列がないので、そのままでは使えない。`resolveAnchor` と `placePanel` は `IkariShellCore` の純関数で、テストがある。
- `BeginSession` は確定履歴を空にする（既存の挙動。`activateServer` のたびに送っている）。
- core には、ユーザー辞書の追加・削除・一覧（`UserDictionary`）、層ごとのリセット（`LearningStore::reset(LayerSel)`）、エクスポートとインポート（`export_text` / `validate_export` / `import_text`、`EXPORT_SCHEMA = 1`、セクションごとにも schema）、プロファイル（プリセット4つ、`often` / `sometimes`）がすでにある。足りないのは「エンジンの中から、画面の操作として呼ぶ口」だけ。

### 0.3 範囲
- **入れる**: 要望の 1〜4。モードの表示名（`InfoPlist.strings`）。「このソフトについて」（NFR-43 のライセンス表示。設定アプリの1タブで、低コスト）。
- **入れない**: 文章の取り込みの画面と取り込み元の一覧（NFR-26。spec どおりフェーズ3。CLI のまま）、Mozc 形式のユーザー辞書のインポートの画面（CLI にある）、アプリごとのプロファイル、ローマ字表の編集画面、辞書の自動更新。
- **先に確かめること（スパイク S0、§11.1）**: §0.1 はヘッダと他の IME から読める事実で、macOS 27 の実機の挙動は未確認。各節に「確かめる前提」と「外れたときの代わり」を書く。

---

## 1. 決定の要約

| 項目 | 決定 |
| --- | --- |
| アイコン | **ON（日本語）は塗りの円に「あ」を白抜き、OFF（英数）は細い輪郭の円に「A」**。16pt、単色テンプレート、PDF。錨は入力メソッドのアイコン（`ikari.pdf`）と設定アプリのアイコンに残し、モードのアイコンには入れない |
| カーソル付近の表示 | 殻のプロセスに、アクティブにならずマウスを透過する `NSPanel` を1つ持つ。キャレットの位置は既存の `resolveAnchor` と `placePanel` で決める。**モードが実際に変わったとき**と、英数・かなキーを押したときに約1秒出す。アプリやフォーカスの移動では出さない。secure でも出す |
| 入力メニュー | `menu()` で毎回作り直し、チェックはそのときの状態から付ける。項目は平らに並べる（サブメニューを使わない）。キー割り当て、LM、属性プロファイル、入力方式、ユーザー辞書…、環境設定… |
| 設定画面の置き場所 | **IME のプロセスでは開かない。** 別のアプリ `IkariSettings.app`（表示名「ikari 設定」）を `ikari.app/Contents/Helpers/` に同梱し、LaunchServices で起動する |
| 設定アプリとエンジンの経路 | 設定アプリ → **殻（中継）** → エンジン。殻が Unix ドメインソケットで待ち、相手のコード署名をチームIDで確かめる。中身は protobuf（`bridge.proto`）。エンジンの管理の要求は、殻が種類だけ見て、中身を解釈せずに XPC へ流す |
| settings.tsv | **書くのは殻だけ**（メニューと設定アプリの両方の変更を殻が書く）。行を保ったまま値だけ差し替える。手で編集してもよい（殻が mtime で気づく） |
| 反映 | 殻の設定は、入力中でないとき（idle）に、その場で各セッションへ反映する。`engine.*` を変えたら、殻がエンジンへ `Shutdown{reason: SETTINGS_CHANGED}` を送り、起こし直して反映する |
| ユーザー辞書と学習 | engine.proto に管理のメッセージを足す（辞書の一覧・追加・削除、学習の件数・層ごとのリセット、エクスポート、インポート（確認だけ／実行）、プロファイルの取得・設定）。エンジンがファイルとメモリを一緒に変え、すぐに Stack を組み立て直す |
| ファイルの受け渡し | エンジン（Sandbox）は利用者が選んだパスを開かない。設定アプリが `NSSavePanel` / `NSOpenPanel` で選んだファイルを読み書きし、**中身をバイト列で渡す**。インポートとリセットの前に、エンジンが自動でバックアップを書く |
| スキーマ | `Envelope.schema_version` は 1 のまま（追加だけなので）。エクスポートのファイルは既存の `EXPORT_SCHEMA = 1` とセクションごとの schema で照合し、合わなければ何も変えずに理由を返す |

---

## 2. プロセスとバンドルの構成（追加分）

```
~/Library/Input Methods/ikari.app
  Contents/
    MacOS/ikari                          ← 殻。新しく: 入力メニュー、モードの表示、settings.tsv の書き手、設定アプリとの中継
    MacOS/ikari-smoke
    Resources/{ikari,ikari-japanese,ikari-roman}.pdf, keymap/, ja.lproj/InfoPlist.strings, en.lproj/InfoPlist.strings
    Helpers/IkariSettings.app            ← 新設。Bundle ID io.github.ikari-zen.inputmethod.ikari.settings、表示名「ikari 設定」
      Contents/MacOS/IkariSettings       ← SwiftUI。Sandbox なし、Hardened Runtime。エンジンには直接つながない
      Contents/Resources/THIRD_PARTY_NOTICES, AppIcon（錨）
    XPCServices/IkariEngine.xpc          ← 変更なし（管理のメッセージを処理する）
~/Library/Application Support/ikari-ime/
  settings.tsv                           ← 殻だけが書く（手の編集は可）
  run/bridge.sock                        ← 新設。殻が待つ。ディレクトリ 0700、ソケット 0600
  backups/before-{import,reset}-<UTC>.tsv ← 新設。エンジンが書く。最新3つ
```

```
 [IkariSettings.app] --Unix socket + bridge.proto--> [ikari（殻）] --XPC + engine.proto--> [IkariEngine.xpc]
     画面だけ。ファイル選択と                         設定の書き手。管理の要求を              ユーザーデータの唯一の書き手
     ファイルの読み書きはここ                          種類だけ見て中継する                    （学習、辞書、プロファイル）
```

- **依存の向きは一方向**: 設定アプリ → 殻 → エンジン。エンジンは設定アプリを知らず、殻は管理の要求の中身を知らない（種類の許可リストだけ持つ）。
- **ユーザーデータの書き手は1つずつ**: 学習・ユーザー辞書・プロファイル・バックアップはエンジン、`settings.tsv` は殻。CLI は ADR 0004 §7.4 のまま（辞書とプロファイルの編集は可、学習の書き込みはロックで拒否）。

---

## 3. アイコンとモードの表示名

### 3.1 案の比較

| 案 | 16pt での見分け | 錨との関係 | 判断 |
| --- | --- | --- | --- |
| **A. 塗りと輪郭の対比**（ON = 塗りの円に「あ」を白抜き、OFF = 輪郭の円に「A」） | 黒い面積が大きく違う（目安: ON は面の 55% 前後、OFF は 15% 前後）。視界の端でも区別できる | 円は錨の上の輪（クラウンリング）。spec の「リングの中の あ / A」をそのまま守る | **採用** |
| B. 形を変える（ON = 角丸の四角、OFF = 円） | 16pt では角の丸みが潰れ、形の差は小さい | 錨の輪と結びつかない | 却下 |
| C. ON だけ錨を足す（錨の上に小さな「あ」） | 16pt では錨も文字も潰れる（spec の「錨の輪郭だけで判別できる太さ」と両立しない） | 直接 | 却下 |
| D. 色で分ける | テンプレート画像は単色（macOS が明暗に合わせて色を付ける）。色は使えない | — | 却下 |

### 3.2 描き方（`macos/tools/make-icons.swift`）
- **ON（`ikari-japanese.pdf`）**: 直径 15pt の円を塗り、「あ」（ヒラギノ角ゴ W6、約 10.5pt）の字形の path を**くり抜く**。くり抜きは `CGPath.subtracting(_:)`（macOS 14 以降）で path の段階で行い、PDF に描くのは1つの path だけにする。ブレンドモード（`.clear` など）は PDF で再現されないので使わない。
- **OFF（`ikari-roman.pdf`）**: 線の太さ 1.25pt の円の輪郭に、「A」（W3、約 9.5pt）を線ではなく字形の塗りで置く。中は透明。
- **入力メソッドのアイコン（`ikari.pdf`）**: 今の錨のまま。システム設定の入力ソースの一覧に出る。設定アプリのアイコンも錨（大きいサイズなので輪郭を細かく描ける）。
- 黒（アルファだけが意味を持つ）で描く。16×16pt のキャンバス。
- **客観的な確認**: 生成スクリプトの最後で、2つの PDF を 16px と 32px にラスタライズし、不透明な画素の割合を出す。**ON が 0.45 以上、OFF が 0.25 以下**でなければ失敗にする（「はっきり違う」を数で縛る）。

### 3.3 Info.plist
- 各モードの辞書で、`tsInputModeMenuIconFileKey`、`tsInputModePaletteIconFileKey`、`tsInputModeAlternateMenuIconFileKey` を**同じファイル**にし、`TISIconIsTemplate = true` を保つ。
  - Alternate は、テンプレートがなかった頃に「反転表示用の別の画像」として使われたキー。テンプレートなら macOS が強調表示の色を付けるので、別の画像は要らない（azooKey と同じ）。
- `tsInputMethodIconFileKey = ikari.pdf` は今のまま。`tsInputMethodAlternateIconFileKey = ikari.pdf` を足す（azooKey と揃える。害はない）。
- **モードの表示名**: `Resources/ja.lproj/InfoPlist.strings` と `en.lproj` に、入力モードの ID をキーにして名前を書く。案は「ikari（あ）」「ikari（A）」（§15-2）。今は表示名がなく、入力メニューの一覧での見え方が定まっていない。

### 3.4 macOS が自動で出す表示との関係
- **メニューバー**: 現在の入力モードの Menu アイコンを、テンプレートとして描く。新しいアイコンはここで効く。
- **入力メニューの一覧**: 有効な入力モードを、アイコンと表示名で並べる。モードの切り替え（あ / A）は**この一覧が担う**ので、殻のメニュー（§5）には入れない。
- **キャレット付近のシステムの表示**: macOS 14 以降、入力ソースを切り替えるとキャレットの近くに小さな表示が出る（既知の挙動）。ikari のモードでも出るか、どのアイコンを使うかは、S0-4 で確かめる。出る場合は §4 の表示と二重になる（§4.6）。
- **アイコンのキャッシュ**: TIS はアイコンをキャッシュする。差し替えても変わらないときは、入力ソースを外して追加し直すか、ログインし直す（§10.2 の手順に入れる）。

---

## 4. カーソル付近の一時表示（モードの表示）

### 4.1 仕組み（`IkariIME/ModeIndicator.swift`）
- プロセスに1つの `NSPanel`。`styleMask = [.borderless, .nonactivatingPanel]`、`ignoresMouseEvents = true`、`hidesOnDeactivate = false`、`collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient, .ignoresCycle]`、`isOpaque = false`、影あり。**`makeKey` と `NSApp.activate` は呼ばない**。`orderFrontRegardless()` で出す。候補ウィンドウ（`CandidatePanel`）と同じ扱いで、フォーカスを奪わない。
- 中身は角丸の背景（`NSVisualEffectView`、HUD の材質）に「あ」または「A」を 20pt 前後で。大きさは 32×32pt 前後。
- ウィンドウレベルは `client.windowLevel() + 1`（候補ウィンドウと同じ）。
- **時間**（`IkariShellCore` の定数 `IndicatorTiming`）: 0.05 秒で現れ、0.7 秒保ち、0.2 秒で消える。表示中にもう一度出すときは、文字を差し替えて保つ時間をやり直す（点滅させない）。
- **打鍵で消す**: 表示中に `handle(_:)` がキーを受けたら、すぐ消す（そのあと候補ウィンドウが同じ場所に出るため）。

### 4.2 位置
- 新しく `ClientAccess.insertionRect()` を足す: `selectedRange()` の位置で、長さ0の `firstRect(forCharacterRange:)`。取れなければ、1つ前の文字（長さ1）の矩形の右端を使う。変換文字列は要らない。
- その結果を、**既存の純関数 `resolveAnchor(client:accessibility:lastKnown:mouseScreenVisibleFrame:)` にそのまま渡す**（アクセシビリティは権限があるときだけ。NFR-20）。取れた正常値は `AnchorMemory` にも覚える（候補ウィンドウと共有）。
- 置き方も**既存の `placePanel(anchor:panelSize:visibleFrame:)`** で、キャレットの下（下端に入らなければ上、右端なら左へ）。基準はキャレットを含むスクリーンの `visibleFrame`。
- キャレットが取れず、マウスのあるスクリーンの中央下に落ちたときも出す（どこかに出る方が、出ないより役に立つ）。

### 4.3 いつ出すか
判定は純関数 `IndicatorPolicy.decide(event:lastShownMode:settings:) -> IndicatorMode?`（`IkariShellCore`）にして、表でテストする。

| 出来事 | 出すか | 取り方 |
| --- | --- | --- |
| ikari の中で あ ⇄ A が変わった（入力メニュー、Ctrl+Space の切り替え、英数・かなキーによる切り替え） | **出す** | `setValue(_:forTag: kTextServiceInputModePropertyTag)` で、プロセス全体の「最後のモード」と違うとき |
| 英数・かなキーを押したが、モードは変わらなかった | **出す**（確認のため。英数キーを念のため押す使い方に応える） | `handle(_:)` で keyCode 0x66（英数）/ 0x68（かな）を受けたとき。S0-2 で、キーが殻に届くかを確かめる |
| 別の入力ソース（ABC など）から ikari に切り替えた | **出す** | `kTISNotifySelectedKeyboardInputSourceChanged`（分散通知）を殻が受けて印を立て、その直後の `activateServer` / `setValue` で出す |
| アプリやウィンドウ、入力欄を移っただけ（`activateServer` だけが来る） | 出さない | 上の印がなければ出さない |
| secure（パスワード欄、ターミナルの「セキュアキーボード入力」） | **出す** | 読むのはキャレットの矩形だけで、文字列は読まない。パスワードを日本語モードで打つ誤りを防げる。NFR-18（文脈と学習を使わない）には触れない |
| `indicator.enabled = false` | 出さない | 設定（§7.3） |

- **英数・かなキーが殻に届く場合**（S0-2 で確かめる）: `handle(_:)` の先頭（今の `romanMode` の判定より前）で受ける。合成中なら今の表示のまま確定し（変換しない）、`client.selectMode(<モードの ID>)` でモードを選び、表示を出して `true` を返す。届かない場合は、OS がモードを切り替えて `setValue` が来るので、1行目の規則だけで足りる（2行目の「変わらなかったとき」は出ない。受け入れる）。
- モードが変わったときに合成中の文字列を確定するのは、今の挙動のまま（`setValue` の中の `commitComposition`）。

### 4.4 確かめる前提と外れたときの代わり
- S0-3: 分散通知 `kTISNotifySelectedKeyboardInputSourceChanged` が、モードの切り替えと入力ソースの切り替えで殻のプロセスに届くこと。**届かなければ**、`deactivateServer` のときに `TISCopyCurrentKeyboardInputSource()` を見て「ikari 以外へ移った」を覚え、次の `activateServer` で出す。
- 表示のためにアクセシビリティの権限やキー入力の監視の権限を求めることはしない（NFR-20）。

### 4.5 しないこと
- 表示に音や VoiceOver の読み上げは足さない（求められていない）。
- 入力欄の中の文字を変えない。表示は入力に影響しない（基本原則）。

### 4.6 システムの表示と二重になる場合
- S0-4 で、macOS 27 がキャレット付近に ikari のモードの表示を出すと分かった場合でも、**ikari の表示を既定で出す**。理由: 英数・かなキーで変わらなかったときの確認を出せるのは ikari だけで、表示の大きさと時間を自分で決められる。
- 二重が気になる場合の選択肢を、設定アプリの「表示」タブに説明として出す: ikari の表示を切る（`indicator.enabled = false`）、またはシステムの表示を切る（`defaults write kCFPreferencesAnyApplication TSMLanguageIndicatorEnabled -bool false` が知られている。**公開の API ではなく、全体の設定を変えるので、ikari からは実行しない。ユーザーが自分で選ぶ**）。

---

## 5. 入力メニュー（`IMKInputController.menu()`）

### 5.1 作り方
- `override func menu() -> NSMenu!` で**毎回新しい `NSMenu` を作る**。中身は純関数 `InputMenuModel.build(state:) -> [MenuItemSpec]`（`IkariShellCore`）で決め、`IkariIME` はそれを `NSMenuItem` に写すだけにする。`state` は、殻の `Settings`、キャッシュしているプロファイルと LM の状態。
- **チェック**: 作るときに `item.state = .on / .off` を付ける。メニューを開くたびに呼ばれるので、別に更新する仕組みは要らない。
- **動作**: `item.action` に、コントローラの `@objc` メソッドのセレクタを入れる（`target` は IMK が使わない）。メソッドは infoDictionary を受け、`kIMKCommandMenuItemName` の `NSMenuItem` の `representedObject` に入れた `MenuCommand`（列挙型。`IkariShellCore`）で何をするか決める。セレクタは1つ（`ikariMenuCommand:`）にまとめ、分岐は `MenuCommand` で行う。
- **主スレッドを止めない**: `menu()` の中ではエンジンに問い合わせない（キャッシュした値だけを使う）。動作のうち、エンジンを呼ぶもの（プロファイル）は、別のキューで送ってからキャッシュを直す。
- `keyEquivalent` は付けない（入力メニューでは効かない）。

### 5.2 項目（平らに並べる）

```
✓ キー割り当て: macOS 風
  キー割り当て: MS-IME 風
  ─────
✓ 入力方式: ローマ字
  入力方式: かな
  ─────
✓ LM（zenz）で並べ替える            ← 状態が「読み込み中」「使えません」のときは、項目名の後ろに括弧で添える
  ─────
✓ 分野: IT・ソフトウェア
  分野: インフラ・クラウド（たまに）  ← 重みが sometimes のときは名前に添える
  分野: 小説
  分野: ビジネス
  ─────
  ユーザー辞書と学習…                ← 設定アプリを「辞書」のタブで開く
  環境設定…                          ← action = showPreferences:（上書きして設定アプリを開く）
```
- **分野の項目**: 選ぶと「含める（often）／外す」を切り替える。`sometimes` にするのは設定アプリから（メニューで3つの状態を回すと分かりにくいため）。
- **サブメニューを使わない理由**: 入力メニューでサブメニューが正しく動くかは、ヘッダにも他の IME の例にも根拠がない。Google 日本語入力も azooKey も平らに並べている。S0-1 で確かめ、動くと分かっても、項目が12前後なので平らのままにする。
- 入れないもの: あ / A の切り替え（OS の一覧がある）、`paging.*` と `engine.idle_unload_secs`（めったに変えない。設定アプリだけ）。

### 5.3 macOS 27 での出方（S0-1 で確かめる）
- 既知の挙動: IME のメニューの項目は、その IME のモードが今の入力ソースのときに、入力メニューの入力ソースの一覧の下に並ぶ。`menu()` は、今のクライアントのコントローラに対して呼ばれる。テキストの入力先がない（デスクトップを選んでいるなど）ときは、項目が出ないことがある。
- 設定は全体で1つ（コントローラごとではない）ので、どのコントローラが呼ばれても同じ中身になる。
- 確かめること: 項目が出ること、チェックが毎回反映されること、`doCommandBySelector` でコントローラのメソッドが呼ばれること、`showPreferences:` の項目が「環境設定…」として動くこと、区切り線が出ること。
- **外れたときの代わり**: 項目が出ない、または動かない場合は、メニューには「環境設定…」だけを置き、切り替えはすべて設定アプリで行う（要望 2 は満たせなくなるので、ユーザーに報告する）。

---

## 6. 設定アプリ

### 6.1 置き場所の比較

| 案 | 良い点 | 悪い点 |
| --- | --- | --- |
| **A. 別のアプリ `IkariSettings.app` を `Contents/Helpers/` に同梱し、LaunchServices で起動する**（採用） | 普通のアプリとして前面に出る（Dock のアイコン、Cmd+Tab、ウィンドウの管理）。**その入力欄に ikari で日本語を打てる**（クライアントは別のプロセス）。画面の不具合や停止が入力に波及しない。Google 日本語入力と同じ構成 | エンジンへの経路を作る必要がある（§6.3）。バンドルと署名の手順が1つ増える |
| B. IME のプロセス（`ikari`、`LSBackgroundOnly`、`.accessory`）の中で `NSWindow` / SwiftUI を開く | 経路が要らない（殻はすでにエンジンにつながっている） | 下の4点 |
| C. `ikari.app/Contents/MacOS/` に2つ目の実行ファイルとして置き、XPC でエンジンに直接つなぐ（smoke と同じ） | 中継が要らない | main bundle が ikari.app になり、IME と同じ Bundle ID の GUI のプロセスが2つになる（LaunchServices と TSM の扱いが不確か）。XPC サービスのインスタンスがプロセスごとに作られる場合、2つ目のエンジンが `engine.lock` を取れずに動かない |

**B を却下した理由（IME のプロセスでウィンドウを開くときの問題）**:
1. **アクティブ化**: 設定のウィンドウを使うには、IME のプロセスがアクティブなアプリになる必要がある。すると、それまでのクライアントのアプリは非アクティブになり、`deactivateServer` と `commitComposition` が走る。メニューから開いた瞬間に入力中の文字列が確定される。
2. **自分自身に日本語を打てない**: そのウィンドウの入力欄のクライアントは IME のプロセス自身になる。IMK のサーバとクライアントが同じプロセスの同じ主スレッドに乗り、入力メソッドが使われない（キーボード配列だけになる）か、同期の呼び出しで止まるおそれがある。**ユーザー辞書の登録は、読み（ひらがな）と表記（漢字）を打つことが前提**なので、これは致命的。
3. **フォーカスを奪う**: `.accessory` のアプリはメニューバーも Dock のアイコンも持たず、ウィンドウが他のウィンドウの後ろに隠れると戻しにくい。候補ウィンドウと同じ主スレッドで SwiftUI を動かすので、画面の処理が重いと打鍵が遅れる。
4. **障害の波及**: 画面の不具合で殻が落ちると、入力そのものが止まる（NFR-16 は「エンジンが落ちても入力を止めない」であり、殻を落としてよい理由にはならない）。

### 6.2 アプリの形
- Bundle ID `io.github.ikari-zen.inputmethod.ikari.settings`、表示名「ikari 設定」、`LSUIElement` なし（開いている間は Dock に出る）。最後のウィンドウを閉じたら終了する。
- **Sandbox に入れない**（Hardened Runtime だけ）。理由: 殻とのソケット（§6.3）は Application Support の下にあり、Sandbox の中から Unix ドメインソケットにつなぐには、ネットワークの entitlement（`network.client`）か、根拠の弱い一時例外が要る。ネットワークの権限を足すくらいなら、Sandbox なしで「通信するコードを持たない」方が NFR-17 の趣旨に合う。不正なファイルの解釈はエンジン（Sandbox の中）で行い、設定アプリはバイト列を運ぶだけなので、攻撃面は小さい。
- SwiftUI のタブ（1つのウィンドウ）:

  | タブ | 中身 |
  | --- | --- |
  | 入力 | 入力方式、キー割り当てのプリセット、操作ごとの上書き（キーを押して記録する欄と「なし」） |
  | 変換 | 最後のページで先頭に戻る、1ページの件数、LM を使う、LM を解放するまでの秒数、属性プロファイル（分野ごとに「使わない／たまに／よく」） |
  | 表示 | カーソル付近の表示を出す（と §4.6 の説明） |
  | ユーザー辞書 | 一覧（読み、表記、品詞、コメント。検索）、追加（品詞は選択式。動詞なら活用型も）、削除 |
  | 学習とデータ | 学習の件数（使用の層、初期学習の層、上限）、層ごとのリセット、エクスポート、インポート（確認してから上書き）、バックアップの場所 |
  | このソフトについて | 版、辞書のラベル、LM の状態、THIRD_PARTY_NOTICES（NFR-43） |

- 起動: 殻の「環境設定…」と「ユーザー辞書と学習…」が `NSWorkspace.openApplication(at:configuration:)`（`activates = true`、引数で開くタブ）で起動する。すでに動いていれば LaunchServices が前面に出す。Finder や Spotlight から直接開いてもよい（殻が待っていればつながる）。

### 6.3 殻との経路（ブリッジ）
- **輸送**: 殻が起動時に `~/Library/Application Support/ikari-ime/run/bridge.sock` で待つ（`run/` は 0700、ソケットは 0600。起動時に古いファイルを消してから作る）。同時に受ける接続は1つだけ。
- **相手の確認**: 受けた接続で `getsockopt(LOCAL_PEERTOKEN)` の audit token から `SecCodeCopyGuestWithAttributes` で相手のコードを得て、`SecCodeCheckValidity` で署名要件を確かめる。要件は既存の `SigningRequirements` に足す:
  - `team`: `identifier "io.github.ikari-zen.inputmethod.ikari.settings" and anchor apple generic and certificate leaf[subject.OU] = "<TEAM_ID>"`
  - `adhoc`: `identifier "io.github.ikari-zen.inputmethod.ikari.settings"`（形だけ。ADR 0004 §5.1 と同じ限界）
- **枠**: 4バイト（ビッグエンディアン）の長さ + protobuf。1つの枠の上限は 64 MiB（§8.4）。
- **メッセージ**（`proto/ikari/bridge/v1/bridge.proto`。Swift だけが使う。Rust は生成しない）:

```proto
syntax = "proto3";
package ikari.bridge.v1;
import "ikari/engine/v1/engine.proto";

message Frame {
  uint64 request_id = 1;                 // 設定アプリが付ける。殻からの通知は 0
  oneof body {
    BridgeHello hello = 10;               BridgeHelloReply hello_reply = 11;
    GetSettings get_settings = 12;        SettingsSnapshot settings = 13;
    UpdateSettings update_settings = 14;  // 返事は SettingsSnapshot（反映後）
    ikari.engine.v1.Envelope engine = 20; // 管理の要求と返事。殻はそのままエンジンへ流し、返事もそのまま返す
    Event event = 30;                     // 殻 → 設定アプリ
    BridgeError error = 31;
  }
}
message BridgeHello { uint32 bridge_version = 1; string app_version = 2; }   // bridge_version = 1
message BridgeHelloReply { uint32 bridge_version = 1; string shell_version = 2; }
message GetSettings {}
message SettingEntry { string key = 1; string value = 2; }       // docs/phase2-contract.md §3 のキーと値
message SettingsSnapshot { repeated SettingEntry effective = 1; } // すべての既知のキーの、今効いている値
message UpdateSettings { repeated SettingEntry set = 1; repeated string reset = 2; }   // reset = その行を消して既定に戻す
message Event { oneof kind { SettingsChanged settings_changed = 1; OpenTab open_tab = 2; } }
message SettingsChanged {}                                        // メニューや手の編集で変わった。設定アプリは取り直す
message OpenTab { string tab = 1; }
message BridgeError { BridgeErrorCode code = 1; string key = 2; }  // key は不正だった設定のキー名だけ
enum BridgeErrorCode { BRIDGE_ERROR_UNSPECIFIED = 0; VERSION_MISMATCH = 1; INVALID_SETTING = 2; NOT_ALLOWED = 3;
                       ENGINE_UNAVAILABLE = 4; TOO_LARGE = 5; WRITE_FAILED = 6; }
```

- **設定は文字列のキーと値で運ぶ**: 型は `IkariShellCore.Settings` の parse で付ける（両側が同じコードを使う）。新しい設定のキーを足しても bridge.proto を変えずに済む。
- **中継の許可リスト**: 殻は `engine` の Envelope の oneof の**種類だけ**を見て、§8.2 の管理のメッセージと `Hello` 以外は `NOT_ALLOWED` で断る（設定アプリから `Commit` を送って学習を汚す経路を作らない）。中身は解釈せず、ログにも書かない。
- **スレッド**: ブリッジは殻の主スレッドで動かさない（専用のキュー）。エンジンへの中継は `EngineClient.relay(_:timeout:)`（§8.3）を使い、IMK の主スレッドの `convert` と並行して動く（`XPCEngineClient` はスレッド安全）。
- **版の照合**: 最初の枠で `BridgeHello` を交わす。合わなければ設定アプリは「入力ソースを選び直すか、ログインし直してください」と出す（入れ替えた直後に古い殻が動いている場合）。
- **切れたとき**: 殻が止まると（`killall ikari`、入れ替え）、設定アプリは接続が切れたことを表示し、操作を無効にする。殻が戻れば、再接続の操作でつなぎ直す（自動の再試行は 5 秒おきに 1 分まで）。

### 6.4 確かめる前提と外れたときの代わり（S0-5）
- `Contents/Helpers/` の中のアプリを、背景のプロセス（殻）から LaunchServices で起動して、前面に出ること。設定アプリの入力欄で ikari の変換ができること（固まらないこと）。
- **前面に出ない場合**: 設定アプリの起動時に `NSApp.activate()` を呼び、それでもだめなら Dock のアイコンを弾ませる（`requestUserAttention`）。開くこと自体は保証され、クリック1回で前面にできるので、致命的ではない。

---

## 7. 設定の反映経路

### 7.1 書き手と書き方
- **`settings.tsv` を書くのは殻だけ**（`IkariIME/SettingsStore.swift`）。メニューの操作も、設定アプリの `UpdateSettings` も、殻がまとめて書く。書き手を1つにすると、同時の書き込みの競合と、反映の通知の漏れがなくなる。
- 書き方: 今のファイルを読む → 純関数 `SettingsFile.update(text:set:reset:) -> String`（`IkariShellCore`）で、**該当するキーの行の値だけを差し替える**（同じキーが複数あれば最後の行。注釈、空行、未知のキー、`engine.*` 以外の行の順はそのまま。ないキーは末尾に足す。`reset` はその行を消す）→ 結果を `Settings.parse` で検証する（警告が増えたら書かない）→ 一時ファイルに書いて rename。
- 手で編集してもよい（今の運用を壊さない）。注釈も残る。

### 7.2 殻への反映
- `SettingsStore` は `(settings, keymap, version)` を持ち、`version` は変わるたびに増やす。
- 読み直す契機: 自分で書いたとき、`activateServer` のとき（`settings.tsv` の mtime と大きさを見て、変わっていれば。手の編集に気づくため。stat 1回なので打鍵には影響しない）。
- 各コントローラは、`handle(_:)` と `activateServer` の入口で `version` を比べ、**状態が idle のときだけ** `InputStateMachine.reconfigure(settings:keymap:composer:)`（新設の契約）を呼ぶ。合成中や変換中なら、次に idle になるまで待つ。表示中の文字列は変えない（基本原則）。
  - `input.method` が変わったときだけ、新しい composer を作って渡す。
  - セッションの ID は変えず、`BeginSession` も送り直さない（確定履歴を消さない）。
- 変えたあと、つながっている設定アプリへ `Event.settings_changed` を送る。

### 7.3 エンジンへの反映
- エンジンが読むのは `engine.lm` と `engine.idle_unload_secs` だけで、読むのは `Engine::open` のときだけ（今のまま）。
- 殻が書いたあと、`engine.*` の値が前と違えば、殻がエンジンへ **`Shutdown{reason: SETTINGS_CHANGED}`** を送る（エンジンは学習を保存して終わる）。殻は「予期した終了」として扱い、落ちた回数に数えない（既存の `expectingExit`）。続けて別のキューで `Hello` を送り、エンジンを起こし直す。セッションは既存の再同期（`needsResync`）で送り直す。
- 起こし直しの間（辞書を開く間）に Space を押すと、殻のフォールバックの候補（ひらがな、カタカナ）が出ることがある。LM の読み込みは別スレッドで、その間は n-gram で変換する（NFR-15）。
- 手で `engine.*` を編集した場合も、殻が §7.2 で気づいたときに同じ手順を踏む。

### 7.4 settings.tsv のキー（追加）

| キー | 値 | 既定 | 読むもの | 意味 |
| --- | --- | --- | --- | --- |
| `indicator.enabled` | `true` / `false` | `true` | 殻 | モードを切り替えたとき、カーソル付近に「あ」「A」を出す（§4） |

既存のキー（phase2-contract §3）はすべて設定アプリで変えられるようにする。

---

## 8. ユーザー辞書と学習の管理

### 8.1 経路の比較

| 案 | 判断 |
| --- | --- |
| **エンジンに管理のメッセージを足し、エンジンが変える**（採用） | 学習はエンジンのメモリが正なので（§0.2）、リセットとインポートはエンジンの中でしか正しくできない。品詞の検証に要る grammar もエンジンが開いている。変えたあとすぐに Stack を組み立て直せる（mtime の1秒を待たない） |
| 設定アプリがファイルを直接書く | 学習はエンジンの次の保存で上書きされる。設定アプリに Rust の core と grammar を持ち込むことになる。Sandbox のエンジンと別に、ファイルの書き手が増える |
| CLI と同じく「エンジンを止めてから」行う | 画面の操作として不自然。止めている間は変換できない |

### 8.2 engine.proto の追加（契約）
`Envelope.body` に 40〜53 を足す。既存の番号は変えない。

```proto
  // ---- 管理（ADR 0005 §8）。殻（メニュー）と、殻が中継する設定アプリだけが送る ----
    UserDictList list_user_dict = 40;      UserDictListReply user_dict_list_reply = 41;
    UserDictAdd add_user_dict = 42;        // 返事は UserDictListReply（変更後の一覧）
    UserDictRemove remove_user_dict = 43;  // 同上
    LearningStatus learning_status = 44;   LearningStatusReply learning_status_reply = 45;
    LearningReset reset_learning = 46;     // 返事は LearningStatusReply（リセット後）
    ExportData export_data = 47;           ExportReply export_reply = 48;
    ImportData import_data = 49;           ImportReply import_reply = 50;
    ProfileGet get_profile = 51;           ProfileReply profile_reply = 52;
    ProfileSet set_profile = 53;           // 返事は ProfileReply（変更後）

message Shutdown { ShutdownReason reason = 1; }   // 既存の空のメッセージにフィールドを足す
enum ShutdownReason { SHUTDOWN_REASON_UNSPECIFIED = 0; VERSION_MISMATCH = 1; SETTINGS_CHANGED = 2; }

message UserDictEntry { string reading = 1; string surface = 2; string pos = 3; string comment = 4; }
message PosOption { string name = 1; bool needs_conj_type = 2; }       // 同梱の user_pos.tsv の簡略品詞
message UserDictList {}
message UserDictListReply { repeated UserDictEntry entries = 1; repeated PosOption pos_options = 2;
                            repeated string verb_conj_types = 3; DataIssue issue = 4; }   // issue は追加・削除の失敗
message UserDictAdd { UserDictEntry entry = 1; }                          // 読みはエンジンが正規化する
message UserDictRemove { string reading = 1; string surface = 2; }

enum LearningLayer { LEARNING_LAYER_UNSPECIFIED = 0; LEARNING_LAYER_USAGE = 1; LEARNING_LAYER_INITIAL = 2; LEARNING_LAYER_ALL = 3; }
message LearningStatus {}
message LearningStatusReply { uint32 usage_rows = 1; uint32 initial_rows = 2; uint32 max_rows = 3; string backup_file = 4; }
message LearningReset { LearningLayer layer = 1; }

message DataCounts { uint32 profile_domains = 1; uint32 user_dict_entries = 2; uint32 learning_rows = 3; uint32 ingest_sources = 4; }
message ExportData {}
message ExportReply { bytes content = 1; uint32 schema = 2; DataCounts counts = 3; }
message ImportData { bytes content = 1; bool dry_run = 2; }                // dry_run = 検証して件数を返すだけ
message ImportReply { bool applied = 1; DataCounts counts = 2; DataIssue issue = 3; string backup_file = 4; }
enum DataIssueKind { DATA_ISSUE_NONE = 0; NOT_IKARI_EXPORT = 1; SCHEMA_UNSUPPORTED = 2; PARSE = 3; INVALID_ENTRY = 4;
                     NOT_FOUND = 5; TOO_LARGE = 6; IO = 7; }
message DataIssue { DataIssueKind kind = 1; string section = 2; uint32 line = 3; uint32 found_schema = 4;
                    uint32 supported_schema = 5; string detail = 6; }

enum ProfileWeightKind { PROFILE_WEIGHT_UNSPECIFIED = 0; PROFILE_WEIGHT_OFF = 1; PROFILE_WEIGHT_SOMETIMES = 2; PROFILE_WEIGHT_OFTEN = 3; }
message ProfileDomainState { string id = 1; string name = 2; ProfileWeightKind weight = 3; }
message ProfileGet {}
message ProfileReply { repeated ProfileDomainState domains = 1; }   // 同梱のプリセットすべて（OFF を含む）
message ProfileSet { repeated ProfileDomainState domains = 1; }     // 全体を置き換える。OFF は書かない
```

- **文字列の扱い**: `ErrorReply.message` には今までどおりユーザーの文字列を入れない。一方、`DataIssue.detail` と `UserDictListReply` には、利用者自身のデータ（読み、表記、問題の行の説明）が入る。これは画面に出すためで、**エンジンのログにも殻のログにも書かない**（ログは種類、所要時間、DataIssueKind だけ。NFR-19）。
- エクスポートの中身には原文がない（学習は特徴量のハッシュだけ。既存の形式のまま。NFR-19）。

### 8.3 エンジンの処理（`engine/src/admin.rs`）
- 管理の要求は1つの `admin` の Mutex で直列にする。変換の経路（`convert`、`prefetch`）はこの Mutex を取らないので、管理の処理中も変換は止まらない。
- **辞書**: 毎回 `user_dict.tsv` をファイルから読み、変えて、一時ファイル + rename で書く（CLI が同時に編集していても上書きしない）。追加の前に、読みを `normalize_reading` で正規化し、品詞を grammar で検証する（`UserDictionary::check` / `build`）。失敗なら書かずに `INVALID_ENTRY`。書いたら**その場で** Stack を組み立て直し、先行計算のキャッシュを捨てる（既存の `refresh_data` の後半を関数に切り出して使う）。`DataWatch` の記録も新しい mtime に合わせる（二重に組み立て直さない）。
- **学習のリセット**: バックアップ（§8.5）→ `LearningStore::reset(layer)` → すぐ保存。`LEARNING_LAYER_INITIAL` の意味は CLI の `learn reset --layer initial` と同じ（同じ core の関数を呼ぶ）。
- **エクスポート**: まず学習を保存する（メモリの最新を含めるため。CLI の `data export` はロックを取らないので最大5秒古い）→ `export_text(dir)` → バイト列で返す。
- **インポート**: `validate_export` で全体を検証する（失敗なら何も変えない）。`dry_run` なら件数を返して終わる。実行するときは:
  1. バックアップ（§8.5）。
  2. 保存の Mutex を持ったまま `import_text(dir, text)`（全ファイルを一時ファイルに書いてから rename。既存）。
  3. **学習ストアを読み直す**: `Reuse { learning: None, lm: 今の LM }` で Stack を組み立て直す（新設の `Reuse::without_learning`）。今の `Reuse::from_stack` のままだと古いメモリが残り、次の保存でインポートを上書きしてしまう。
  4. 変換の表（`conversions`）と先行計算のキャッシュを捨てる。インポートの前に始めた変換を確定しても、`Commit` は `UNKNOWN_CONVERSION` になり、学習されない（文字列の確定は殻が行うので、入力は止まらない）。
- **プロファイル**: `ProfileSet` は `Profile::from_settings` で検証して `profile.tsv` に書き、その場で組み立て直す。
- **大きさの上限**: 要求の上限を2段にする。受け付ける上限は 64 MiB（`MAX_ADMIN_REQUEST_BYTES`）。decode のあと、`ImportData` 以外で 256 KiB（今の `MAX_REQUEST_BYTES`）を超えるものは `BAD_REQUEST`。エクスポートが 64 MiB を超えるときは `TOO_LARGE`（`MAX_LEARNING_ROWS` の範囲では超えない見込み。テストで最大行数を確かめる）。
- **殻の口**: `EngineClient` に `relay(_ request: Data, timeout: Duration) throws -> Data` を足す（Envelope のバイト列をそのまま送って返す。`request_id` は殻が付け直す）。待ちの上限は、インポートとエクスポートが 30 秒、ほかは 3 秒。メニューのプロファイルの操作は、`IkariXPC` の型付きの小さな関数（`EngineAdmin.profile()` / `setProfile`）からこれを使う。

### 8.4 Sandbox とファイルの受け渡し
- エンジンは Sandbox のまま、許可も増やさない（ADR 0004 §8.2 の一時例外の1つだけ）。利用者が選んだパスは開かない。
- **エクスポート**: 設定アプリが `ExportData` を送り、返ってきたバイト列を `NSSavePanel` で選んだ場所に書く。既定の名前は `ikari-export-YYYYMMDD.tsv`。
- **インポート**: 設定アプリが `NSOpenPanel` で選んだファイルを読み（64 MiB を超えたら送らない）、まず `dry_run = true` で送って「上書きする件数（辞書 N 語、学習 M 行、分野 K、取り込み元 L）」を確認のダイアログに出す。了承されたら `dry_run = false` で送る。
- 枠の上限（§6.3）とエンジンの上限を同じ 64 MiB にそろえる。

### 8.5 バックアップ
- インポートと学習のリセットの直前に、エンジンが `export_text` の結果を `backups/before-{import,reset}-<UTC>.tsv` に書く。最新の3つだけ残す。
  - 追記（2026-10-14、B22）: 空（分野・ユーザー辞書・学習・取り込み元の件数がすべて0）のとき、または一番新しいバックアップと内容が同じときは書かない。3つを超えて消すときも、中身のある最新の1つは残す（消去を続けると、中身のあるものが空のものに押し出されたため）。
- 形式はエクスポートと同じなので、元に戻すときは、そのファイルをインポートすればよい（設定アプリに「バックアップの場所を開く」を置く。Finder で開くだけ）。
- 原文を含まない（エクスポートと同じ。NFR-19）。容量は最大で数十 MB × 3。

### 8.6 スキーマの版
- **`Envelope.schema_version` は 1 のまま**。足すのはメッセージとフィールドだけで、既存のものの意味は変えない（`Shutdown` の reason は省略時 UNSPECIFIED で、今の挙動と同じ）。古いエンジンに新しい管理の要求を送ると、未知の oneof として `BAD_REQUEST`（"empty message"）が返る。設定アプリは「エンジンが古い。入力ソースを選び直してください」と出す。殻とエンジンの版の違いは、既存の `Hello` の照合で直る。
  - 規則（この ADR で決める）: **追加だけなら上げない。既存のフィールドの意味を変えるか、消すときに上げる。**
- **bridge.proto** は `BridgeHello.bridge_version = 1` で照合する（殻と設定アプリは同じバンドルから出るが、入れ替えの直後に古い殻が新しい設定アプリを起動しうる）。
- **エクスポートのファイル**: 既存の `#ikari-export<TAB>schema=1` と、セクションごとの schema（profile、user_dict、learning、ingest_sources）で照合する。合わなければ `SCHEMA_UNSUPPORTED` と、`section`、`found_schema`、`supported_schema` を返し、何も変えない。版を変換する関数（`transfer::migrate`）は、schema を初めて上げるときに作る（今は作らない。YAGNI）。

---

## 9. 基本原則との整合

| 原則 | この ADR での扱い |
| --- | --- |
| ユーザーの入力を正とする | モードの表示もメニューも、入力欄の文字を変えない。モードを切り替えたときは、表示中の文字列をそのまま確定する（変換しない。今の挙動）。設定の反映は idle のときだけで、合成中や変換中の文字列を変えない。インポートとリセットは、確定済みの文字に触れない |
| 学習は支援であり、決定ではない | 学習を層ごとに消せる（使用の層だけ、初期学習だけ、全部）。消す前にバックアップを取り、戻せる。学習の中身は特徴量だけなので、画面には件数だけを出す（原文を作り出さない） |
| 選択肢を不当に制限しない | ユーザー辞書は候補を**足す**だけで、ほかの候補を消さない。LM のオン・オフと属性プロファイルは順位だけを変え、候補の集合を変えない（プロファイルの加点の上限と文法の保護は既存の GrammarGuard のまま）。「この候補を出さない」（候補の除外）は入れない |
| 最終的な選択権はユーザー | インポートは確認のダイアログ（件数を出す）を経て上書きする。メニューの切り替えはすべて元に戻せる |
| NFR-17 | 設定アプリは通信のコードを持たない。エンジンは Sandbox でネットワークなしのまま |
| NFR-18 | secure ではモードの表示だけを出し（文字列を読まない）、文脈と学習は今までどおり使わない |
| NFR-19 | ログに読み、表記、文脈、辞書の語を書かない。エクスポートとバックアップに原文はない |
| NFR-20 | 新しい OS の権限を求めない。モードの表示は IMK のクライアントからキャレットを取り、アクセシビリティは権限があるときだけ（今のまま）。キー入力の監視の権限も要らない |

---

## 10. テスト方針

### 10.1 自動テスト

| 層 | 方法 | 主な内容 |
| --- | --- | --- |
| エンジンの管理 | `cargo test -p ikari-engine`（fixture の辞書） | 辞書の追加・削除・一覧（読みの正規化、品詞の検証の失敗で書かない、追加の直後の `Convert` に新しい語が出る＝1秒待たない、CLI が並行して書いた行を消さない）。リセット（層ごとに件数が合う、保存される、バックアップができる）。エクスポート → インポートの往復でファイルが一致。不正なファイルで何も変わらない。`dry_run` は書かない。**インポートの後の確定が、インポートした学習の上に保存される（古いメモリで上書きしない）**。インポートの前の `conversion_id` の `Commit` は `UNKNOWN_CONVERSION`。上限（256 KiB と 64 MiB の2段、最大行数のエクスポート）。新しいメッセージのバイト反転と切り詰めでパニックしない。ログに語が出ない（ログの行を検査する） |
| 殻のロジック | `swift test`（`IkariShellCoreTests`） | `SettingsFile.update`（注釈と未知の行を保つ、最後の行を差し替える、ないキーを足す、reset、検証で警告が増えたら拒む）。`Settings` の `indicator.enabled`。`InputMenuModel`（状態ごとの項目とチェック、LM の状態の添え書き、sometimes の添え書き）。`IndicatorPolicy`（§4.3 の表を全部）。`reconfigure` は idle のときだけ効き、表示中の文字列を変えない。`engine.*` の差分で Shutdown を送るかの判定 |
| ブリッジ | `swift test`（`IkariBridgeTests`） | 枠の読み書き（分割された読み、上限、長さ0、壊れた長さ）。中継の許可リスト（`Convert`、`Commit` などは `NOT_ALLOWED`）。版の照合。署名要件の文字列（team と adhoc） |
| XPC | `IkariXPCTests`（モック） | `relay` のタイムアウトと再接続。`Shutdown{SETTINGS_CHANGED}` の後の終了を落ちた回数に数えない |
| アイコン | `make-icons.swift` の最後 | 不透明な画素の割合（ON 0.45 以上、OFF 0.25 以下）を 16px と 32px で |
| E2E | `ikari-smoke`（XPC 経由） | `UserDictAdd`（見張りの語「いかりすもーく → IKARI_SMOKE」）→ `Convert` で出る → `UserDictRemove`。`LearningStatus`。`ExportData` → `ImportData{dry_run}` の件数が一致。**本物のユーザーデータに触れるので、書き込むのは見張りの語の追加と削除だけにし、インポートは dry_run だけ** |
| 署名 | build.sh の最後 | `codesign --verify --deep --strict` が設定アプリを含めて通る。設定アプリが §6.3 の要件を満たす。設定アプリに network の entitlement がない |

### 10.2 手動のチェックリスト（実機。完了条件に含める）
1. **アイコン**: メニューバーで、ライトとダーク、強調表示（メニューを開いた状態）のそれぞれで、ON と OFF が一目で分かる。システム設定の入力ソースの一覧に錨が出る。入力メニューの一覧に表示名が出る。変わらないときは入力ソースを外して追加し直す（手順を記録する）。
2. **入力メニュー**: 項目が出る。プリセットを切り替えると、再起動なしで Ctrl+J / Ctrl+U の働きが変わる。入力方式（ローマ字／かな）が次の入力から変わる。LM を切ると `logs/engine.log` にエンジンの再起動が出て、「このソフトについて」の LM の状態が変わる。分野の切り替えで `profile.tsv` が変わる。チェックが毎回正しい。
3. **モードの表示**: 英数・かなキー、入力メニュー、Ctrl+Space（ABC から ikari へ）のそれぞれで、キャレットの近くに出る。テキストエディット、Safari、VS Code、ターミナル、フルスクリーンのアプリ、別の Space、外部ディスプレイ。アプリを移っただけでは出ない。**表示の直後にそのまま打ち続けられる（フォーカスが移らない、最初の打鍵が失われない）**。打つとすぐ消える。パスワード欄で出る。システムの表示と二重になるかを記録する。
4. **設定アプリ**: 「環境設定…」で前面に開く。もう一度選ぶと前面に戻る。Dock に出る。**その入力欄に ikari で日本語を打てる（固まらない）**。閉じると終わる。`killall ikari` で切断の表示になり、入力ソースを選び直すとつなぎ直せる。Finder から直接開いてもつながる。
5. **すべての設定**: `input.method`、`keymap.preset`、操作ごとの上書き（と「なし」）、`paging.wrap`、`paging.size`、`engine.lm`、`engine.idle_unload_secs`、`indicator.enabled` を画面から変え、それぞれ挙動が変わる。手で書いた注釈が残る。手で `settings.tsv` を編集すると、次の入力欄の切り替えで反映される。合成中に設定を変えても、表示中の文字列が変わらない。
6. **ユーザー辞書**: 名詞、人名、動詞（活用型つき）、形容詞を登録し、すぐに変換できる（活用形も）。削除するとすぐに出なくなる。不正な品詞で理由が出る。CLI の `user-dict add` と画面の操作を交互にしても消えない。
7. **学習**: 件数が出る。使用の層だけ消すと初期学習が残る。バックアップができ、それをインポートすると元に戻る。
8. **エクスポートとインポート**: 書き出したファイルに原文がない（取り込んだ文の一部で grep して出ない）。別の名前で書き出したものをインポートすると、確認のダイアログの件数が正しく、上書きされる。壊したファイル（先頭行を消す、schema=2 にする）で、理由が出て何も変わらない。
9. **速さ**: 設定アプリでインポートを実行している間も、テキストエディットでの変換が遅れない（ログの convert の所要時間）。
10. **メモリ**: 殻が 50 MiB 以下のまま（ADR 0004 §10.2-11）。設定アプリは別に測って記録する。

---

## 11. 並行実装のレーン分割

### 11.1 スパイク S0（リード。レーンを始める前。半日）
捨てるコードで、macOS 27 の実機で次を確かめ、結果を §0.1 に追記する。外れたものは各節の「代わり」を採る。

| # | 確かめること | 外れたとき |
| --- | --- | --- |
| S0-1 | `menu()` の項目が入力メニューに出る、チェックが反映される、`doCommandBySelector` でコントローラのメソッドが呼ばれる、`showPreferences:` が動く、区切り線 | §5.3 の代わり（「環境設定…」だけ） |
| S0-2 | 英数・かなキーの keyDown が殻の `handle(_:)` に届くか。届かないとき、同じモードでも `setValue` が来るか | §4.3 の「届かない場合」 |
| S0-3 | `kTISNotifySelectedKeyboardInputSourceChanged` が殻に届く | §4.4 の代わり |
| S0-4 | macOS 27 のキャレット付近のシステムの表示が ikari のモードに出るか、どのアイコンを使うか | §4.6（既定は変えず、記録する） |
| S0-5 | `Contents/Helpers/` のアプリを殻から LaunchServices で起動して前面に出る。そのアプリの入力欄に ikari で打てる。Unix ソケットの `LOCAL_PEERTOKEN` から署名を確かめられる | §6.4 の代わり。ソケットの確認ができなければ報告して判断を仰ぐ |
| S0-6 | `CGPath.subtracting` で作った PDF がテンプレートとしてメニューバーに正しく出る | 16px と 32px の TIFF（`@2x`）を生成して使う |

### 11.2 契約コミット（リード）
1. `proto/ikari/engine/v1/engine.proto` に §8.2 を足す。`Shutdown.reason`。エンジンの `dispatch` に、新しい種類を受けて `BAD_REQUEST`（"not implemented"）を返す仮の分岐。`engine.pb.swift` を再生成する。
2. `proto/ikari/bridge/v1/bridge.proto`（§6.3 の全文）と、生成した `bridge.pb.swift`（`IkariProto` に置く。Rust の `ikari-proto/build.rs` は対象にしない）。`gen-proto.sh` を2ファイルに対応させる。
3. `macos/Package.swift`: library `IkariBridge`（枠の読み書き、`BridgeTransport` プロトコル、許可リスト。**枠の実装とテストはここで完成させる**。小さいので）、executable `IkariSettings`、test `IkariBridgeTests`。
4. `IkariShellCore` の公開の型とシグネチャ: `Settings.indicatorEnabled`、`SettingsFile.update(text:set:reset:)`、`InputMenuModel` / `MenuItemSpec` / `MenuCommand`、`IndicatorPolicy` / `IndicatorEvent` / `IndicatorMode` / `IndicatorTiming`、`InputStateMachine.reconfigure(settings:keymap:composer:)`、`EngineClient.relay(_:timeout:)` と `shutdown(reason:)`（モックも直す）。
5. `IkariXPC/Identifiers.swift`: `IkariIdentifiers.settings = "io.github.ikari-zen.inputmethod.ikari.settings"` と、`SigningRequirements.settingsApp(mode:)`。
6. `docs/phase2-contract.md`: §1 に設定アプリの識別子、§3 に `indicator.enabled`、§5（新設）にブリッジのソケットの場所と枠、管理のメッセージの上限（256 KiB / 64 MiB）、タイムアウト（3 秒 / 30 秒）。
7. `macos/scripts/templates/IkariSettings.Info.plist` と `settings.entitlements`（Hardened Runtime。Sandbox なし）の雛形。

### 11.3 レーン

| レーン | 持つファイル | 最初に置くもの | 完了条件 |
| --- | --- | --- | --- |
| **E3: エンジンの管理（Rust）** | `engine/src/admin.rs`（新設）、`engine/src/lib.rs` の `dispatch` と上限と `refresh_data` の切り出し、`engine/src/tests.rs`（管理の分）、`stack/src/lib.rs` の `Reuse::without_learning`、`core/src/user/**` の小さな公開の追加（品詞の選択肢の一覧、エラーから `DataIssue` への対応）、`engine/src/log.rs` | 辞書の追加 → 直後の Convert に出る、のテスト | §10.1 の「エンジンの管理」の行がすべて通る。`cargo test --workspace` が通る。eval の結果が変わらない（変換の経路に触れないことの確認） |
| **S3: 殻（Swift、IME 側）** | `macos/Sources/IkariShellCore/**`、`macos/Sources/IkariIME/**`（`ModeIndicator`、`SettingsStore`、`BridgeServer`、`SettingsLauncher`、`menu()`、`insertionRect`）、`macos/Sources/IkariXPC/**`（`relay`、`shutdown(reason:)`、`EngineAdmin`）、`macos/Tests/{IkariShellCoreTests,IkariXPCTests}/**`、`macos/tools/make-icons.swift`、`macos/Resources/icons/**`、`macos/Resources/*.lproj/**`、`macos/scripts/templates/ikari.Info.plist` | `IndicatorPolicy` と `SettingsFile.update` の表のテスト | §10.1 の殻のロジック、XPC、アイコンの行が通る。§10.2 の 1〜3 と 5（メニューから変えられる分）を、実エンジンで通す |
| **U: 設定アプリと配布（Swift）** | `macos/Sources/IkariSettings/**`、`macos/Sources/IkariSmoke/**`（管理の E2E）、`macos/Resources/settings/**`（アプリのアイコン）、`macos/scripts/{build,install,lib}.sh`（Helpers への同梱、内側からの署名、検証）、`macos/scripts/templates/IkariSettings.Info.plist`、`settings.entitlements`、`macos/README.md` | 偽のブリッジ（`BridgeTransport` のメモリ上の実装で、決まった返事を返す）で全タブを動かす | build.sh と install.sh が team と adhoc の両方で通り、`codesign --verify --deep --strict` が通る。smoke の管理の E2E が通る（E3 のマージ後）。§10.2 の 4〜9 を通す（E3 と S3 のマージ後） |

- ファイルの境界: `IkariBridge` は契約コミットで完成させ、以後は S3 と U のどちらも変えない（変えるときはリードが契約として直す）。`ikari.Info.plist` は S3、`IkariSettings.Info.plist` と build.sh は U。`engine.proto` と `bridge.proto` は契約コミットのあとは変えない。
- アイコンの描画（錨の大きいアプリのアイコン）だけは U が `make-icons.swift` に足したくなるが、**ファイルの持ち主は S3**。U は自分のディレクトリにアプリのアイコンを生成する別のスクリプト（`macos/tools/make-app-icon.swift`）を置く。

### 11.4 マージの順
1. S0（リード）→ 契約コミット（リード）。
2. **E3 と S3 を並行に**（Rust と Swift で、ファイルが重ならない）。S3 のメニューの分野の項目は、E3 のマージまではモックの `ProfileReply` で作る。E3 → S3 の順にマージする（S3 の手動チェックでプロファイルを実エンジンで確かめるため）。
3. **U**（ブリッジの相手の S3 と、管理の E3 が揃ってから、偽のブリッジを本物に替えて E2E を通す。U の画面の実装自体は 2 と並行に進めてよい）。
4. リードが §10.2 をすべて流し、`docs/phase3a-checklist.md` に記録する。

---

## 12. 完了条件
- `cargo test --workspace`、`cargo test -p ikari-engine-ffi -p ikari-lm --features lm-metal`、`swift test --package-path macos` が通る。
- `build.sh --signing team` と `install.sh` が通り、`codesign --verify --deep --strict` が設定アプリを含めて通る。エンジンの entitlement は変わらない（app-sandbox あり、network なし、一時例外は1つ）。
- smoke の既存の E2E と、管理の E2E（§10.1）が通る。
- ADR 0003 の評価の下限が変わらない（変換の経路に手を入れていないことの確認）。NFR-02 の性能の条件（ADR 0004 §12）が変わらない。
- アイコンの画素の割合の検査が通る。
- §10.2 の 1〜10 がすべて OK（記録を残す）。

---

## 13. 却下した案

| 案 | 却下した理由 |
| --- | --- |
| IME のプロセスで設定のウィンドウを開く | §6.1（アクティブ化で入力中の文字列が確定される、自分自身に日本語を打てない、フォーカスと主スレッドの問題、障害の波及） |
| 設定アプリを `Contents/MacOS/` の2つ目の実行ファイルにし、XPC でエンジンに直接つなぐ | 同じ Bundle ID の GUI のプロセスが2つになる。XPC サービスのインスタンスがプロセスごとなら、2つ目のエンジンがロックを取れない（§6.1 の C） |
| 設定アプリを `Process` で子プロセスとして起動し、stdin / stdout のパイプで話す | 経路の認証は要らなくなるが、LaunchServices を通らない起動なので、macOS 14 以降の協調的なアクティブ化で前面に出る保証がない。メニューからしか開けない |
| 設定アプリを Sandbox に入れる | Application Support の下の Unix ソケットにつなぐのに `network.client` か根拠の弱い一時例外が要る。不正な入力の解釈はエンジン（Sandbox）で行うので、得るものが小さい |
| 設定アプリが settings.tsv を直接書き、殻は mtime を見る | 書き手が2つになり、メニューの変更との競合と、反映の遅れ（次の `activateServer` まで）が出る。どのみちブリッジは要る（辞書と学習のため） |
| エンジンが settings.tsv を監視し、LM を実行中に読み込み・解放する | LM の状態が増え、組み立て直しの経路（`StackPaths` の `lm_model` は open のときに決まる）を変える必要がある。殻からの Shutdown と再起動は、版の不一致と kill -9 で使って試験済みの経路で、LM を変えるのはまれ |
| メニューにサブメニューを使う | 入力メニューで動く根拠がない（§5.2） |
| モードのアイコンに錨を入れる | 16pt で潰れる（§3.1 の C） |
| システムのキャレット付近の表示だけに任せる | 英数・かなキーで変わらなかったときの確認を出せない。macOS の版で挙動が変わりうる（§4.6） |
| 表示のためにキー入力の監視（CGEventTap）で英数・かなキーを拾う | 入力監視の権限が要り、NFR-20 に反する |
| インポートをマージにする | spec の「上書き（マージしない）」に反する |
| 管理の操作ごとに schema_version を上げる | 追加だけなら互換で、殻とエンジンは同じバンドルで出る。上げると smoke を含めて同時に直す必要が増えるだけ（§8.6） |

---

## 14. ユーザーの操作が必要なこと
1. §15 の判断。
2. S0 の実機の確認に立ち会う（入力メニューの見え方、モードの表示、システムの表示との二重）。
3. 新しいアイコンに差し替えたあと、表示が変わらなければ、入力ソースを外して追加し直す（または、ログインし直す）。
4. （任意）システムのキャレット付近の表示を切る場合は、自分で `defaults write kCFPreferencesAnyApplication TSMLanguageIndicatorEnabled -bool false` を実行する（ikari は実行しない）。
5. §10.2 の手動のチェックリストを実機で流す（リードと一緒に）。

---

## 15. 判断が必要な点
1. **設定画面を別のアプリにすること**（§6.1）。名前は「ikari 設定」（`IkariSettings.app`、`io.github.ikari-zen.inputmethod.ikari.settings`）、`ikari.app/Contents/Helpers/` に同梱、Sandbox なし（Hardened Runtime と、チームIDでの署名の確認）。**推奨: この案**。
2. **アイコンとモードの表示名**（§3）。ON は塗りの円に「あ」の白抜き、OFF は輪郭の円に「A」。錨はモードのアイコンに入れず、入力メソッドと設定アプリのアイコンに残す。表示名は「ikari（あ）」「ikari（A）」でよいか。
3. **モードの表示**（§4）。既定で出す。英数・かなキーでモードが変わらなくても出す。secure（パスワード欄）でも出す。システムの表示と二重になっても既定は変えない（切るかはユーザーが選ぶ）。
4. **入力メニューの中身**（§5.2）。キー割り当て、入力方式、LM、分野、ユーザー辞書と学習…、環境設定…。分野はメニューでは「含める（よく使う）／外す」だけで、「たまに」は設定アプリから。
5. **LM のオン・オフの反映をエンジンの再起動で行うこと**（§7.3）。切り替えの直後の数百ミリ秒は殻のフォールバックの候補になりうる。LM の読み込み（数秒）の間は n-gram で変換する。
6. **インポートとリセットの前の自動バックアップ**（§8.5）。`backups/` に最新3つ。最大で数十 MB × 3 のディスクを使う。
7. **spec の扱い**: 「ユーザー辞書の画面」をフェーズ3から前倒しし、この作業を「フェーズ3a」と呼ぶ。文章の取り込みの画面と取り込み元の一覧（NFR-26）は、フェーズ3のまま CLI で行う。spec の該当の節（「文章の取り込み」のルールの最後の行、ロードマップ）を、決定後に直してよいか。
8. **管理のメッセージの schema_version の規則**（§8.6）。追加だけなら 1 のまま、意味を変えるか消すときに上げる。

### 15.1 決定（2026-10-12）
| # | 決定 | 決めた人 |
| --- | --- | --- |
| 1 | 設定画面は別アプリ「ikari 設定」（IkariSettings.app）にして、Helpers に同梱する | ユーザー |
| 2 | アイコンは、ON が塗りの円に「あ」の白抜き、OFF が輪郭の円に「A」。表示名は「ikari（あ）」「ikari（A）」 | ユーザー |
| 3 | モードの一時表示は §4 の条件どおり（設定でオフにできる） | ユーザー |
| 4 | 入力メニューの中身は §5.2 のとおり | リード（案どおり） |
| 5 | LM のオン・オフは、エンジンを再起動して反映する | ユーザー |
| 6 | インポートとリセットの前に自動でバックアップを取る（最新3つまで） | リード（案どおり） |
| 7 | この作業をフェーズ3aと呼ぶ。spec のロードマップと「文章の取り込み」のルールを直す | リード（案どおり） |
| 8 | schema_version は、追加だけなら 1 のまま。意味を変えるか消すときに上げる | リード（案どおり） |

### 15.2 S0 の結果と、それを受けた決定（2026-10-12）
実機（macOS 27.0.1、ユーザーの自作キーボード）で確かめた結果:

| # | 結果 |
| --- | --- |
| S0-1 | ○ 入力メニューに項目が出て、チェックも反映される（ユーザーは右クリックで開いた）。項目の実行はアクション（`s0MenuCommand`）が直接呼ばれる（`doCommand(by:)` は0回）。**`menu()` はほぼ打鍵ごとに呼ばれた（30分で2048回）** |
| S0-2 | ○ かなキー（0x68）の keyDown は、macOS が入力ソースを切り替えたあと、殻にも届く。その文字は見えない空白で、アプリに返すと空白が入っていた（565f53b で修正）。英数キー（0x66）は、ABC が有効だと macOS が ABC に切り替え、殻には届かない（0回） |
| S0-3 | ○ `kTISNotifySelectedKeyboardInputSourceChanged` が届く |
| S0-4 | ○ 入力ソースを切り替えると、macOS がキャレット付近に表示を出す |
| S0-5 | ○ Helpers の設定アプリが LaunchServices で前面に出る。入力欄で ikari が使える。`LOCAL_PEERTOKEN` で相手を確認できた（チームID が一致し、偽の要件は false） |
| S0-6 | ○ 塗りと輪郭のアイコンは、ライトでもダークでも区別できる |

決定:
| # | 決定 | 決めた人 |
| --- | --- | --- |
| 1 | **英数モード（.Roman）をなくす。** 英語は ABC に任せる。ikari のモードは「日本語」だけにする（将来「速記」を足す）。ON/OFF は入力ソースの切り替えそのものになる | ユーザー |
| 2 | 1 に伴い、**ikari 独自のモード表示（§4）は作らない。** 切り替えの表示は macOS の表示（S0-4）に任せる。§3 のアイコンは ON（塗り）だけを使う | リード |
| 3 | 入力メニューのキー割り当て（macOS 風／MS-IME 風）は**排他（ラジオ式）**にする。S0 では両方にチェックを付けられた（ユーザーの指摘）。3a の完了条件に「一方を選ぶと、もう一方のチェックが外れる」を足す | ユーザーの指摘、リード |
| 4 | `menu()` は打鍵ごとに呼ばれる前提で、軽く作る（状態が変わらなければ前回のメニューを使い回す。1回 0.1ms 未満を目安） | リード |
| 5 | 英数・かなキーは殻で受け止め、アプリに渡さない（565f53b）。英数モードをなくしたあとの英数キーは、受け止めるだけで何もしない（届いた場合） | リード |

### 15.3 契約コミットの実施と、ADR からの差分（2026-10-12）
§11.2 の 1〜7 を、§15.2 の決定を反映して実施した。§4（モードの表示）と、それに関わる §11.2-4 の `IndicatorPolicy`、`IndicatorEvent`、`IndicatorMode`、`IndicatorTiming`、`Settings.indicatorEnabled`、`indicator.enabled`（§7.4）は契約に入れていない。設定アプリの「表示」タブ（§6.2）は、中身が `indicator.enabled` だけだったので、なくす（タブは入力、変換、ユーザー辞書、学習とデータ、このソフトについて）。

差分と補足:
| 項目 | 内容 |
| --- | --- |
| 排他の選択 | `MenuItemSpec.Kind.radio(MenuRadioGroup)`。同じ組の項目は、ちょうど 1 つだけ `isChecked`。組は `keymapPreset` と `inputMethod` |
| `menu()` の軽さ | `InputMenuModel.build(state:)` は純関数で、`InputMenuState`（`Equatable`）が同じなら同じ結果。キャッシュは IkariIME（レーン S3）が `InputMenuState` を鍵にして持つ |
| `engine.lm` | 殻の `Settings` は `engine.*` を持たないので、`InputMenuState.lmEnabled` で渡す。`Settings` に足すかはレーン S3 が決める（§7.3 の差分の判定にも要る） |
| `EngineClient` | `shutdown()` を `shutdown(reason: EngineShutdownReason)` に置き換えた（§11.2-4 の `shutdown(reason:)`）。`relay(_:timeout:)` を足し、`XPCEngineClient.relay` は S3 が実装するまで `unavailable` を投げる |
| 待ちの上限 | `EngineTimeouts.admin`（3 秒）、`.adminTransfer`（30 秒）を足した（§8.3）。`RelayPolicy.timeout(for:)` が種類で選ぶ |
| `IkariBridge` | `IkariProto` に加えて `IkariShellCore` に依存する（待ちの上限を共有するため）。長さ 0 の枠は壊れた枠として扱う（正しい `Frame` は必ず本体を持つ）。`FileDescriptorTransport` は本物のソケット用、`BridgeTransport` を U の偽のブリッジが実装する |
| 英数・かなキー | 殻は 0x66 と 0x68 を受け止めて何もしない。`setValue` による合成中の確定は、モードが 1 つになったので消した |
| 英数のアイコン | Info.plist は日本語モードだけにした。`ikari-roman.pdf` と `make-icons.swift` の描き直しはレーン S3 の持ち分（ファイルは残してある） |
| 管理のメッセージ | `dispatch` は 9 種類の要求を `BAD_REQUEST`（"not implemented"）で返す仮の分岐。返事の種類を要求として受けたときは、今までどおり "unexpected message" |


### 15.4 実装の結果（2026-10-12、リード）
- E3 → S3 → U の順にマージした（097a5a6、57dedbe、4224541）。S3 のために IkariIME を IkariBridge / IkariProto に依存させた（5f5…、Package.swift）。
- `scripts/check.sh` は通過（Rust 834件、Swift 324件）。team 署名のリリース版をインストールし、smoke の管理 E2E（辞書の追加と削除が変換に効く、学習の状態、プロファイル、エクスポートとインポート、不正なファイルの拒否）が通った。
- smoke の latency にときどき外れ値（68〜261ms）が出た。原因は再現できず確定しなかったが、構造上の問題2つを直した（c48cfd1: 組み立て直しと学習の保存で変換の経路を待たせない。4d03c0b: ロックを取れずに開けなかったエンジンが、あとから開き直す）。修正後は smoke を3回連続で通過（p99 0.69〜1.15ms）。外れ値は、再現しない限り追わない（ユーザーの判断）。
- E3 の逸脱: `ExportReply` と `ProfileReply` には `DataIssue` の欄がないので、失敗は `ErrorReply`（INTERNAL / BAD_REQUEST）で返す。インポートでは、いまの grammar でユーザー辞書を展開できるかも検証する。
- 手動のチェックリスト（§10.2）はユーザーの確認待ち。
