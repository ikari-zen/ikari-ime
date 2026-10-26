# 日本語の機械速記の調査と ikari-ime への組み込み案（2026-10）

状態: 調査メモ。決定ではない。決めたら ADR に起こす。
目的: メジロ式のような同時押し（コード入力）の速記を ikari の「速記モード」として使えるようにするため、方式を比較し、組み込み方を推奨する。

前提（ユーザーの要望）:

- 速記で打った音（かな）は、読みとして既存の変換（文脈と LM で並べ替える）に渡す。
- 略語（定型文のショートカット）は、その場で確定する。
- 通常の入力とは別に「速記モード」を設ける。
- 自作キーボード（ZMK/QMK 系の可能性が高い）を使う。

確認できなかったもの（先に書いておく）:

- **鉄器式**: 日本語・英語の両方で検索したが、一次情報が見つからなかった。日本ステノタイプ協会の Wiki のページ一覧（60 件）にもない。出典を教えてほしい（「判断してもらう点」の Q1）。
- **はやとち**: 見つからなかった。近い名前として、ソクタイプ方式の電子速記ソフト「はやとくん」が検索結果の要約に出てくるが、一次情報には届かなかった。
- **ステンチュラ系の日本語理論**: Stenograph 社の機械（Stentura）向けの日本語理論について、公開された資料は見つからなかった。

---

## 1. 方式の一覧

### 1.1 Plover 向けの日本語方式（オープンなもの）

