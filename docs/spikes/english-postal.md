# スパイク: 英語のモードで郵便番号 → 住所、`te` + 数字、`u` + 16進（issue #27、#28）

捨てるコード。ADR 0032 §9 と §11 の 6 の前に、実機で確かめる。ブランチ `spike/english-postal`（マージしない）。

## 1. 何を確かめるか
英語のモード（打鍵をそのままアプリに渡す）で、語の頭の次の入力をためて、Tab か ↓ で候補ウィンドウを出せるか。

**方針（2026-10-28 ユーザー決定）**: 最後の桁を打った瞬間に、候補を自動で出す（Tab / ↓ は要らない）。最後の桁より前は、候補も下線も出さない。候補を出している間も下線は付けない。選ぶのは ↑ / ↓ だけで、Space・Tab・数字キーでは選ばない。数字だけの 7 桁は対象外（自然な英語の邪魔にならないように）。

| 入力（語の頭で） | ためる条件 | 最後の桁で出る候補（0 番が「そのままの入力」） |
|---|---|---|
| `te` + 数字 7 桁 | `t`、`te`、`te` + 数字 | `te1000001`、`〒100-0001`、`〒1000001`、住所（非同期で後から足される） |
| `u` + 16 進（`u+` も可） | `u`、`u+`、16 進 | 4 桁で `u+3042`、`あ`。5 桁、6 桁と増えるたびに更新 |

- 形から外れたキー（Space、Tab、ほかの文字、桁あふれ、数字キーでの選択）が来たら、ためた文字をそのまま確定して、そのキーをアプリに渡す（打ち直し）。
- 候補を出している間: ↓ で次、↑ で前、Enter で選んでいる候補を確定、Esc で「そのままの入力」を確定して閉じる（Esc は消費）、BackSpace で 1 文字戻る（`u+` の 4 桁を割ると候補が閉じる）。
- ダミーの表は公開の郵便番号 5 件だけ: `1000001`（千代田）、`1000004`（大手町）、`1500002`（渋谷）、`5300001`（梅田）、`6008216`（京都駅）。本物の住所の表ではない。
- 非同期の問い合わせは `DispatchQueue.main.asyncAfter` のダミー（既定 20 ms）。

確かめたいこと（ADR 0032 §9 の S1）:
1. 見た目と打ち直しが崩れないか（アプリごと）。
2. 候補ウィンドウの位置がアプリごとに正しいか（ログの `panel` の取り方と目視）。
3. 打鍵の取りこぼしがないか（速く打っても、ためた文字と確定した文字が合う）。
4. 非同期の返事を、開いている候補ウィンドウに出せるか。
5. `te`、`u` を語頭でためる遅れと見た目（`the`、`to`、`unit`、`use` を普通に打てるか）。

## 2. 入れかた（リードが行う。スパイクのエージェントは入れていない）
```sh
git switch spike/english-postal
macos/scripts/install.sh --signing team --build      # 入れ替えたら、開いているアプリを再起動する
# 検証用のフラグ（既定オフ。殻のプロセスの設定。ユーザーデータのファイルは触らない）
defaults write io.github.ikari-zen.inputmethod.ikari ikariSpikePostalEnglish -bool YES
```
- フラグを反映するには、入力ソースを一度切り替えるか、ikari を再起動する（`defaults` は打鍵ごとに読むので、通常は即時）。
- 終わったら必ず戻す。入れ替えた殻も main のビルドに戻す。
```sh
defaults delete io.github.ikari-zen.inputmethod.ikari ikariSpikePostalEnglish
defaults delete io.github.ikari-zen.inputmethod.ikari ikariSpikePostalDelayMs
git switch main && macos/scripts/install.sh --signing team --build
```
- `data/`、`~/Library/Application Support/ikari-ime/` は触らない。smoke は流さない。

## 3. ログの取りかた
別のターミナルで流したままにする。
```sh
log stream --level info --predicate 'subsystem == "io.github.ikari-zen.inputmethod.ikari" AND eventMessage BEGINSWITH "spike-postal"'
# 後から見るとき
log show --last 10m --info --predicate 'subsystem == "io.github.ikari-zen.inputmethod.ikari" AND eventMessage BEGINSWITH "spike-postal"'
```
遅れの値を変える（ミリ秒。既定 20）。
```sh
defaults write io.github.ikari-zen.inputmethod.ikari ikariSpikePostalDelayMs -int 300   # 間に合わない場合
defaults write io.github.ikari-zen.inputmethod.ikari ikariSpikePostalDelayMs -int 0
```
ログには文字を書かない（NFR-19）。secure（パスワード欄）では状態に入らず、何も書かない（NFR-18）。

### 1 打鍵の行
`spike-postal app=<bundle id> ev=<キーの種類> from=<前の状態> to=<今の状態> res=<結果> kind=<digits|te|unicode> len=<ためた長さ> keys_in=<入ってから受けたキー数> gap_ms=<前の打鍵からの間隔> pass=<今のキーをアプリに渡したか> marked_len=<アプリが持つ変換中の範囲の長さ> expect_marked=<殻が渡した長さ> why=<確定を強いた理由>`

