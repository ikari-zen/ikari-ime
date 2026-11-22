# ADR 0080: 配布物にモデルと辞書を同梱する（通常版）と、通信を持たないオフライン版。置き場所、パッケージの形、版の分け方、更新

- 状態: 承認（2026-11-22 ユーザー。macOS の配布の形（dmg）と §12 の Q1〜Q3 もユーザーが決定済み）
- 日付: 2026-11-22
- 対象: `engine`（辞書とモデルの解決）、`stack`、`ikari-resource`、`shell-core`・`shell-ffi`・`windows/settings`（版の機能の判断）、`xtask`（検査、辞書のビルド）、`macos/scripts/build.sh`、`windows/installer/`、`.github/workflows/release.yml`、`THIRD_PARTY_NOTICES.md`
- 前提: ADR 0058（inbox と束）、0060（モデルの選択。更新 2026-11-19）、0066（前提。R1〜R6、通信しない）、0067（Windows の置き場所）、0069（真正性）、0070（更新）、0072（檻の段）、0076（ikari-data の契約）、0077（macOS の LM の檻）、0079（macOS の ikari-data）。調査は [license-redistribution-2026-11.md](../research/license-redistribution-2026-11.md)（#242）
- 判断の置き場所: すべて Rust（辞書とモデルをどこから読むか、版がどの機能を持つか）。Swift と C# は、Rust が返す「出してよい機能」に従って、ボタンを出す・出さないだけ。パッケージを作る指定はシェル（macOS）と Inno Setup（Windows）と workflow で、判断は持たない
- 関連: #490、#242、#246、#241、#99、#479、#162

## 0. ユーザーの決定（覆さない）

- 2026-11-19: 通常版も、公式のままの zenz-v3.2-small と、デプロイ時点の辞書を、配布物に同梱する。通常版の通信は、自由指定のモデルの取得と版の確認だけ（ボタンを押したときだけ）。最初のセットアップの取得の同意画面は作らない。オフライン版は、通信のロジック（`ikari-data` と通信ライブラリ）を持たない版。ソースは 1 つで、コンパイル時とパッケージ作成時の指定で作る。オフライン版に通信の部品が入っていないことを CI で検査する。通信の機能を出すかの判断は Rust 側。ファイルからの取り込み（#246）が要る。オフライン版の更新はパッケージの入手し直し。
- 2026-11-22: 辞書とモデルは 0.1.0 から同梱する（取得の手順だけで出す案は取らない）。配布は先に GitHub Release、そのあとで Homebrew と winget（#479）。未署名の配布（ADR 0069）。
- 2026-11-22: **macOS の配布の形は dmg。** dmg の中に `~/Library/Input Methods` へのショートカットと案内を入れる。pkg は管理者の権限でスクリプトが走る点を懸念して見送り。未署名の警告の越え方は #241 の実機の確認で確かめる。
- 2026-11-22（§12 の Q1〜Q3）: dmg の案内は `~/Library/Input Methods` だけ（`/Library/Input Methods` は並べない。利用者が束を差し替えられる限界を `SECURITY.md` に書く）。外来語の一覧を含む `system.dict` を CC BY-SA 4.0 として示す。`vectors.bin` は同梱する（話題の手がかりの主力で効果が大きい。ADR 0003 の実験で ext 30.9% → 58.6%）。

## 1. 分かったこと（2026-11-22）

### 1.1 同梱するものの大きさ（手元のファイルで測った。`data/build/dict/` は 2026-11-10 のビルド）

| ファイル | 中身 | 大きさ | gzip -6 | xz -6 |
| --- | --- | --- | --- | --- |
| `dict/system.dict` | 語彙（SudachiDict、chiVe の頻度、ABR、外来語の一覧） | 242.3MB | 85.9MB | 62.4MB |
| `dict/vectors.bin` | chiVe のベクトル（話題の手がかり。R3） | 336.7MB | 220.5MB | 209.3MB |
| `dict/grammar.bin` | 接続表 | 71.7MB | 25.4MB | 13.6MB |
| `dict/postal.tsv` | 郵便番号（ABR） | 24.2MB | 2.7MB | 2.0MB |
| `dict/emoji.tsv` | 絵文字 | 0.04MB | 0.01MB | 0.01MB |
| `models/zenz-v3.2-small-Q5_K_M.gguf` | モデル（配布元のまま） | 73.9MB | 71.7MB | 71.3MB |
| **合計** | | **748.8MB** | **406.2MB** | **358.6MB** |
| （参考）`vectors.bin` を除く | | 412.1MB | 185.7MB | 149.3MB |

- 実行ファイル（llama.cpp、Metal・Vulkan のバックエンド、.NET と Windows App SDK）は数えていない（未計測）。
- `vectors.bin` が半分近くを占め、ほとんど縮まない（浮動小数の量子化済みのため、と推測）。`stack` は `vectors.bin` が無ければ使わずに動く（`stack/src/lib.rs` の `vectors`。話題の手がかり R3 が効かなくなる）。効果は ADR 0003 の実験で ext 30.9% → 58.6% と大きく、同梱する（§12 の Q3）。
- GitHub の Release の 1 ファイルの上限は 2 GiB（GitHub の文書）。どの形でも収まる。

### 1.2 ライセンス（#242 の結果。法的助言ではない）

- 配れないデータは無かった。同梱するものと義務は [調査 §1](../research/license-redistribution-2026-11.md) の表のとおり（zenz と外来語の一覧は CC BY-SA 4.0、SudachiDict と chiVe は Apache-2.0（SudachiDict は `LEGAL` に UniDic BSD-3-Clause と NEologd を含む）、ABR は PDL1.0、郵便番号は PDL1.0 と日本郵便の「著作権を主張しない」、絵文字は MIT）。
- 未改変の zenz を別ファイルで同梱するのは集合物に近く、本体（Apache-2.0）に ShareAlike は及ばないと読める（調査 §2.3。残るグレーは 2026-11-19 にユーザーが受け入れた）。
- 外来語の一覧（CC BY-SA 4.0）を `system.dict` に混ぜて同梱するときの扱いは、調査 §3.2 で 3 案が示された。**`system.dict` を CC BY-SA 4.0 として示すと決めた**（2026-11-22 ユーザー決定。§4 の 5、§12 の Q2）。
- Windows のインストーラは、今は `THIRD_PARTY_NOTICES.md` を入れていない（`windows/installer/ikari.iss` の `[Files]`）。

### 1.3 今の置き場所と読み方

- 辞書もモデルも、利用者のデータの場所（macOS は `~/Library/Application Support/ikari-ime/`、Windows は `%LOCALAPPDATA%\ikari-ime\`）の `dict/` と `models/` から読む。開発者が `install-data.sh` で置く。
- エンジンは起動時に辞書を開けないと起動に失敗する（ADR 0058 §1.3）。モデルは、既知の sha256 の表と照らして通らなければ LM なしで動く（`engine/src/model_select.rs` の `resolve_preset`）。
- Windows のインストーラは `%LOCALAPPDATA%` に触れない（ADR 0067 §3.3）。実行物とインストーラが置く物は `C:\Program Files\ikari-ime\` だけ（§3.1）。
- `xtask` の `net_crates` の検査（`xtask/src/check/supply.rs`）が、通信の crate（`ureq` など）に依るのは `ikari-data` だけであることを見ている。macOS の `build.sh` は、束の中で `network.*` の entitlement を持つのが `ikari-data` だけであることを見ている（ADR 0079）。
- リリースの workflow（`release.yml`）は、macOS は ad-hoc の `ikari.app` の zip、Windows は exe と DLL の zip（仮）。辞書とモデルは入っていない。

## 2. 問題

1. 同梱するデータ（約 750MB）を、どこに置き、エンジンがどう見つけるか。Windows のインストーラは利用者のデータの場所に書けない。macOS のエンジンは App Sandbox の中にいる。
2. 利用者が後から入れた辞書やモデル（#246 の取り込み、URL の取得）と、同梱の物のどちらを使うか。
3. 未署名の macOS の配布の形は dmg に決まった（2026-11-22）。dmg の中身（置き場所への道、案内）と、未署名の警告の越え方（#241）を決める。入力メソッドは利用者がダブルクリックで開く物ではなく、OS が起動する。
4. 通常版とオフライン版を 1 つのソースから作り、オフライン版に通信の部品が無いことを機械で確かめる形。
5. 配る辞書を、作者の手元ではなく GitHub のランナーで作る経路（ADR 0069 §4）。

## 3. 決定 1: 同梱の物を、アプリの中の「同梱の場所」に置き、利用者のデータの場所を上に重ねる

### 3.1 置き場所

| OS | 同梱の場所（読むだけ） | 利用者のデータの場所（今のまま） |
| --- | --- | --- |
| macOS | `ikari.app/Contents/XPCServices/IkariEngine.xpc/Contents/Resources/data/`（`dict/`、`models/`、`NOTICE.txt`）。エンジンの XPC の束の中に置く（§3.4） | `~/Library/Application Support/ikari-ime/` |
| Windows | `C:\Program Files\ikari-ime\data\`（`dict\`、`models\`、`NOTICE.txt`） | `%LOCALAPPDATA%\ikari-ime\` |

- 同梱の場所は、エンジンが**自分の実行ファイルのパスから**決める（R2）。環境変数や設定では変えない（R5）。
- インストーラは利用者ごとの場所に何も写さない（ADR 0067 §3.3 を守る。利用者ごとに 750MB を複製しない）。
- 同梱の場所の物も、信用しない入力として読む（P5。今の magic、形式の版、`grammar_id`・`dict_id` の照合、モデルの sha256 の照合をそのまま通す）。Windows の同梱の場所は管理者だけが書ける（R1）。macOS は `~/Library/Input Methods` の束の中なので利用者が書ける（§5.4 の限界と同じ）。どちらでも検査を省かない。
- Windows の ACL の最後の検査（ADR 0067 §3.1）は、`data\` も含めてインストール先の全体にかかる（今の検査のまま）。

### 3.2 どちらを使うか（部品ごとに、上から順に、検査に通った最初のもの）

| 部品 | 1 番目 | 2 番目 |
| --- | --- | --- |
| 辞書（`grammar.bin`、`system.dict`、`vectors.bin` の組） | 利用者のデータの場所の `dict/`（取り込みか、開発者が置いたもの） | 同梱の場所の `dict/` |
| `postal.tsv`、`emoji.tsv` | 利用者のデータの場所の `dict/` | 同梱の場所の `dict/` |
| モデル | 設定 `lm.model` が指すもの。プリセットは利用者のデータの場所の `models/` → 同梱の場所の `models/` の順。自由指定は利用者のデータの場所だけ（ADR 0060） | — |

- 理由: 利用者が入れた物を正とする（取り込みは利用者の操作の結果。ADR 0058）。同梱の物は「何も入れていない人の既定」。
- **1 番目が検査に落ちたら、ログに書いて 2 番目を使う**。今の「起動時に辞書を開けないとエンジンが起動しない」（ADR 0058 §1.3）を改める。同梱の辞書があれば、壊れた辞書や形式の古い辞書（アプリを更新したあとに残った物）で変換が止まらない。
- 辞書の 3 ファイルは組で選ぶ（`grammar_id`・`dict_id` の照合があるので、場所をまたいで混ぜない）。
- どちらを使っているかと、その `BUILD_INFO` の日付を、エンジンが状態として返す。設定画面は「同梱の辞書（日付）」か「入れた辞書（日付）」を出し、入れた辞書が同梱より古いときは知らせる。「同梱に戻す」は、利用者のデータの場所の `dict/` を `dict.prev/` に移すこと（担い手はエンジン。ADR 0058 §7.1 の「書き手を増やさない」を守る）。自動では戻さない。
- 判断は `engine`（場所の候補の列と選び方）と `stack`（組の検査）。OS ごとの差は「同梱の場所のパスを決める関数」1 つだけにする。

### 3.3 反映

- 同梱の物はアプリの更新でだけ変わる。macOS は束ごと置き換わり、動いているエンジンは古い inode の mmap のまま動き、次の起動で新しい物を読む（rename の性質。ADR 0058 §7.1）。Windows はインストーラがエンジンを止めてから置き換える（ADR 0067 §3.2）。
- 取り込み（ADR 0058 §7.2）の手順は変えない。書く先は利用者のデータの場所だけ。

### 3.4 macOS で XPC の束の中に置く理由（未確認を含む）

- 読むのはエンジン（`IkariEngine.xpc`。App Sandbox、ネットワークなし）。Sandbox の中のプロセスは自分の束を読める（Apple の文書の一般の説明。この構成では未確認）。親の `ikari.app` の `Resources` を XPC から読めるかは確かめていないので、確実な自分の束の中に置く。
- モデルは、エンジンが開いた fd を檻役に渡す（ADR 0077 の fd 3）。檻役と LM の子は同梱の場所を読まない（ADR 0077 の「檻役はコンテナから読まない」と同じ考え）。
- 束の署名（ad-hoc でも）が `Resources` を封印する（build.sh の検査で `CodeResources` に載っていることを見る）。750MB を封印すると、署名と最初の起動の検証に時間がかかるかもしれない（推測。測る）。

## 4. 決定 2: ライセンスの表示の置き場所

1. **`THIRD_PARTY_NOTICES.md`** を、同梱の実態に合わせて書き直す（調査 §6 の 1。zenz の節は調査 §2.2 の文面、辞書の各節の「含めない」「各自が生成」を改める、llama.cpp の節を `ikari-lm-host` に合わせる）。
2. 配る物の中の置き場所:
   - macOS: 今の `IkariSettings.app/Contents/Resources/THIRD_PARTY_NOTICES.md`（設定アプリの「このソフトについて」で表示）に加え、同梱の場所に `NOTICE.txt`（どのファイルがどのライセンスか、全文はどこか、の短い案内）と `licenses/`（Apache-2.0 の本文、SudachiDict の `LEGAL`、MIT の本文。CC BY-SA 4.0 と PDL1.0 は URI で足りる（調査 §2.2・§1）が、確かめやすさのため名前と URI を書く）。
   - Windows: `{app}\THIRD_PARTY_NOTICES.md` を `[Files]` に足す（今は無い）。`{app}\data\NOTICE.txt` と `{app}\data\licenses\`。設定画面の「このソフトについて」で表示する。
3. データのファイルのそばに `NOTICE.txt` を置く理由: 辞書やモデルのファイルだけを写して持ち出す人にも表示が届くようにする（CC BY-SA 4.0 §3(a)(2) の「合理的な方法」の側に倒す）。
4. Release のノートに、同梱したデータの一覧（名前、版、ライセンス、出典）を載せる（版を出す手順に足す）。
5. **外来語の一覧を含む `system.dict` は、そのまま同梱し、`system.dict` のファイルを CC BY-SA 4.0（と SudachiDict・chiVe・ABR の各表示）として示す**（2026-11-22 ユーザー決定。§12 の Q2。調査 §3.2 の案 A）。本体（Apache-2.0）とは別のファイルで、本体には及ばない。`THIRD_PARTY_NOTICES.md` と `NOTICE.txt` にそう書く。

## 5. 決定 3: macOS の配布の形は dmg（2026-11-22 ユーザー決定）

### 5.1 dmg の中身

- `ikari.app`（ad-hoc の署名。同梱のデータは §3.1 のとおり束の中）。
- `~/Library/Input Methods` へのショートカットと、短い案内（`ikari.app` をそこへ動かす、ログアウトかアプリの再起動、システム設定で入力ソースを足す、消し方、確かめ方（SHA256SUMS と証拠）の README へのリンク）。案内の文は、実機で確かめた手順だけを書く（ADR 0069 §3 の 5）。
- **ショートカットの作り方は未確認**: シンボリックリンクは `~` を展開しないので、作る時点で利用者のホームが分からない dmg には、`~/Library/Input Methods` を指す固定のリンクを置けない（事実。リンクの仕組み）。Finder の別名（alias）が開く人のホームに解決されるか、Finder の「フォルダへ移動」に貼る文字列で代えるか、を #241 の実機の確認で決める。確かめられた形が無ければ、案内の文だけにする。
- dmg は読み取りだけの圧縮の形で作る（`hdiutil`。macOS のランナー）。lzma（ULMO）か lzfse（ULFO）かは、大きさとマウントの時間を測って決める。
- 中にスクリプトや、利用者の操作なしに動く物を入れない。

### 5.2 未署名の警告の越え方（**未確認**。#241）

- 隔離の印（quarantine）は、ブラウザで取った dmg に付き、dmg から写した `ikari.app` にも引き継がれると推測する（**未確認**）。
- 入力メソッドは OS が起動するので、利用者が「開く」操作で許可する場面が無いかもしれない（**未確認**。#241 の中心の論点）。macOS 15 以降は、Control キーを押して開く道が無くなり、「システム設定 → プライバシーとセキュリティ」から許可すると Apple が案内している。27 での入力メソッドの場合の動きは**未確認**。
- #241 で確かめて README と `SECURITY.md` に書くこと: (a) dmg を開く手順、(b) `~/Library/Input Methods` の ad-hoc の `ikari.app` が入力ソースに出て、エンジン・設定アプリ・LM が動くこと、(c) 隔離の印が付いたままで動くか、付いていたらどう許可するか。隔離の印を消すコマンドや Gatekeeper を切る方法は書かない（ADR 0069 §3 の 5）。

### 5.3 比べたもの（記録）

| 軸 | **dmg（採用）** | zip（今の形） | pkg（署名なし、`/Library/Input Methods`） | Homebrew cask（後。#479） |
| --- | --- | --- | --- | --- |
| 未署名の警告 | 隔離の印が引き継がれる見込み（**未確認**） | 展開した物に隔離の印が付く（Safari などで取った場合） | pkg が止められ、システム設定から開く。入れた物には隔離の印が付かない見込み（推測） | 取得で隔離の印が付くと推測。公式の cask は署名・公証の無い物を受けない方向と聞く（**未確認**） |
| 置き場所 | 利用者が `~/Library/Input Methods` へ動かす（ショートカットと案内で迷いを減らす） | 利用者が手で動かす。案内は README だけ | `/Library/Input Methods` に固定（管理者のパスワード） | cask の書き方による（**未確認**） |
| 同梱で大きくなる影響 | 約 360〜410MB（圧縮の方式による。見積もり。§1.1） | 約 410MB（gzip 相当） | 約 360〜410MB | 取る物は Release の dmg |
| 更新 | 古い束を消して置き直す。開いているアプリは変換できなくなることがある（ADR 0070 §4 の 4） | 同じ | 開き直すと上書き | `brew upgrade` |
| アンインストール | 束を消してログアウト | 同じ | 標準の手段が無い（手順で案内） | `brew uninstall` |
| 証拠との相性 | 1 ファイル。SHA256SUMS、証明、作者の鍵の署名にそのまま載る | 同じ | 同じ | cask に sha256。Release の物を指す |
| 採らない理由 | — | 置き場所と手順の案内を配る物の中に持てない | 管理者の権限でスクリプトが走りうる点の懸念（ユーザー決定。スクリプトなしでも、入れる処理は root で動く） | 署名が付くまで後（#479） |

### 5.4 ADR 0069 §6 の 3 との関係（2026-11-22 ユーザー決定で整理）

- ADR 0069 §6 の 3 は、`~/Library/Input Methods` は利用者が書けるので同じ利用者のプログラムが束を差し替えられる（R1 を満たさない）とし、`/Library/Input Methods` を #241 で試して、動けば既定にする、としていた。
- **決定: dmg の案内は `~/Library/Input Methods` だけにし、`/Library/Input Methods` は並べない**（§12 の Q1）。これで 0069 §6 の 3 の「動けば既定にする」は取りやめ、「動かなければ今の場所のまま、この限界を `SECURITY.md` に書く」の側を採る。#241 では `/Library/Input Methods` を試す必要がなくなる（`~/Library/Input Methods` での確認だけ）。
- 受け入れる限界: 同じ利用者の別のプログラムが `ikari.app` を差し替えられる。差し替えられた入力メソッドは、打鍵の読み取りの許可なしで打鍵を受ける道になりうる（0069 §6 の 3 の P2 の論点）。同梱の場所のデータ（§3.1）も同じく書き換えられるので、検査（P5）は省かない。
- 実施事項: (1) `SECURITY.md` に、この限界（どこに置かれ、だれが書けるか、何が起きうるか）を書く。(2) ADR 0069 §6 の 3 の末尾に、この ADR の決定で改めた旨の追記を足す（本文は書き換えない。ほかの ADR と同じ流儀）。(3) `security-design.md` の該当の行を合わせる。どれも実装の PR（§11 のレーン 7）で行う。
- Homebrew は #479 で、Release の dmg を指す自前の tap から始める案を調べる（今は決めない）。

## 6. 決定 4: Windows のインストーラへの同梱

- 同梱の場所は `C:\Program Files\ikari-ime\data\`（§3.1）。`%LOCALAPPDATA%` には置かない（ADR 0067 §3.3。インストーラは管理者で動き、利用者の場所に触れない）。
- ADR 0067 §3.1 の配置の図に `data\dict\`、`data\models\`、`data\NOTICE.txt`、`data\licenses\` を足す（実装の PR で直す）。
- `[Files]` に足す項目は、ほかと同じく `Check: AppDirReady`（置き場所の検査とエンジンの停止を先に行う）。辞書とモデルは「版ごとのフォルダ」にしない（使用中でもエンジンを止めてから置くので上書きできる。TIP の DLL と違い、ほかのプロセスに読み込まれない）。
- LM の子は AppContainer の中で動く（ADR 0072 段 2）。Program Files は既定で `ALL APPLICATION PACKAGES` に読み取りを許している（ADR 0067 §3.1 の事実）ので、モデルを読める見込み。ただし、モデルのパスをエンジンが決めて渡す今の形（ADR 0072）で、同梱の場所から読めることは VM で確かめる（**未確認**）。
- 圧縮は Inno Setup の lzma2。setup.exe は約 360MB の見積もり（xz -6 相当。§1.1。実行物を除く）。展開の時間は測っていない。
- 削除は今の順のまま（`data\` もインストール先の一部として消える）。利用者のデータの場所に取り込んだ辞書やモデルは残る（ADR 0067 §3.3）。

## 7. 決定 5: 通常版とオフライン版の分け方

### 7.1 コンパイル時: cargo の feature `online`

- 通信の部品を起こす・その結果を読むコードを持つ crate（`shell-core` の取得の核、`shell-ffi` の `fetch_launcher`、`windows/settings` の `fetch_os`）に、feature **`online`** を置く。**既定では付けない**（足すと機能が増える形。cargo の feature は足し算にする）。通常版のビルドは `--features online` を明示する（`build.sh`、`release.yml`、`xtask`）。付け忘れはオフライン側（通信が少ない側）に倒れる。
- `ikari-data`（`fetch/` の crate）は通常版のパッケージでだけビルドし、オフライン版ではビルドもしない。
- `engine`、`lm`、TIP、候補窓、殻は変えない（もともと通信の部品に依らない。`net_crates` の検査が見ている）。

### 7.2 Rust が返す「出してよい機能」

- `shell-core` に `edition_capabilities() -> Capabilities { edition: Edition, url_model_fetch: bool, version_check: bool, file_import: bool }` を置く（契約）。`Edition` は `Normal` と `Offline`。値は `cfg!(feature = "online")` と、檻の段（自由指定は段 2 のある OS だけ。ADR 0072 Q1）と、`ikari-data` が同梱の場所にあるか、から Rust が決める。
- Swift と C# は、この値でボタンを出す・出さないを決めるだけ。オフライン版では「URL から取る」「新しい版を確認」を出さない。「ファイルから取り込む」（#246）は両方の版で出す。「このソフトについて」に版の名前（オフライン版）を出す。
- ビルドの日付からの古さの知らせ（ADR 0070 §3 の 2。通信なし）は、両方の版で出す。

### 7.3 パッケージ作成時の指定

- macOS: `build.sh --edition normal|offline`（既定は `normal`）。`offline` のときは `Contents/Helpers/ikari-data` を作らず置かない。束の中の `network.*` の entitlement の検査を「`ikari-data` だけ」から「どこにも無い」に替える。成果物の名前に `-offline` を付ける。
- Windows: `ISCC /DEdition=offline`。`ikari-data.exe` の項目と、そのファイアウォールの規則を入れない。成果物の名前に `-offline` を付ける。
- どちらも、同梱のデータ（§3）は両方の版で同じ物を入れる。

### 7.4 CI の検査（オフライン版に通信の部品が無いこと）

| 検査 | いつ | 中身 |
| --- | --- | --- |
| 依存 | PR ごと（`scripts/check.sh` の `xtask check`） | 配る crate を `online` なしで `cargo tree` し、`NET_CRATES` と `ikari-data` の crate が 1 つも出ないこと（今の `net_crates` に版の引数を足す）。`online` ありでも、通信の crate に依るのは `ikari-data` だけ（今の検査） |
| 配る物の中身 | リリースの workflow（両方の版を作るたび） | オフライン版の束・stage に `ikari-data` の実行ファイルが無いこと。macOS は束の中のどこにも `network.*` の entitlement が無いこと。Windows はインストーラのスクリプトに `ikari-data` の項目が無いこと |
| 実行ファイルの文字列 | リリースの workflow（`cargo xtask scan-product`） | オフライン版の実行ファイルに、取得の印（版の確認の固定の URL、`ikari-data` の起動の引数の語など。印の語は実装で決める）が無いこと |
| 実機 | 版を出す前（§10） | ネットワークを切った VM で動くこと、通常版の送信の記録（security-design §5.2） |

- オフライン版の配る物の検査は、PR ごとには macOS の束を作らない（時間がかかる）。代わりに、`build.sh` と `ikari.iss` の版の指定の分岐を、`xtask` の静的な検査（スクリプトの文字列）で PR ごとに見る。

### 7.5 版の確認と、自由指定のモデルの取得

- 通常版: 今の決定のまま（ボタンを押したときだけ。ADR 0070 §3 の 3、ADR 0076 §6・§7、ADR 0060）。同梱のため、最初の取得は無くなる（プリセットのモデルの取得は、同梱の物を消した人のための道として残すかを実装で決める。残しても、ボタンを押したときだけ）。
- オフライン版: どちらも無い。モデルと辞書の入れ替えは、ファイルからの取り込み（#246）だけ。

## 8. 決定 6: 更新の扱い

1. **辞書とモデルの更新は、アプリの版に乗せる。** 版を出すたびに辞書を作り直す（§9 の 1）。辞書の元データの版を上げるのは、`xtask/src/data/fetch.rs` の固定の値を変える PR で行う（評価で 1 位が悪化しないことを確かめる。CLAUDE.md）。モデルは zenz-v3.2-small のまま（替えるのは ADR とユーザーの判断。CLAUDE.md）。
2. **辞書だけの束を作者が配ることは、0.x ではしない。** 配ると `origin = release` の束になり、データの鍵の署名が要る（ADR 0069 §5）。アプリの版に乗せれば、真正性は Release の 3 つの証拠で足りる。
3. ADR 0070 §6 の 2（同梱した他社の部品は版を出すたびに上げる）に、辞書の元データとモデルを足す。ただし辞書の元データは「上げるかを評価で決める」（上げると候補が変わるため）。上げたかどうかと元データの版を Release のノートに書く。
4. 取り込み（#246）との関係: 取り込んだ物は利用者のデータの場所に入り、同梱の物より先に使われる（§3.2）。アプリを更新して同梱の辞書が新しくなっても、取り込んだ辞書を自動では消さない。設定画面で古いことを知らせる（§3.2）。
5. オフライン版の更新は、新しいパッケージの入手し直し（ユーザー決定）。隔離された環境の運用者が、Release の物と 3 つの証拠を持ち込む。
6. セキュリティの修正の扱い（ADR 0070 §4）は変えない。辞書とモデルの更新は「ふつうの更新」に数える。

## 9. 決定 7: 配る辞書を GitHub のランナーで作る

1. リリースの workflow に `build-dict` の job（ubuntu）を足す。元データは `fetch.rs` の sha256 で照らして取り（今の `cargo xtask data fetch`）、`dict-builder` で作り、`BUILD_INFO` ごと成果物にする。macOS と Windows の job は、この 1 つの成果物を受けて同梱する（両 OS で同じバイト列）。
2. モデルは、配布元の URL から取り、`ZENZ_SMALL_SHA256` で照らす（ikari 側で変換しない。ADR 0060 更新）。
3. キャッシュは読まない（ADR 0069 §4.1）。元データの取得先（SudachiDict、chiVe、ABR、Hugging Face、外来語の一覧の Release）が落ちていると版を出せない。中身は sha256 で守られるが、取れるかどうかは外に依る（受け入れる。代わりの置き場所は作らない）。
4. 辞書のビルドが同じ入力で同じバイト列になるか（再現性）は確かめていない（**未確認**。両 OS で同じ物を配るのは 1 の形で足り、再現性は要件にしない）。
5. job の時間とディスクは測っていない（元データは合わせて約 2.6GB。`BUILD_INFO` の入力の大きさから）。ランナーに収まらなければ、job を分けるか元データの扱いを見直す（測って決める）。

## 10. 確かめ方

| # | 確かめること | どこで |
| --- | --- | --- |
| C1 | macOS の XPC の束の中の `Resources/data/` を、Sandbox の中のエンジンが読み、mmap できる | 手元の Mac（スパイク） |
| C2 | 750MB を封印した ad-hoc の束の署名の時間、最初の起動の時間 | 手元の Mac とランナー |
| C3 | #241: dmg の開き方、`~/Library/Input Methods` で動くこと、隔離の印の扱い、ショートカットの形（§5.1・§5.2） | macOS の別の機械か VM（ユーザーの実機の入力ソースは変えない） |
| C4 | Windows: `Program Files\ikari-ime\data\` のモデルを、AppContainer の LM の子が読める | Windows の VM |
| C5 | **オフライン版を、ネットワークを切った VM に入れて**、辞書の変換と LM が動く。設定画面に取得と版の確認が出ない。ファイルからの取り込み（#246）でモデルを入れ替えられる | Windows の VM、macOS の VM か別の機械 |
| C6 | 通常版も、ネットワークを切ったまま同じに動く（同梱のため）。「新しい版を確認」は失敗の語を出して終わる | 同上 |
| C7 | 通常版の送信の記録で、ボタンを押したとき以外に送信が無い（security-design §5.2） | Windows の VM |
| C8 | 利用者のデータの場所の壊れた辞書・形式の古い辞書で、同梱の辞書に落ちて起動する（§3.2） | 単体テスト（共通のシナリオ）と VM |

- smoke を手元の Mac で流すときは、ユーザーの了解を取る（CLAUDE.md）。

## 11. 実装の分け方（1 タスク 1 ブランチ。issue はリードが切る。#246 と並べてよい）

| 順 | レーン | 中身 | 持つファイル |
| --- | --- | --- | --- |
| 0 | 契約（リード） | 同梱の場所を決める関数のシグネチャ、場所の候補の列の型、`Capabilities` と `Edition`、各 crate の `online` の宣言だけ（中身はスタブ） | `engine/src/` の新しいモジュール 1 つ、`shell-core` の 1 ファイル、該当の `Cargo.toml` |
| 1 | 解決（Rust） | §3.2 の選び方、落ちたときの 2 番目、状態の返し方、共通のシナリオ | `engine/src/lib.rs`・`model_select.rs`、`stack/src/lib.rs`、`shell-core/scenarios/` |
| 2 | 版の分け方（Rust） | `online` で取得の核を囲う、`Capabilities` の中身、`net_crates` の版の引数 | `shell-core`、`shell-ffi`、`windows/settings`、`xtask/src/check/supply.rs` |
| 3 | 画面（Swift、C#） | `Capabilities` でボタンを出す・出さない、版の名前、同梱か入れた辞書かの表示 | `macos/Sources/` の設定画面、`windows/settings` の画面 |
| 4 | macOS のパッケージ | `build.sh --edition`、同梱の場所への配置と封印の検査、dmg の作成（ショートカットと案内は C3 のあと） | `macos/scripts/` |
| 5 | Windows のインストーラ | `data\` の項目、`/DEdition`、`THIRD_PARTY_NOTICES.md`、検査 | `windows/installer/`、`xtask/src/check/installer.rs` |
| 6 | リリースの workflow | `build-dict` の job、モデルの取得、4 つの成果物（2 OS × 2 版）、配る物の検査 | `.github/workflows/release.yml`、`xtask/src/check/product.rs` |
| 7 | 表示と文書 | `THIRD_PARTY_NOTICES.md`（`system.dict` を CC BY-SA 4.0 として示す）、`NOTICE.txt`・`licenses/` の元、`SECURITY.md` の置き場所の限界（§5.4）、ADR 0069 §6 の 3 への追記、`fetch.rs`・`fetch-data.sh` のコメント（CC BY-SA 4.0 に）、README、SECURITY.md、publish-checklist §4、spec NFR-17、security-design（オフライン版の行）、ADR 0060・0067・0071・0072・0076 の備考 | 文書だけ |

- レーン 4・5・6 は、同梱する物の一覧を 1 か所（`xtask` の表）から読む（OS ごと・版ごとに一覧を重ねて持たない）。
- 1 と 2 は並べられる（持つファイルが重ならない）。3 は 0 のあと。4・5 は 1 のあと。6 は 4・5 のあと。7 は最後（実装が入るまで「配らない」の記述は今の事実として正しい。ADR 0060 更新の「事実と未確認」）。
- 先にスパイク: C1（macOS の読み）と C2（署名の時間）。捨てるコード。

## 12. ユーザーの決定（2026-11-22。§5 の dmg と合わせて）

| # | 問い | 決定 | 反映した節 |
| --- | --- | --- | --- |
| Q1 | dmg の案内に `/Library/Input Methods` も並べるか | 並べない。`~/Library/Input Methods` だけを案内し、利用者が束を差し替えられる限界を `SECURITY.md` に書く。ADR 0069 §6 の 3 の「動けば既定にする」は取りやめる | §5.4、§11 のレーン 7 |
| Q2 | 外来語の一覧を含む `system.dict` の表示 | 含めて同梱し、`system.dict` を CC BY-SA 4.0（と各表示）として示す（調査 §3.2 の案 A） | §4 の 5、§11 のレーン 7 |
| Q3 | `vectors.bin`（337MB）を同梱するか | 同梱する。話題の手がかりの主力で効果が大きい（ADR 0003 の実験で ext 30.9% → 58.6%） | §1.1 |

### （記録）決定の前の選択肢

- Q1: (a) `~/Library/Input Methods` だけ（推奨）、(b) `/Library/Input Methods` の別名も並べる。
- Q2: (a) 含めて CC BY-SA 4.0 を示す（推奨）、(b) 外来語の一覧なしで作る、(c) 含めて示さない。
- Q3: (a) 先に評価で無い場合と比べる（推奨）、(b) 同梱する、(c) 外す。いったん (a) で決めたが、ADR 0003 の実験の数字があるので (b) に改めた（同日）。

## 13. 検討した案（採らなかったもの）

- **最初の起動で同梱の物を利用者のデータの場所に写す**: 解決の場所が 1 つで済む。利用者ごとに 750MB を複製し、Windows ではインストーラが利用者の場所に触れない決まり（ADR 0067 §3.3）のため、エンジンが写すことになり、最初の起動が遅くなる。採らない。
- **同梱の物を利用者のデータの場所より先に使う**: アプリの更新で必ず新しい辞書になる。利用者が取り込んだ物が黙って効かなくなり、取り込み（#246）の意味が無くなる。採らない（§3.2 の「古いことを知らせる」で受ける）。
- **実行時の設定で通信を切る（1 つの配る物で済ませる）**: 作るのは楽。オフライン版は「通信のロジックを持たない」というユーザーの決定に合わず、検査で「持たない」を示せない。採らない。
- **feature `online` を既定で付ける（オフライン版は `--no-default-features`）**: ふだんの開発のビルドが今と同じになる。付け忘れたときに通信を持つ側に倒れ、ほかの既定の feature も一緒に外れる。採らない。
- **辞書を作者の手元で作って Release に上げる**: 速い。配る物を GitHub のランナーで作る決まり（ADR 0069 §4）に反する。採らない。
- **辞書を別のデータの Release（`dict-*`）にして、アプリの Release から参照する**: アプリと辞書の版を分けられる。利用者が束として取り込める物を作者が配ることになり、データの鍵の署名（ADR 0069 §5）が先に要る。0.x では採らない。

## 14. 結果

- 得るもの: 取得の手順なしで、入れればすぐ LM つきで変換できる。ネットワークを隔離した環境向けに、通信の部品を持たないことを検査で示せる版を出せる。壊れた辞書でエンジンが起動しない事態が、同梱の辞書で救われる。
- 失うもの: 配る物が約 360MB（xz 相当。実行物を除く）になる。版を出すたびに辞書のビルドが workflow に乗り、元データの取得先に依る。リリースの成果物が 2 OS × 2 版の 4 つになる。macOS は `~/Library/Input Methods` に置くため、ADR 0069 §6 の 3 の限界（R1）が残る。
- 改める文書（実装の PR で直す。今は直さない）: ADR 0058 §1.3（起動の失敗）、0060（更新の実装の参照）、0067 §3.1（配置の図）、0069 §6 の 3（`/Library` を既定にする案の取りやめ。§5.4）、0070 §6 の 2（辞書とモデル）、0071 §3・0076 §2・§7（取得の範囲）、0072（配る条件）、`security-design.md`、spec NFR-17、`publish-checklist.md` §4、`SECURITY.md`、README、`windows/installer/README.md`、`release.yml` の冒頭の「未完了」。

## 15. 推測と未確認（まとめ）

- 未確認: §3.4 の XPC の束の読み（C1）、署名と起動の時間（C2）、#241 の 3 点と dmg のショートカットの形（C3）、dmg の圧縮の方式ごとの大きさとマウントの時間、AppContainer からの読み（C4）、Homebrew の公式の cask の条件（#479）、辞書のビルドの再現性、workflow の時間とディスク、実行物を含めた配る物の大きさ。
- 推測: dmg から写した物に隔離の印が引き継がれる、隔離の印のある入力メソッドを OS がどう扱うか、`vectors.bin` が縮まない理由。

## 参考

- [ADR 0058](0058-offline-resource-install.md)、[0060](0060-user-selectable-lm-model.md)（更新 2026-11-19）、[0066](0066-security-premises.md)、[0067](0067-windows-install-layout-and-launch-chain.md)、[0069](0069-distribution-authenticity.md)、[0070](0070-update-and-vulnerability-response.md)、[0072](0072-lm-cage-levels.md)、[0076](0076-ikari-data-contract.md)、[0077](0077-macos-lm-cage.md)、[0079](0079-macos-ikari-data-sandbox.md)
- [ライセンスと再配布の調査（#242）](../research/license-redistribution-2026-11.md)、[security-design.md](../security-design.md)、[publish-checklist.md](../publish-checklist.md) §4
- #490、#242、#246、#241、#99、#479、#162