| 方式 | 概要 | 一次情報 |
| --- | --- | --- |
| **メジロ式**（v2） | じーびす氏が作った Plover 向けの日本語速記。左右対称で、片手で 1 音節、両手で 2 音節を打つ。2026-10 時点で活発に更新されている（最終 push 2026-10-04） | [JEEBIS27/Plover_Mejiro](https://github.com/JEEBIS27/Plover_Mejiro)、[メジロ式v2-基本入力（note）](https://note.com/jeebis_keyboard/n/ne8430676220b)、[日本ステノタイプ協会 Wiki: メジロ式](https://scrapbox.io/Stenotype-Japan/%E3%83%A1%E3%82%B8%E3%83%AD%E5%BC%8F) |
| メジロ式の派生実装 | ZMK のモジュール（かなをローマ字のキー入力として OS の IME に送る）、AutoHotKey 版、QMK 版（専用機 Mejiro31） | [hamehame2/zmk-behavior-mejiro](https://github.com/hamehame2/zmk-behavior-mejiro)、[hamehame2/AHK_Mejiro](https://github.com/hamehame2/AHK_Mejiro)、[JEEBIS27/qmk_firmware の mejiro31](https://github.com/JEEBIS27/qmk_firmware/tree/master/keyboards/jeebis/mejiro31) |
| **ソクタイプ**（Plover 版） | 法廷・議会向けの日本語機械速記「ソクタイプ」を Plover に移したもの（WIP）。Tréal という機械が必要と README にある | [na4zagin3/plover_japanese_sokutaipu](https://github.com/na4zagin3/plover_japanese_sokutaipu)、[ソクタイプ資料集](https://sokkidouraku.lsv.jp/001c/index.html) |
| **ステノワード**（Plover 版） | 字幕向けの商用方式「ステノワード」の配列を Plover に移したもの。2018 年から更新なし | [na2hiro/plover-japanese-stenoword](https://github.com/na2hiro/plover-japanese-stenoword) |
| **まち針**（Machibari） | 「はちどり」機械速記配列を元に、出力を 1:1 にしようとしている配列。子音だけの入力から和語を引く案もある。開発途中 | [Meltedsoftcream/Plover_Japanese_Machibari](https://github.com/Meltedsoftcream/Plover_Japanese_Machibari)、[Wiki: まち針](https://scrapbox.io/Stenotype-Japan/%E3%81%BE%E3%81%A1%E9%87%9D) |
| **Chidori** | 英語の steno 配列のまま、和文を**ローマ字**で出し、OS の IME に変換させる Python 辞書。速さは狙っていないと作者が書いている | [paulfioravanti/plover-chidori](https://github.com/paulfioravanti/plover-chidori) |
| plover_japanese_kana | 英語の steno 配列でひらがなを打つ、簡易な方式。`*` で IME 変換を起こす案は未実装 | [Willem3141/plover_japanese_kana](https://github.com/Willem3141/plover_japanese_kana) |

日本語のステノの情報は、2025 年 6 月に発足した「日本ステノタイプ協会」の Wiki に集まりつつある（[Stenotype-Japan](https://scrapbox.io/Stenotype-Japan/)、発足時期は [toyboot4e の Zenn スクラップ](https://zenn.dev/toyboot4e/scraps/af274eca435d20) による）。

### 1.2 商用の日本語速記

| 方式 | 概要 | 一次情報 |
| --- | --- | --- |
| **ステノワード** | スピードワープロ社の字幕制作用キーボード。文字キーは 10 個（左 5 に子音、右 5 に母音）。1 打で 2〜20 字程度、熟練で毎分 300 字以上。研修は同社の学院 | [スピードワープロ: ステノワード](https://www.speed-wp.co.jp/speed-wp-stenoword/) |
| **ソクタイプ** | 機械速記の方式。親指キーで左右のかなの間に「ん・ち・く・き・つ・い」などを挟み、二字熟語を 1 打で出す。略語は裁判の記録向けで多い（[メジロ式の比較記事](https://note.com/jeebis_keyboard/n/nf5ec466bd37a) による） | [ソクタイプ資料集](https://sokkidouraku.lsv.jp/001c/index.html) |

商用方式の教本や辞書は公開されていない。ステノワードは、入力方法の特許（特開昭63-010220）が切れていると Wiki にあるが、教本の著作権は不明（[Wiki: ステノワード](https://scrapbox.io/Stenotype-Japan/%E3%82%B9%E3%83%86%E3%83%8E%E3%83%AF%E3%83%BC%E3%83%89)）。

### 1.3 同時押しのかな配列（速記ではないが比べる対象）

| 方式 | 概要 | 一次情報 |
| --- | --- | --- |
| **薙刀式**（v17） | 大岡俊彦氏の配列。26 キー＋シフト（スペース）で 50 音を打つ 2 層の構造。濁音・半濁音・拗音は同時押し。エミュレータ（桔梗、漢直WS など）か、QMK/ZMK のファームウェアで動かす | [GIZMODO の紹介記事（2026-05）](https://www.gizmodo.jp/2026/05/naginata-ime.html)、[作者のブログ](https://oookaworks.seesaa.net/article/456099128.html)、[eswai/zmk-naginata](https://github.com/eswai/zmk-naginata)（MIT） |
| **新下駄配列** | kouy 氏の配列（2010）。単打、中指シフト、薬指シフトの 3 面。拗音は [I] か [O] との同時押し | [kouy 氏のブログの発表記事](https://kouy.exblog.jp/13627994/) |

---

## 2. 比較表

「キー」は論理キーの数（Plover のシステム定義の `KEYS`）。

| 項目 | メジロ式 v2 | ソクタイプ（Plover 版） | ステノワード（商用 / Plover 版） | まち針 | Chidori / kana | 薙刀式 | 新下駄 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **キー配置** | 独自（左右対称、各手 2 段×4 指＋親指 3）。Plover 標準の steno 機（Gemini PR / Plover HID）の物理キーに割り当て直す表と、QWERTY の表がある | 独自（左右 8 ＋親指 t, k, i, n ＋ `*`）。Tréal 前提 | 独自（子音 5 ＋母音 5 ＋ L M N R ＋ `#`） | はちどり系の独自 | **英語の steno 配列のまま** | 通常のキーボード配列（26 キー＋親指シフト） | 通常のキーボード配列 |
| **キー数** | 24（`#`、左右各 11、`*`）。専用機 Mejiro31 は物理 31 キー | 22 | 商用は文字キー 10。Plover 版は 15 | 不明 | 英語の 23 前後 | 26＋シフト | 30 前後（3 段×10） |
| **1 打の範囲** | 片手で 1 音節＋追加音（ん・つ・く・っ・ち・き・ー）、二重母音も 1 打。両手で 2 音節（「にゅうりょく」が 1 打、v1 で最大 17 字相当） | 左右の 2 かな＋親指の挟み音で二字熟語 | 2〜20 字（語の略語を含む） | 1:1 を目標 | 1 かな程度 | 1 かな（拗音は 2 かな） | 1 かな |
| **公開・ライセンス** | 理論は note で公開。コードと辞書は **GPL-3.0** | 理論は資料集（PDF）。Plover 版のリポジトリに**ライセンス表記なし** | 商用。特許は失効、教本の著作権は不明。Plover 版は**ライセンス表記なし** | **ライセンス表記なし** | Chidori: GPL-3.0、kana: CC-BY-4.0 | 配列の利用条件は未確認。ZMK 実装は MIT | 記事にライセンス表記なし |
| **実装の資料** | Plover システムプラグイン（`system.py`）＋ **Python 辞書**（`mejiro.py` が規則で組み立てる。`LONGEST_KEY = 1`）＋ JSON（ユーザー略語、コマンド） | システムプラグイン＋ JSON 辞書 | システムプラグイン＋ JSON 辞書（約 2,650 行） | システムプラグイン＋ Python 辞書 | Python 辞書 | エミュレータの設定、QMK/ZMK のコード | エミュレータの設定ファイル |
| **習得** | 覚えるのは五十音の組み立て（子音 4 キー×母音 4 キーの組み合わせ）が中心。略語なしでも打てる設計 | 略語が多い（法廷向け） | 略語が必須で、暗記量は最大（比較記事） | 不明 | 英語 steno の経験者向け | 30 時間で見ずに打て、100 時間で快適（紹介記事） | 中程度 |
| **コミュニティ** | 最も活発（作者の note、X、協会 Wiki、ZMK/AHK の派生） | 小さい | 業界内（研修あり）。オープン版は休眠 | 小さい | ごく小さい | 大きい（かな配列で最大級） | 中程度 |
| **略語辞書** | ユーザー略語（`abbreviations.py` の `USERS_MAP` か `mejiro_users.json`）、一般略語（これ・ところ など、出力は**かな**）、動詞の規則生成（活用は親指キー） | 多い | 多い（例: `LH` → わたくし、`PK` → ます） | 不明 | なし | なし | なし |
| **出力** | **かな**（読み）。タイピングモードでローマ字か JIS かなのキー入力も出せる | かな | かな | かな（語を当てる案あり） | ローマ字（IME に渡す） | かな（IME に渡す） | かな（IME に渡す） |
| **ikari との相性** | ◎ 出力がかなで、略語もかななので「読み→変換」にそのまま乗る | △ 機械と略語の壁 | △ ライセンスと資料の壁 | × 未完成 | ○ 仕組みは近いが速くない | ○ ただし速記ではない（1 打 1 かな） | ○ 同上 |

メジロ式の比較記事（v1 時点）は、ステノワード、ソクタイプ、まち針、メジロ式を比べ、メジロ式を「キー数は最多だが、略語なしで 1 打の入力が多い」と位置づけている（[note](https://note.com/jeebis_keyboard/n/nf5ec466bd37a)）。

---

## 3. IME 内で実装するときの技術的な論点

### 3.1 macOS IMK でストロークを検出する

速記は「押しているキーの集合」を、**全キーが離れた時点**で 1 打として確定させるのが基本（all-up）。英語の Plover には、最初のキーが離れた時点で確定させる first-up もある。どちらも **keyUp が要る**。

- IMK の `recognizedEvents(_:)` は既定で keyDown だけを返す（SDK のヘッダ `IMKInputController.h` の説明: "The default implementation returns NSKeyDownMask"）。flagsChanged は加えられる（Rime の Squirrel は `.keyDown, .flagsChanged` を返している: [SquirrelInputController.swift](https://github.com/rime/squirrel/blob/master/sources/SquirrelInputController.swift)）。
- **keyUp は、マスクに加えても IMK から届かない**という報告がある（[Apple Developer Forums #5446](https://developer.apple.com/forums/thread/5446)、2015 年、rdar://21376535。返信なし）。現行の macOS で直ったかは確認できなかった。**実機で確かめる必要がある**（ログを出すだけの小さな実験で済む）。
- ikari の今の殻は `event.type == .keyDown` 以外を捨てている（`macos/Sources/IkariIME/IkariInputController.swift` の `handle(_:client:)`）。keyUp が届く場合も、ここを変える必要がある。
- keyDown だけで同時押しを判定する方法（最初の keyDown から一定時間に来たキーを 1 打とみなす）はある。ただし、離した時点が分からないので、速く打つと次の打鍵と混ざる。押しっぱなしのオートリピート（`isARepeat`）も除く必要がある。速記の打ち方には向かない。
- IMK の外でキーを拾う方法（CGEventTap、IOHIDManager）は、殻のプロセスでなら使える（ikari.app は Sandbox なし）。ただし、キーボードを監視するには「入力監視」などの権限が要る。権限の要否は実機で確かめる。

**結論**: IMK だけで速記の打鍵を正しく取れる見込みは低い。打鍵の判定はキーボード側（ファームウェア）に任せ、ikari には「確定した 1 打」を渡すのが筋がよい（3.5）。

### 3.2 ロールオーバーと NKRO

- ホスト（ikari か Plover）で同時押しを判定するなら、キーボードは**全キーの同時押しを報告できる**必要がある。6KRO では両手の大きなストローク（メジロ式は最大 24 キー）が欠ける。QMK のドキュメントも、QWERTY のキーボードで Plover を使うなら NKRO を勧めている（[QMK: Stenography](https://docs.qmk.fm/features/stenography)）。
- ZMK の NKRO は `CONFIG_ZMK_HID_REPORT_TYPE_NKRO=y`（[Wiki: NKROの設定](https://scrapbox.io/Stenotype-Japan/NKRO%E3%81%AE%E8%A8%AD%E5%AE%9A)）。
- **ファームウェアで判定するなら、ホスト側の NKRO は要らない**。キーボードがマトリクスの状態を見て判定し、結果だけを送るため。ただし、マトリクス自体がゴーストを起こさないこと（ダイオードあり）は要る。ZMK のメジロ式モジュールの作者は、Charlieplexing のキーボードでは NKRO にしても同時押しの成功率が低かったので、ファームウェアで判定する版を作ったと書いている（[zmk-behavior-mejiro](https://github.com/hamehame2/zmk-behavior-mejiro)）。

### 3.3 Plover の辞書形式とシステム定義をそのまま読めるか

- **JSON 辞書**: キーはストローク（`/` 区切り）、値は Plover の翻訳言語の文字列（[Plover: Dictionary Formats](https://plover.readthedocs.io/en/latest/dict_formats.html)）。値には `{^}`（つなげる）、`{#Return}`（キー入力）、`{plover:...}`（コマンド）などのメタ記法が混じる（[Plover: Translation Language](https://plover.readthedocs.io/en/latest/translation_language.html)）。ikari で読むなら、文字列の部分だけを取り、メタ記法は一部（`{^}` と `{^^}` を無視、改行など）だけ解釈する、が現実的。
- **システム定義**: Python のモジュールで、`KEYS`、`IMPLICIT_HYPHEN_KEYS`、`UNDO_STROKE_STENO`、`KEYMAPS` などを持つ（[Plover: Systems](https://plover.readthedocs.io/en/latest/plugin-dev/systems.html)）。Python なので Rust から直接は読めない。中身は定数の表なので、**手で TOML などに写す**のが早い。
- **メジロ式の本体は JSON ではない**。`mejiro.py` が正規表現でストロークを左右の子音・母音・追加音に分け、`settings.py` の表で組み立てる（約 1,200 行の Python）。JSON はユーザー略語とコマンドだけ。したがって ikari では、**メジロ式の規則を Rust で実装し直す**ことになる。規則は表（子音 16 通り×母音 8 通り、二重母音、例外、助詞）なので、移植の量は大きくない。
- ライセンス: メジロ式のコードと表は GPL-3.0。表をそのまま写すと、ikari に GPL がかかるかが問題になる（ikari はまだライセンスを決めていない。NFR-41〜45）。「判断してもらう点」の Q3。

### 3.4 取り消し（`*`）

- Plover の `*` は「直前のストロークを取り消す」（`=undo`。[Translation Language](https://plover.readthedocs.io/en/latest/translation_language.html)）。メジロ式は `*` を略語の修飾に使い、取り消しは `UNDO_STROKE_STENO = '-U'` になっている（[system.py](https://github.com/JEEBIS27/Plover_Mejiro/blob/main/Mejiro/system.py)）。
- ikari での扱いの案:
  - **未確定の読みの中**: 1 打ごとに「何文字を足したか」を積んでおき、取り消しの 1 打で最後の 1 打分を消す。BackSpace（1 文字）より粒度が粗いだけで、spec の「未確定中の Esc／BackSpace で読みに戻る」と同じ側の操作になる。
  - **変換中**: 候補を閉じて読みに戻し、最後の 1 打分を消す。
  - **確定後**: spec が「確定後のアンドゥは実装しない」と決めているので、取り消しはしない（通常の BackSpace をアプリに渡す）。略語は即時確定なので、略語の取り消しもここに入る。
- ファームウェアで判定する場合も、取り消しは ikari が持つ（ファームウェアは「この 1 打は取り消しキーだった」と送るだけ）。ファームウェアが BackSpace を連打する方式（ZMK のメジロ式モジュールのような、ローマ字のキー入力を送る方式）は、ikari の未確定の読みとずれやすい。

### 3.5 ファームウェア側で処理する案との比較

| 案 | 仕組み | 長所 | 短所 |
| --- | --- | --- | --- |
| **A. ファームウェアがかなを解釈し、ローマ字のキー入力を送る** | ZMK の [zmk-behavior-mejiro](https://github.com/hamehame2/zmk-behavior-mejiro)、QMK の Mejiro31 版。ikari は普通のローマ字入力として受ける | **ikari の変更がゼロ**。今日から試せる。BLE でも動く | 略語も「ローマ字の連打」になり、**読みとして入る**（即時確定にできない）。送信が速すぎると IME が追いつかず、モジュールは 1 キー 25 ms の遅延を入れている。1 打の取り消しができない |
| **B. ファームウェアは打鍵の判定だけをし、「1 打」を符号化して送る** | all-up で確定した押下集合を、ikari だけが解釈する専用のキー列（例: 前置キー＋ビットを表すキー）で送る。ikari が規則（メジロ式など）と略語を Rust で解釈する | 普通の HID キーボードとして送るので BLE でも動き、権限も要らない。ホスト側の NKRO が要らない。略語の即時確定、1 打の取り消し、モードの管理が ikari でできる | ファームウェアの改造が要る（ZMK のモジュールか behavior を書く）。符号の取り決めが ikari 専用になる |
| **C. Plover HID を ikari が直接読む** | キーボードは押下と解放のたびに 64 ビットの状態を送る（usage page 0xFF50、report ID 0x50。[plover-machine-hid](https://github.com/dnaq/plover-machine-hid)、[Plover wiki: Plover HID](https://plover.wiki/index.php/Plover_HID)）。ikari.app が IOHIDManager で開いて、all-up も first-up も自分で判定する | 生のキー状態が取れる、最も「本物の速記機」に近い形。QMK は Plover HID を本体で持つ（[QMK: Stenography](https://docs.qmk.fm/features/stenography)） | **ZMK は本体未対応**（[PR #962](https://github.com/zmkfirmware/zmk/pull/962) は WIP のまま。2026-02 に「本体ではなくモジュールで」と保守者がコメント。モジュールは [zmk-hid-io-plover-hid](https://github.com/petercpark/zmk-hid-io-plover-hid)、[zmk-behavior-plover64](https://github.com/hamehame2/zmk-behavior-plover64)）。BLE での動作報告がまちまち。殻に HID の読み取りを足す必要があり、権限の要否も未確認 |
| **D. ZMK/QMK の combo で略語だけ出す** | combo に専用キー（F13〜F24 など）や文字列マクロを割り当てる | 第 1 段階（略語）には十分。ikari 側は「専用キー→定型文」の表を持つだけ | combo はキーの組み合わせごとに 1 つ定義する方式で、速記の理論（数千通り）は書けない。ZMK は同時に有効な combo の数などに上限がある（[ZMK: Combos の設定](https://zmk.dev/docs/config/combos)）。combo は「押した時点」で発火するので（[ZMK: Combos](https://zmk.dev/docs/keymaps/combos)）、all-up の判定とは違う |
| **E. Plover を前に置く** | Plover＋メジロ式のタイピングモード（ローマ字）で、ikari にローマ字を送る | ikari の変更がゼロ。メジロ式の本家をそのまま使える | macOS でキーボードを監視する権限が要り、Plover の常駐が要る。略語は A と同じく読みとして入る。ホストで判定するので NKRO が要る |
| **F. IMK の keyDown だけで判定する** | 3.1 の時間窓の方式 | 追加のソフトもファームウェアも要らない | 誤判定が多い。keyUp が届けば all-up にできるが、届かない報告がある |

---

## 4. 基本原則との整合

spec の基本原則は「ユーザーが入力した文字列を起点とする。入力していない内容を先回りして補完しない。最終的な表現の選択権は常にユーザーにある」。

### 4.1 速記で打った音

速記の 1 打は「複数のかなを一度に入力する操作」で、ローマ字の「kyo」で「きょ」を打つのと同じ種類のもの。出力は打鍵から一意に決まり、推測が入らない。読みとして変換に渡すのは、FR-06（入力した読みと完全一致する候補だけを出す）に合う。メジロ式の一般略語（「これ」「ところ」など）と動詞の規則生成も、出力が**かな**で、打鍵から一意に決まるので、同じ扱い（読みに足す）でよい。

まち針の「子音だけから和語を当てる」案は、打鍵から語が一意に決まらず、候補から推測することになるので、原則とぶつかる。採らない。

### 4.2 略語（定型文）の即時確定

略語の展開は、**ユーザーが明示的に入力したものとして扱える**と考える。理由:

- 対応表を作ったのはユーザー自身で、打鍵と出力が 1 対 1 で決まっている。IME が文脈から選ぶわけではない。
- 「ありがとうございます」を 1 打で打つのは、その文字列を選んで入力したのと同じ。補完（入力の途中から続きを推測すること）ではない。

原則に合わせるために守る条件:

1. **ストロークの完全一致だけで展開する**。途中の打鍵から候補を出したり、似たストロークに寄せたりしない。
2. **文脈で展開先を変えない**。同じ打鍵は常に同じ文字列になる。
3. **展開してよいのは、ユーザーが登録したか、明示的に有効にした表だけ**。同梱の表を黙って有効にしない。
4. 確定した定型文は、ほかの確定と同じく**左文脈として次の変換に渡す**（錨）。ただし「読み→語」の学習には入れない（読みを経ていないので、学習のキーがない）。
5. 未確定の読みがあるときに略語を打ったら、どうするかを決める（Q5）。案: 読みを今の表示のまま確定してから定型文を確定する（英数キーと同じ扱い）。読みを勝手に変換して確定するのは、選択権をユーザーから奪うので避ける。

### 4.3 そのほか

- 速記モードの切り替えは、ユーザーの操作でだけ行う（文脈から自動で切り替えない）。
- 次語予測（FR-08）、ライブ変換はしない。速記モードでも、変換はスペースで明示的に行う。

---

## 5. 推奨

### 5.1 方式: **メジロ式 v2 を採る**

- 出力がかな（略語もかな）で、ikari の「読み→変換」にそのまま乗る。漢字の表記を引く方式（ステノワードやソクタイプの略語）は、ikari の変換と役割がぶつかる。
- 略語なしでも 1 打で 1〜2 音節が打てる。略語を大量に覚えなくても速くなる。短い確定を重ねる ikari の書き方（「単語/単位/で/確定」）にも合う。
- 理論が公開され、2026 年も更新され、派生実装（ZMK、AHK、QMK）がある。日本語のオープンな方式で最も活発。
- 左右対称で、キー数 24 は 30〜40% の自作キーボードに収まる。
- 弱点: GPL-3.0（Q3）。キー数が多い（親指 3 キー×2）。

ステノワード、ソクタイプは、ライセンスと資料の不足、略語の暗記量で外す。薙刀式・新下駄は速記ではなく、ファームウェアや既存のエミュレータで ikari の外から使えるので、ikari に入れる必要がない。

### 5.2 入力の経路: **B（ファームウェアで打鍵を判定し、1 打を ikari に送る）を本命にする**

- IMK は keyUp が届かない可能性が高く（3.1）、IMK だけで all-up を判定するのは危うい。
- B なら、BLE の ZMK でも動き、権限が要らず、ホスト側の NKRO も要らない。略語の即時確定と 1 打の取り消しを ikari が持てる。
- C（Plover HID）は、ZMK の本体対応が進んでいないので、将来の選択肢にとどめる。QMK に移るなら有力。
- A（既存の ZMK モジュールでローマ字を送る）は、ikari の変更なしで今日から試せるので、**理論を試す期間に使う**。

### 5.3 ikari 側の置き場所（案）

- 規則（ストローク→かな）と略語の表の引き当ては、辞書に依存しない純粋な処理なので `input` crate に置く（ローマ字と同じ扱い。`shell-ffi` 経由で殻から呼ぶ）。
- 速記モードは殻の状態機械（`IkariShellCore`）のモードとして持つ。読みに足したかなは、今のローマ字入力と同じ経路でエンジンの変換に渡る。エンジン（`core`、`engine`）は変えない。
- 略語の表はユーザーデータ（`~/Library/Application Support/ikari-ime/` に TSV か JSON）。Plover の JSON を取り込む機能は、文字列の部分だけを読む。

---

## 6. 段階的な計画

| 段階 | 内容 | ikari の変更 | 完了の目安 |
| --- | --- | --- | --- |
| **0. 試す** | (a) 自分のキーボードに [zmk-behavior-mejiro](https://github.com/hamehame2/zmk-behavior-mejiro) を入れ、ローマ字として ikari に流す（案 A）。メジロ式が手に合うかを確かめる。(b) IMK に keyUp が届くかを、ログだけの実験で確かめる | なし（(b) は捨てる実験のコード） | メジロ式を続けるかを決める。keyUp の可否が分かる |
| **1. 同時押し→略語** | ZMK の combo（案 D）か、段階 2 の B の符号で「略語の ID」を送る。ikari は速記モードで ID を受け、ユーザーの略語表から定型文を即時確定する。未確定の読みがあるときの扱い（4.2 の 5）を決める | 殻: 速記モード、専用キーの受け取り。`input`: 略語表の引き当て。ユーザーデータ: 略語表 | 「ありがとうございます」などを 1 打で確定でき、左文脈として次の変換に渡る |
| **2. 速記理論による音の入力** | ファームウェアが all-up で確定した押下集合を符号化して送る（案 B）。ikari がメジロ式の規則でかなにし、読みに足す。スペースで変換。取り消しは 1 打単位 | `input`: メジロ式の規則（子音、母音、二重母音、追加音、助詞、例外）。殻: 1 打の受け取り、1 打単位の取り消し。ファームウェア: ZMK の behavior | ローマ字入力と同じ評価セットの読みを、速記で打って同じ変換結果になる |
| **3. 略語と動詞の拡充** | メジロ式の一般略語・動詞の規則生成（どちらもかな出力）を読みとして入れる。Plover の JSON 辞書（ユーザー略語）の取り込み | `input`、ユーザーデータの取り込み | 自分の文章で、ローマ字入力より打鍵数が減る |
| **4.（任意）Plover HID** | ZMK の Plover HID が安定するか、QMK に移ったら、殻が HID を直接読む（案 C）。first-up も選べるようにする | 殻: IOHIDManager | ファームウェアの符号を ikari 専用にしなくてよくなる |

設定アプリ（ADR 0005）に、略語表の編集画面を足すのは段階 3 以降。

---

## 7. ユーザーに判断してもらう点

1. **Q1 鉄器式の出典**: 見つからなかった。どこで見たか教えてほしい。はやとち（はやとくん？）、ステンチュラ系の日本語理論も同じ。
2. **Q2 キーボード**: 使っているのは ZMK か QMK か。BLE で使うか。親指キーはいくつあるか（メジロ式は片手に 3 つ要る）。マトリクスにダイオードがあるか。
3. **Q3 ライセンス**: メジロ式の表（GPL-3.0）を ikari に写してよいか。選択肢: (a) ikari を GPL にする、(b) 自分用なので当面は気にしない（配布するときに決める）、(c) note の解説から規則を書き起こし、コードは写さない、(d) 作者に相談する。
4. **Q4 入力の経路**: 本命を B（ファームウェアで判定し、ikari 専用の符号で送る）にしてよいか。ファームウェアの改造を避けたいなら、F（IMK の keyDown だけ）か E（Plover を前に置く）になるが、精度か手間で劣る。
5. **Q5 略語と未確定の読み**: 読みが残っているときに略語を打ったら、(a) 読みを表示のまま確定してから定型文を確定、(b) 略語を無視して警告、(c) 定型文を読みの後ろに足して未確定のままにする、のどれにするか。推奨は (a)。
6. **Q6 速記モードの持ち方**: 入力メニューの 3 つ目のモード（日本語・英数・速記）にするか、日本語モードの中のトグルにするか。ファームウェアがレイヤーを切り替えるのと同時に、ikari のモードも切り替える仕組み（専用キーを送る）が要るか。
7. **Q7 メジロ式の助詞の打鍵**: メジロ式は親指だけで「に・は・の・が」などを打ち、右親指の単独打鍵に Return や Space を当てている。ikari の「スペースで変換」とぶつかるので、変換キーをどこに置くかを決める。
8. **Q8 略語の学習**: 略語で確定した文字列を、学習（左文脈＋読み→語）に入れないことでよいか。

---

## 出典（本文で使ったもの）

- メジロ式: [Plover_Mejiro](https://github.com/JEEBIS27/Plover_Mejiro)（GPL-3.0、`system.py`、`mejiro.py`、`settings.py`、`abbreviations.py` を読んだ）、[v2 基本入力](https://note.com/jeebis_keyboard/n/ne8430676220b)、[他の速記と比較する](https://note.com/jeebis_keyboard/n/nf5ec466bd37a)、[協会 Wiki](https://scrapbox.io/Stenotype-Japan/%E3%83%A1%E3%82%B8%E3%83%AD%E5%BC%8F)、[zmk-behavior-mejiro](https://github.com/hamehame2/zmk-behavior-mejiro)
- ソクタイプ: [plover_japanese_sokutaipu](https://github.com/na4zagin3/plover_japanese_sokutaipu)（`system.py` を読んだ）、[ソクタイプ資料集](https://sokkidouraku.lsv.jp/001c/index.html)
- ステノワード: [スピードワープロ](https://www.speed-wp.co.jp/speed-wp-stenoword/)、[plover-japanese-stenoword](https://github.com/na2hiro/plover-japanese-stenoword)（`system.py`、`StenoWord.json` を読んだ）、[協会 Wiki](https://scrapbox.io/Stenotype-Japan/%E3%82%B9%E3%83%86%E3%83%8E%E3%83%AF%E3%83%BC%E3%83%89)
- まち針: [Plover_Japanese_Machibari](https://github.com/Meltedsoftcream/Plover_Japanese_Machibari)、[協会 Wiki](https://scrapbox.io/Stenotype-Japan/%E3%81%BE%E3%81%A1%E9%87%9D)
- 英語配列の日本語: [plover-chidori](https://github.com/paulfioravanti/plover-chidori)、[plover_japanese_kana](https://github.com/Willem3141/plover_japanese_kana)
- かな配列: [GIZMODO: 薙刀式](https://www.gizmodo.jp/2026/05/naginata-ime.html)、[zmk-naginata](https://github.com/eswai/zmk-naginata)、[新下駄配列](https://kouy.exblog.jp/13627994/)
- Plover: [Dictionary Formats](https://plover.readthedocs.io/en/latest/dict_formats.html)、[Translation Language](https://plover.readthedocs.io/en/latest/translation_language.html)、[Systems](https://plover.readthedocs.io/en/latest/plugin-dev/systems.html)、[Plover HID（wiki）](https://plover.wiki/index.php/Plover_HID)、[plover-machine-hid](https://github.com/dnaq/plover-machine-hid)
- ファームウェア: [QMK: Stenography](https://docs.qmk.fm/features/stenography)、[ZMK: Combos](https://zmk.dev/docs/keymaps/combos)、[ZMK: Combos の設定](https://zmk.dev/docs/config/combos)、[ZMK PR #962](https://github.com/zmkfirmware/zmk/pull/962)、[協会 Wiki: 速記キーボードの作り方](https://scrapbox.io/Stenotype-Japan/%E9%80%9F%E8%A8%98%E3%82%AD%E3%83%BC%E3%83%9C%E3%83%BC%E3%83%89%E3%81%AE%E4%BD%9C%E3%82%8A%E6%96%B9%20(QMK%2C%20ZMK))、[協会 Wiki: NKROの設定](https://scrapbox.io/Stenotype-Japan/NKRO%E3%81%AE%E8%A8%AD%E5%AE%9A)
- macOS IMK: SDK の `InputMethodKit.framework/Headers/IMKInputController.h`、[Apple Developer Forums #5446](https://developer.apple.com/forums/thread/5446)、[Squirrel の InputController](https://github.com/rime/squirrel/blob/master/sources/SquirrelInputController.swift)
- コミュニティ: [日本ステノタイプ協会 Wiki](https://scrapbox.io/Stenotype-Japan/)、[toyboot4e のスクラップ](https://zenn.dev/toyboot4e/scraps/af274eca435d20)、[大岡俊彦「ステノが広まらない理由」](https://oookaworks.seesaa.net/article/519844611.html)

## 8. ユーザーの決定（2026-10-12）
- キーボードは ZMK（Bluetooth）。入力の経路は、ファームウェアが1打を判定する案 B を前提に設計する。
- **ikari には汎用の速記エンジンだけを入れ、メジロ式のコードや表は同梱しない。** キー配置、組み合わせからかなへの表、略語辞書は、ユーザーが作る定義ファイルから読み込む。これで GPL-3.0 の素材が ikari 本体に入らないようにする。設計の最初に、メジロ式 v2 の規則を定義ファイルの形で再現できるかを確かめ、できなければ速記は実装しない。
- 実装の時期は、いまの作業（フェーズ3a など）がひととおり終わってからにする。