- `ev`: digit、t、e、u、plus、hex、tab、backtab、down、up、bs、esc、space、enter、other_char、other、flush。
- `res`: typing（ためた）、choices（候補ウィンドウを出した）、move（選択が動いた）、commit（候補を確定した）、retype（打ち直し）、cancel（BackSpace で空になった）。
- 問い合わせの行: `spike-postal app=... ev=lookup res=lookup_arrived|lookup_stale wait_ms=<実際の待ち> delay_ms=<設定> rows=<住所の件数>`。

### 何を読むか
- 取りこぼし: 打った数（キーを数える）と、`res=typing` の行の数が合うか。`keys_in` が 1 ずつ増えているか。`gap_ms` が極端に長い行（アプリが重い）と、行の欠け（殻に届いていない）を見分ける。確定のあと、アプリの画面の文字数が `len` と合うか（欠けや重複がないか）。
- 変換中の範囲の食い違い: `marked_len` が `expect_marked` と違う行が続くなら、そのアプリは `setMarkedText` を受けていない（見た目が崩れる）。`marked_len=na` はアプリが範囲を返さない。ただし `typing` の直後の 1 行だけでなく、複数打鍵で見る。
- 候補ウィンドウの位置: `ev=panel anchor=client|accessibility|lastKnown|mouseScreen` の行が位置の取り方。目視で、文字の近くか、画面の端に飛んでいないかを書く。
- 非同期: `lookup_arrived` が `wait_ms` で返ったあと、2 番目以降の住所の行が増えたか（目視）。`delay_ms=300` で `lookup_stale` になるのは、返事の前に Esc や確定をしたとき。
- 遅れ: `t` / `u` を語頭で打ったとき、`the`、`to`、`unit`、`use` を普通の速さで打って、`retype` 後に文字が正しく並ぶか。目に見える遅れがあるか。

## 4. 試す操作
各アプリで、英語のモードに切り替えて、以下を順にやる。1 つの行は「操作 → 期待」。

1. 語頭で `te1000001` と打つ → 6 桁目までは下線なしで見えるだけ。7 桁目で候補が自動で出る（下線なし）。Enter → `te1000001` が確定。
2. `te1000001` → ↓ → ↓ → Enter → 住所（20 ms で足される）。↑ で戻れる。
3. `te1000001` の後に Space または Tab → `te1000001` が確定して、Space / Tab が入る（選ばない）。数字キー 1〜9 も選ばない（桁あふれで打ち直し）。
4. `te100000` で止める → 候補は出ない。
5. `te1500002` → 〒つきの 2 つ + 渋谷区。Esc → `te1500002` が残って閉じる。
6. `the`、`to`、`tea`、`test` が普通に入る。
7. `u+3042` → 4 桁目で `あ` が自動で出る。`u+30420` と続けると 5 桁で更新、Enter で確定。`use`、`unit`、`ubuntu` も普通に入る。
8. 文の途中（`abc 1000001`）→ 語頭として働く。`abcte1000001`（語の途中）→ ためない。
9. BackSpace: `te10` で BackSpace を 4 回 → 空になる（最後の BackSpace は消費）。候補が出た後の BackSpace は 1 文字戻る。
10. 速打ち: `te1000001` を速く打つ（ログの `gap_ms` が 30 ms 前後）。欠けがないか。
11. 候補を出している間に、別アプリへ切り替える、クリックする → ためた文字が確定される（ログの `ev=flush`）。
12. 遅れを 300 にして `te1000001` → 先に「そのまま」だけが出て、あとで住所が足される。出る前に Enter したら `lookup_stale`。

## 5. 試すアプリ
TextEdit（リッチテキスト／プレーンテキスト）、VS Code（エディタ。内蔵ターミナルは除外アプリの扱いを見る）、Safari か Chrome（アドレスバー、フォームの入力欄、`contenteditable`）、Slack、メモ、Numbers か Excel のセル、Terminal（動かないのが正しければ、そのように書く）、パスワード欄（何も起きず、ログも出ない）。

## 6. 結果の欄（ユーザーが書く）
○ 問題なし、△ 気になる、× 崩れる。気になる点は 1 行で（文字はログに出ないので、見た目を書く）。

| アプリ | 見た目・打ち直し | 候補ウィンドウの位置 | 取りこぼし | 非同期（20 ms / 300 ms） | `the` `to` `use` の遅れ | メモ |
|---|---|---|---|---|---|---|
| TextEdit（リッチ） | | | | | | |
| TextEdit（プレーン） | | | | | | |
| VS Code | | | | | | |
| Safari / Chrome（フォーム） | | | | | | |
| Safari / Chrome（アドレスバー） | | | | | | |
| Slack | | | | | | |
| メモ | | | | | | |
| 表計算のセル | | | | | | |
| Terminal | | | | | | |
| パスワード欄 | | | | | | |

総括（ADR 0032 §9 の決定に使う）:
- 英語のモードで郵便番号を出す／出さない:
- 候補を出すキー（Tab・↓ のまま／変える）:
- `te` をためる遅れは許せるか（`te` を外す案も含む）:
- `u` + 16進をためる遅れは許せるか:
- 問い合わせ方（(b) 非同期 1 回でよいか）:
- 除外するアプリ:
