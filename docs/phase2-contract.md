# フェーズ2・3a・4 契約（識別子、署名要件、settings.tsv、キー割り当て、ブリッジと管理の上限、文節と再変換）

ADR 0004 §11.1-5 の置き場所。決定の根拠は ADR 0004 §15.1。§5 はフェーズ3a（[ADR 0005](adr/0005-macos-settings-and-indicator.md) §11.2-6）で、§6 と §4 の文節と再変換の行はフェーズ4（[ADR 0006](adr/0006-segment-editing.md) §11.2、§16）で足した。値を変えるときは ADR を先に直す。

## 1. 識別子

| 項目 | 値 | 使う場所 |
| --- | --- | --- |
| Bundle ID（殻） | `io.github.ikari-zen.inputmethod.ikari` | `ikari.app` の `CFBundleIdentifier`、署名の identifier。`.inputmethod.` を含める |
| 表示名 | `ikari` | 入力ソースの名前、`CFBundleName` |
| XPC サービスの Bundle ID / サービス名 | `io.github.ikari-zen.inputmethod.ikari.engine` | `IkariEngine.xpc`、`NSXPCConnection(serviceName:)` |
| smoke の identifier | `io.github.ikari-zen.inputmethod.ikari.smoke` | `ikari-smoke` の署名 |
| IMK の接続名 | `io.github.ikari-zen.inputmethod.ikari_Connection` | `InputMethodConnectionName` |
| 設定アプリの Bundle ID | `io.github.ikari-zen.inputmethod.ikari.settings` | `ikari.app/Contents/Helpers/IkariSettings.app`。表示名「ikari 設定」。署名の identifier（ADR 0005 §6.2） |
| 入力モード | `io.github.ikari-zen.inputmethod.ikari.Japanese`（あ）の1つだけ | `ComponentInputModeDict`。英数モード（`.Roman`）はなくした（ADR 0005 §15.2-1）。将来「速記」を足す |
| チームID | （`lib.sh` の `TEAM_ID`） | Apple Development 証明書の `certificate leaf[subject.OU]` |

Swift の定数は `macos/Sources/IkariXPC/Identifiers.swift`（`IkariIdentifiers`）。ここと値を揃える。

## 2. 署名要件

`build.sh --signing team|adhoc` で選ぶ。既定は `team`（ADR 0004 §15.1-1。NFR-42 は「Apple Development 証明書で署名する（無料、年1回更新）」と読み替える）。
文字列は `SigningRequirements`（`IkariXPC/Identifiers.swift`）が作り、`NSXPCConnection.setCodeSigningRequirement` に渡す。

| SIGNING | 殻 → エンジン（相手 = エンジン） | エンジン → 殻（相手 = 殻、smoke） |
| --- | --- | --- |
| `team` | `identifier "io.github.ikari-zen.inputmethod.ikari.engine" and anchor apple generic and certificate leaf[subject.OU] = "<TEAM_ID>"` | `(identifier "io.github.ikari-zen.inputmethod.ikari" or identifier "io.github.ikari-zen.inputmethod.ikari.smoke") and anchor apple generic and certificate leaf[subject.OU] = "<TEAM_ID>"` |
| `adhoc` | `identifier "io.github.ikari-zen.inputmethod.ikari.engine"` | `identifier "io.github.ikari-zen.inputmethod.ikari" or identifier "io.github.ikari-zen.inputmethod.ikari.smoke"` |

殻がブリッジ（§5.1）で受けた接続の相手（設定アプリ）の要件は `SigningRequirements.settingsApp(mode:)`:

| SIGNING | 殻 → 設定アプリ（相手 = 設定アプリ） |
| --- | --- |
| `team` | `identifier "io.github.ikari-zen.inputmethod.ikari.settings" and anchor apple generic and certificate leaf[subject.OU] = "<TEAM_ID>"` |
| `adhoc` | `identifier "io.github.ikari-zen.inputmethod.ikari.settings"` |

- ad-hoc では identifier の一致しか見られない（形だけ）。同梱の XPC サービスは launchd の名前空間で守られる。
- `team` のビルドの署名 ID は Keychain の `Apple Development: …` 証明書（期限 2027-05-23。切れたら Xcode で更新する）。
- エンジンの entitlement: `com.apple.security.app-sandbox = true`、network なし、
  `com.apple.security.temporary-exception.files.home-relative-path.read-write = ["/Library/Application Support/ikari-ime/"]`。

## 3. settings.tsv

場所: `~/Library/Application Support/ikari-ime/settings.tsv`。殻とエンジンが読む。書くのは殻だけ（メニューと設定アプリの変更。ADR 0005 §7.1）。手で編集してもよい。
形式: 1行1設定の `キー<TAB>値`。`#` で始まる行と空行は無視する。ファイルがない、または読めないときは既定値を使う。
未知のキーと不正な値は無視して既定値を使う（入力を止めない。警告は文字列を含まない形でログに残す）。

| キー | 値 | 既定 | 読むもの | 意味 |
| --- | --- | --- | --- | --- |
| `input.method` | `romaji` / `kana` | `romaji` | 殻 | 入力方式。`kana` は物理キーを `IkariJisKey` の番号に対応づける（配列に依存しない） |
| `keymap.preset` | `macos` / `ms-ime` | `macos` | 殻 | キー割り当てのプリセット（§4）。既定は macOS 風（ADR 0004 §15.1-4） |
| `keymap.<操作>` | キーの表記（§4）/ `none` | （プリセットのまま） | 殻 | 個別の上書き。`none` はその操作の割り当てを外す |
| `paging.wrap` | `true` / `false` | `true` | 殻 | 最後のページで Space を押したとき、先頭のページに戻る。`false` なら止まる（ADR 0004 §15.1-3）。前のページへは常に Shift+Space |
| `paging.size` | `1`〜`9` | `3` | 殻 | 最初に見せる候補数。3 件目の次へ進むと 9 件に広がり、以後は 9 件ずつ（ADR 0014）。エンジンへの `Convert.page_size` はいつも 9（`Settings.enginePageSize`） |
| `engine.lm` | `on` / `off` | `on` | エンジン | LM（zenz）を使う（`EngineConfig.enable_lm`） |
| `engine.idle_unload_secs` | 秒（`0` = 解放しない） | `300` | エンジン | アイドル時に LM を解放するまでの秒数（`EngineConfig.idle_unload_secs`） |
| `dict.pack_it` | `on` / `off` | `on` | エンジン | 同梱の IT の語彙集（`core/assets/packs/it.tsv`。ADR 0012 §4）を使う。変更は次のエンジン起動から。設定画面にはまだ項目がない |
| `postal.japanese` | `on` / `off` | `on` | エンジン | 住所 ↔ 郵便番号の候補（`PostalRewriter`。ADR 0032 §2〜5）。`postal.tsv` がなければ何も出ない |
| `postal.english` | `on` / `off` | `off` | 殻 | 英語のモードで郵便番号の候補を出す（ADR 0032 §9。段階 3。エンジンの `Settings` も値は読む） |
| `postal.te_prefix` | `on` / `off` | `off` | エンジン・殻 | `て` + 郵便番号 → 〒つきの候補（ADR 0032 §8）。日本語・英語の両方に効く。設定画面にはまだ項目がない |

操作の名前は `KeymapAction`（`IkariShellCore/Keymap.swift`）の rawValue。

## 4. キー割り当て

### 4.1 表記
`修飾キー+キー`。修飾キーは `ctrl`、`shift`、`option`、`command`。キーは小文字で、1文字のキー（`j`、`;`）、`space`、`enter`、`escape`、`backspace`、`left`、`right`、`up`、`down`、`f1`〜`f20`。例: `ctrl+j`、`f6`、`shift+space`、`shift+left`、`ctrl+shift+r`。
`left` / `right` は ←/→（kVK_LeftArrow 123、kVK_RightArrow 124）。↑↓ は kVK_DownArrow 125、kVK_UpArrow 126（ADR 0014 が ADR 0006 §16-11 の「割り当てない」を置き換えた）。`left` / `right` は文節の軸、`up` / `down` は候補の軸。

### 4.2 プリセット（`macos/Resources/keymap/{macos,ms-ime}.tsv`）
形式は `キー<TAB>操作`。F6〜F10 はどちらのプリセットにも入れる（spec の字種変換）。

| 操作 | F キー | macos（既定） | ms-ime |
| --- | --- | --- | --- |
| `convert.hiragana` | F6 | `ctrl+j` | `ctrl+u` |
| `convert.fullwidth_katakana` | F7 | `ctrl+k` | `ctrl+i` |
| `convert.halfwidth_katakana` | F8 | `ctrl+;` | `ctrl+o` |
| `convert.fullwidth_alnum` | F9 | `ctrl+l` | `ctrl+p` |
| `convert.halfwidth_alnum` | F10 | （なし） | `ctrl+t` |
| `segment.focus_left` | （なし） | `left` | `left` |
| `segment.focus_right` | （なし） | `right` | `right` |
| `segment.shrink` | （なし） | `shift+left` | `shift+left` |
| `segment.expand` | （なし） | `shift+right` | `shift+right` |
| `reconvert` | （なし） | `ctrl+shift+r` | `ctrl+shift+r` |
| `candidate.next` | （なし） | `down` | `down` |
| `candidate.previous` | （なし） | `up` | `up` |

`candidate.next` / `candidate.previous`（↓/↑）は変換中だけ効き、Space / Shift+Space と同じ（ページの境目、広げる動き、`paging.wrap` も同じ）。文節のモードには入らない。idle は矢印をアプリに渡し、合成中は受け止めて何もしない。Shift+↑↓ は変換中は受け止めて何もしない。

文節の操作と再変換（下の 5 つ。ADR 0006 §8.1、§16.1-7）は **両プリセットとも同じ**で、F キーの割り当てはない。ほかの操作と同じく `keymap.<操作>` で上書きでき（`none` で外せる）、設定アプリの画面に自動で並ぶ。
`segment.*` は変換中に、修飾キーの有無を問わず矢印のキーを割り当てで引く。`reconvert` は idle のとき、Ctrl / Option / Command を伴う文字キーを引く（今の字種変換と同じ場所）。
**idle の矢印はアプリに渡す**（カーソル移動を奪わない）。合成中の矢印は今までどおり受け止めて何もしない。`segment.*` を矢印以外のキー（自作キーボードのレイヤーなど）に割り当てたときも、変換中だけ効く。

### 4.3 固定のキー（設定で変えない）
ADR 0004 §6.3 の表のとおり。ページ送りは、Space が次の候補（ページの最後の候補なら次のページの先頭、最後のページなら `paging.wrap` に従う）、Shift+Space が前の候補（ページの先頭なら前のページの最後）。

変換中の Enter、Esc、BackSpace、数字キー、文字キーの文節での意味（ADR 0006 §8.2、§9.6、§16.1-3。1文節のときは今までどおり）:

| キー | 1文節 | 2文節以上 |
| --- | --- | --- |
| Space / Shift+Space | フォーカスしている文節の次／前の候補 | 同じ |
| 1〜9 | その候補を選んで**全体を確定** | フォーカスしている文節にその候補を**選ぶだけで、確定しない**。選んだら**次の文節へフォーカスを進める**（最後の文節では動かさない） |
| Enter | 全体を確定 | 全体を確定（`CommitSegments`） |
| Esc / BackSpace | 読みに戻る | 読みに戻る（全体。区切りと選択は捨てる） |
| 文字 | 全体を確定して、その文字から合成を始める | 同じ |
| F6〜F10、字種変換のキー | 読み全体を字種変換 | フォーカスしている文節だけを字種変換し、`Literal` として置く（学習しない） |

再変換中（`ConversionOrigin.reconversion`）の Esc は、元の文字列を `KeyAction.restore` で書き戻して終える。BackSpace は推定した読みの合成に戻り、その中の Esc も元に戻す（ADR 0006 §9.6）。
殻が何も入力していない状態で Enter / Esc を受けたときは `KeyAction.resolveStrayMarkedText`（取り残された変換中の文字列を守る。ADR 0006 §16.3）。

## 5. ブリッジと管理のメッセージ（フェーズ3a。ADR 0005）

### 5.1 ブリッジ（設定アプリ ⇄ 殻）
- 輸送: 殻が `~/Library/Application Support/ikari-ime/run/bridge.sock`（`BridgeProtocol.socketRelativePath`）の Unix ドメインソケットで待つ。`run/` は 0700、ソケットは 0600。起動時に古いファイルを消してから作る。同時に受ける接続は1つだけ。
- 相手の確認: `getsockopt(LOCAL_PEERTOKEN)` の audit token からコードを得て、§2 の `SigningRequirements.settingsApp(mode:)` で確かめる。
- 枠: 4バイト（ビッグエンディアン）の長さ + `Frame`（`proto/ikari/bridge/v1/bridge.proto`）。1つの枠の上限は 64 MiB（`BridgeLimits.maxFrameBytes`）。長さ0と上限超えは壊れた枠として接続を閉じる。実装は `macos/Sources/IkariBridge`。
- 最初の枠は `BridgeHello`（`bridge_version = 1`、`BridgeProtocol.version`）。合わなければ `VERSION_MISMATCH`。
- 中継の許可リスト: 殻は `Frame.engine` の `Envelope` の種類だけを見て、`Hello` と §5.2 の管理の要求だけをエンジンへ流す。ほかは `NOT_ALLOWED`（`RelayPolicy`）。

### 5.2 管理のメッセージ（殻 ⇄ エンジン）
- `engine.proto` の `Envelope.body` の 40〜53。`Envelope.schema_version` は 1 のまま（追加だけなら上げない。意味を変えるか消すときに上げる）。
- 要求の上限: エンジンが受け付けるのは 64 MiB（`MAX_ADMIN_REQUEST_BYTES`）まで。decode のあと、`ImportData` 以外で 256 KiB（`MAX_REQUEST_BYTES`）を超えるものは `BAD_REQUEST`。エクスポートが 64 MiB を超えるときは `DataIssue.TOO_LARGE`。
- `Shutdown.reason`: `VERSION_MISMATCH`（版の不一致）、`SETTINGS_CHANGED`（`engine.*` の変更。ADR 0005 §7.3）。省略（`UNSPECIFIED`）は今までと同じ。どちらも保存して終了し、殻は落ちた回数に数えない。
- `ErrorReply.message` と、殻とエンジンのログには、読み、表記、文脈、辞書の語を入れない。`DataIssue.detail` と `UserDictListReply` は画面に出すためで、ログには書かない（NFR-19）。

### 5.3 待ちの上限（殻 → エンジン、`EngineTimeouts`）
| 要求 | 上限 |
| --- | --- |
| `ImportData`、`ExportData` | 30 秒（`adminTransfer`） |
| 上記以外の管理の要求、`Hello` の中継 | 3 秒（`admin`） |

## 6. 文節と再変換（フェーズ4。ADR 0006）

契約コミットで置いた型とシグネチャ。**中身は各レーンが実装する**（契約コミットの時点では `todo!()` / 仮の分岐）。変えるときはリードが契約として直す。

### 6.1 engine.proto（`schema_version` は 1 のまま）
- `Envelope.body`: `segment_page` 24 / `segment_page_reply` 25 / `resize_segment` 26 / `segment_layout` 27（`ResizeSegment` と `Reconvert` の返事）/ `commit_segments` 28（返事は `Ack`）/ `reconvert` 29（返事は `SegmentLayout`）。
- `ConvertReply.layout`（7）: 新しいエンジンは常に入れる（`focused = 0`）。ない（古いエンジン）なら殻は 1 文節として扱う。`candidates` は今までどおり読み全体の候補の 1 ページ目。
- `CandidateKind.LITERAL`（再変換で読めなかった部分）、`ErrorCode.STALE_LAYOUT`（`layout_rev` が古い）。
- 上限（超えたら `BAD_REQUEST`）: `SegmentChoice` の数は読みの文字数（256）まで、`literal` は 256 文字まで、`Reconvert.surface` は 64 文字まで。ログに読みと表記を書かない。
- 新しい殻と古いエンジン: 新しい要求には `BAD_REQUEST` が返る。殻は文節の操作を何もしない。

### 6.2 core（`ikari-core`）
- `converter::segment`: `SegmentSpan`、`SegmentLayout`、`SegmentChoice`、`SegmentError`（`StaleLayout` / `OutOfRange` / `Empty`）、`Segmented`（`layout` / `origin` / `whole` / `whole_mut` / `page_until` / `commit`）、`SegmentCommit`、`SegmentedOrigin`、`Converter::segment` / `resize`。
- `converter::reconvert`: `Converter::reconvert(surface, ctx, hint, deadline) -> Result<Segmented, SegmentError>`（読める部分がなければ `Empty`）。
- `CandidateOrigin::Literal`、`Dictionary::reading`（既定 None）、`AnalyzedContext::extended`、`pos::segment_role`、crate 内部の `NBest::with_end` と `lattice::build_constrained`。
- 統合で足したもの（lane C・E のあと）:
  - `Dictionary::warm_reading_index(&self)`（既定は何もしない）: `reading` の逆引き表を先に作る。エンジンが起動のあとに裏で呼び、最初の再変換が表の作成を待たないようにする。
  - `Dictionary::prefault(&self)`（既定は何もしない）: mmap した辞書のページを先に読む。エンジンが `warm_reading_index` と一緒に裏で呼ぶ。ないと、最初の再変換が参照のたびにページフォールトで 1〜8ms かかる（B6）。
  - `ContextAnalyzer::analyze_text(&self, text) -> Vec<ContextToken>`: 文脈とは別に、文字列だけを解析する（再変換が選択した表記を語に分ける）。左文脈のキャッシュを読み書きしない。既定は `analyze(text, &[])`。
  - `pos::{ClauseWord, clause_starts, joins_following_noun}`: 文節の区切りの規則（ADR 0006 §2.2）。`clause_starts(&[ClauseWord]) -> Vec<bool>` は語の並びのうち新しい文節の先頭の位置を返す。`joins_following_noun` は右に名詞が来たとき同じ文節の複合名詞として付く語（名詞、接頭辞、名詞的な接尾辞）か。
- 学習の形式（`learning.tsv`）は変えない。

### 6.3 IkariShellCore（Swift）
- `KeyInput.Key.left` / `.right`、`KeyCodes.leftArrow` / `.rightArrow`、`KeymapAction` の 5 つ（`isScriptConversion`）、`MarkedText.segments` / `focused` / `replacementRange`、`TextRange`、`SelectedText`。
- `InputStateMachine.handle(_:surrounding:selection:)`: `selection` は `reconvert` のキーのときだけ評価する。
- `ConvertingState.segments` / `focused` / `layoutRev` / `origin`（`ConvertingSegment`、`ConversionOrigin`）。
- `KeyAction.restore(String)`（元の文字列を `insertText(_, replacementRange: markedRange)` の 1 回で書き戻す）、`KeyAction.resolveStrayMarkedText`（§4.3）。
- `EngineClient`: `segmentPage` / `resize` / `commitSegments` / `reconvert`（待ちの上限は `EngineTimeouts.convertTimeout(文節の読みの文字数)`）。型は `SegmentLayoutInfo`、`SegmentPage`、`SegmentSummary`、`SegmentChoiceValue`、`ReconvertRequest`。`ConversionPage.layout`、`CandidateKind.literal`、`EngineErrorCode.staleLayout`。
- `RelayPolicy`: 文節と再変換のメッセージは設定アプリから中継しない（許可リストに入れない）。
