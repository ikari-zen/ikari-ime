# ADR 0046: 左文脈が足りないぶんを同じアプリの確定履歴から足し、アプリを行き来しても履歴を保つ（ほかのアプリからの借用は廃止）

- 状態: 採用（2026-11-03〜04 にユーザーが §12 を決定。ほかのアプリからは借りない）。値（`carry_fill_chars`、区切り）は掃引のあとに確認する
- 日付: 2026-11-03（初版）、2026-11-04（借用の廃止に改訂）
- 対象: 新しい `core/src/context_carry.rs`（左文脈を組み立てる純関数と型）、`core/src/context.rs` と学習・変換の `left_borrowed` の分岐、`engine/src/session.rs`、`engine/src/context_limits.rs`、`engine/src/tests/context.rs`、`shell-core/src/controller/mod.rs`（アプリごとのセッションの ID を使い回す）、`shell-core/src/machine/`・`shell-core/src/settings/`・`shell-core/src/types.rs`・`shell-ffi/src/`（`share_context` と `context.shared_exclude` の削除）、proto 3 つ（§8.3）、設定アプリ（Swift。除外するアプリの節の削除）、`eval/src/set.rs`・`eval/src/scenario.rs`・`eval/src/args.rs`、`eval/scripts/`、`eval/sets/`、`eval/scenarios/`。LM（`lm/`）と学習の形式は変えない
- 前提: main = dda8fe3。ADR 0015（D1 休止と再開、D5 アプリをまたぐ「最近の確定」と X5、D6 除外、D7 LM の窓 40、D8・Q7 借りた文脈で学習しない）、ADR 0002 §0.4（zenz のプロンプト）
- タスク: #182（ユーザー決定 2026-11-03: 実機のログは確かめずに進める。2026-11-04: ほかのアプリからは借りない、今の借用も廃止する）
- 判断の置き場所: Rust の core（組み立ての純関数。eval からも同じ関数を呼ぶため）、engine（どの材料を渡すか）、shell-core（セッションの ID）。Swift は、廃止する設定の UI を消すだけで、判断を足さない

---

## 0. 現状の把握（#182 の調査と、コードを読んだ範囲）

- 左文脈の組み立ては `engine/src/session.rs` の `build_context_borrowing`。分かれ方は 3 つ。
  1. 周辺テキストが取れた（`available`）: 入力欄の左の末尾（最大 2000 文字）だけ。**確定履歴は足さない**。左が空でも足さない。
  2. 取れない・同じセッションの履歴あり: 履歴の表記を区切りなしで連結した末尾。
  3. 取れない・履歴なし: 直近 `BORROW_WINDOW`（2 分）以内のほかのセッションの確定（X5。`borrowed_left`。`left_borrowed = true`）。
- アプリをまたぐ仕組み（ADR 0015 D5・D6・Q7）は、engine の「最近の確定」（`RecentCommit`、`RECENT_COMMITS_TTL` 10 分、`RECENT_COMMITS_MAX_CHARS` 2000）、`BeginSession.share_context`（除外するアプリでは false）、殻の設定 `context.shared_exclude` と設定アプリの「除外するアプリ」の節、core の `Context.left_borrowed`（学習の文脈を空にする）でできている。`share_context` と `context.shared_exclude` は、**借用のためだけ**に使われている（engine では `push_recent` と `borrowed_left` の判定だけ）。
- セッションの ID は shell-core の `make_client` が振る。**アプリが替わるたびに新しい ID**（`next_session_id`）。A → B → A と戻ると A は新しい ID になり、A の古いセッションは engine の中で休止のまま 30 分後に消える。D1（休止と再開）が効くのは、同じアプリの中で ikari を切り替えたときだけ。
- LM は `left_text` の末尾 40 文字だけを見る（`zenz.context_chars`）。話題（chiVe）、文書内キャッシュ、連接（`LeftContextRanker`）は `left_text` 全体と `history` を見る。
- zenz v3 のプロンプトは `[EE03 プロフィール][EE04 話題] EE02 左文脈 [EE07 右文脈] EE00 読み EE01`（`lm/src/prompt.rs`）。左文脈は「同じ文書で直前にある文」として学習されている（ADR 0002 §0.4）。左文脈の中に「別の文書の境目」を表す印はない。改行が学習データにどれだけあるか、トークナイザがどう扱うかは**確かめていない**。
- 評価（ADR 0015 §0.6・§4。homophones-ext 1266 件）: 直前の文脈が届くかどうかがほとんどすべて（LM あり 610 → 1093）。**本当の文脈の前に無関係な文（約 150 字）があると害が大きい**（1093 → 950。LM なし 979 → 738）。害の大部分は話題（chiVe）から来る。
- 評価の scenario とセットには、`context`（そのまま `left_text`）と `borrowed`（フラグ）しかない。eval は engine に依存しないので、engine の組み立てを通らない。

## 1. 問題の定義

ユーザーがチャットで「品詞」の話を送ったあと、次のメッセージで「ひんしを」と打つと、LM は文脈なしで判断して「瀕死を」を 1 位にする。直前に自分で書いた文が、同じアプリにあるのに使われていない。

1. **入力欄の左が空・短いとき、同じアプリの確定履歴が使われない**。チャットは送ると入力欄が空になる。アプリが周辺テキストを返す（`available`）と、空でも「これが文脈のすべて」として扱い、履歴を捨てる。
2. **アプリを行き来すると、同じアプリの履歴が切れる**。セッションの ID がアプリの切り替えごとに振り直されるため。

ほかのアプリの文は、今の文書と文脈が同じだという保証がない（ユーザーの判断 2026-11-04）。この ADR は、文脈を**同じアプリの中だけ**で保つ形に絞り、今あるアプリをまたぐ借用は廃止する。

## 2. 決定の要約

| # | 決定 |
|---|---|
| E1 | **左が足りないぶんだけ、同じセッションの確定履歴の末尾を左の前に足す**。目標の長さ `carry_fill_chars`（候補は LM の窓と同じ 40。値は掃引で決める）に対して、`available` の左が短いときだけ、足りない文字数ぶんを足す。足した結果は LM の窓を超えないので、プロンプトは今より長くならない |
| E2 | **重なりの除去は「左の全体が履歴の末尾と一致するときだけ」**。左が履歴の末尾（接尾辞）なら、履歴のうち左より前の部分を足す。一致しないとき（確定後の編集、アプリの整形、左が古い、カーソルを動かした、入力欄にもとから文がある）は足さない（安全側） |
| E3 | **つなぎ目**: 足した履歴と、入力欄の左（または読み）のあいだに、**境目だと分かっているときだけ**区切りを入れる。種類（なし／「。」／改行）は掃引で決める。取れない入力欄の今の経路（履歴をそのまま左にする）には入れない |
| E4 | **アプリを行き来しても、同じアプリの履歴を保つ（A1）**。shell-core が Bundle ID ごとにセッションの ID を覚えて使い回す（最大 64 アプリ。古いものから忘れる）。engine の D1（休止と再開、30 分）がアプリの行き来でも効く |
| E5 | **ほかのアプリからは借りない。今の借用を廃止する**。X5（`borrowed_left`、`BORROW_WINDOW`）、「最近の確定」（`RecentCommit`、`RECENT_COMMITS_TTL`、`RECENT_COMMITS_MAX_CHARS`）、`BeginSession.share_context`、設定 `context.shared_exclude` と設定アプリの除外するアプリの節、core の `Context.left_borrowed` を消す（§5、§8.3）。ADR 0015 Q7（借りた文で学習しない）は、借りる経路がなくなるので不要になる |
| E6 | secure は今どおり何も入れない。同じセッションの履歴から足したものは、今の「取れない入力欄の履歴」と同じく自分の文脈として扱い、学習の文脈にも入る |
| E7 | **組み立ては core の純関数 1 つにする**（`compose_left`）。engine と eval が同じ関数を呼ぶ。eval のセットと scenario に `field`・`history` の列を足し、`borrowed` の列を外す |
| E8 | 契約の PR では `carry_fill_chars = 0`（E1 は効かない）を既定にし、掃引の PR で既定を替えてユーザーに確認する。E4・E5 は値に依らず入る |

## 3. E1・E2: 足りないぶんを足し、重なりを除く

### 3.1 案の比較

**足す量**

| 案 | 内容 | 良い点 | 問題 |
|---|---|---|---|
| **F1（採用）: 足りないぶんだけ** | 目標 `carry_fill_chars` − 左の文字数 だけ、履歴の末尾から足す | LM のプロンプトは今の最大（40 文字）を超えない（時間が増えない）。話題に入る履歴も最大 40 文字に収まり、ADR 0015 §4 の「前に 150 字」の害の形より小さい。左が十分に長ければ何も変わらない | 目標の値が要る（掃引） |
| F2: 履歴を全部（2000 文字まで）前に足す | 取れない入力欄の今の経路と同じにする | 実装が一番少ない | ADR 0015 の X1 と同じ形（本当の文脈の前に長い文）。話題に古い文がまるごと入り、測定で −143〜−241 の型の害 |
| F3: LM にだけ足す（話題・連接には足さない） | `Context` に LM 専用の左を別に持つ | 話題の害を入口で止められる | `Context` と LM の契約を増やす。LM なしでは何も得ない。害の大部分が話題だと分かっているが、F1 で話題に入るのは最大 40 文字で、まず測ってから考える（掃引で害が出たら F3 を測る） |

**重なりの除去**

| 案 | 内容 | 良い点 | 問題 |
|---|---|---|---|
| **O1（採用）: 左が履歴の接尾辞のときだけ** | `H.ends_with(L)` のときだけ、`H` の `L` より前を足す。一致しなければ足さない | 誤って二重に足すことも、順序の違う文をつなぐこともない。判定が一行で、境界が明快 | 確定後に少し直した、アプリが整形した、改行を打った、などで足さなくなる（取りこぼし）。取りこぼしは「今と同じ」なので害はない |
| O2: 最長の共通の接尾辞で切る | `L` と `H` の共通の接尾辞の長さ k を求め、`H` の残りを足す | 一部が一致すれば足せる | `L` に履歴にない文（もとからあった文、別の欄の文）が前にあるときも、その前に履歴をつないでしまう（順序が壊れる）。危ない側に倒れる |
| O3: 空白・改行を無視して比べる | 比べる前に両方から空白を除く | 改行を打った入力欄でも足せる | 正規化の規則が増える。取りこぼしの量が分からないうちに入れる理由がない。実際に取りこぼしが目立てば足す |
| O4: 履歴に「入力欄での位置」を持つ | 確定のときの周辺テキストの長さなどを覚えて合わせる | 精確 | 状態が増え、アプリごとの周辺テキストの癖（取れない、古い）にまた依存する |

### 3.2 アルゴリズム（擬似コード）

```
入力:
  field   = Unavailable | Available(L)   // L は入力欄の左（末尾 2000 文字に切ったもの）
  H       = 同じセッションの確定の表記の連結（2000 文字まで）
  fill    = carry_fill_chars（0 なら足さない）
  J       = carry_joint（None | Period「。」 | Newline「\n」）

compose_left:
  match field
    Unavailable:
      H が空でない          -> left = tail(H, 2000), 出どころ = History      // 今と同じ。区切りなし
      それ以外              -> left = "",            出どころ = Empty
    Available(L):
      L が空白だけ:
        H が空でない        -> left = tail(H, 2000) + joint(H), 出どころ = History  // 送信で空になった入力欄
        それ以外            -> left = L,             出どころ = Empty
      deficit = fill - len(L)（負なら 0）
      deficit == 0          -> left = L, 出どころ = Field                    // 十分に長い
      H.ends_with(L) でない -> left = L, 出どころ = Field                    // 一致しない。足さない
      P = H から末尾の L を除いた残り
      P が空                -> left = L                                      // 履歴はすべて見えている
      j = joint(P)
      deficit <= len(j)     -> left = L
      left = tail(P, deficit - len(j)) + j + L,  出どころ = FieldWithHistory

joint(X) = X の最後の文字が { 。 ． . ！ ？ ! ? 、 ， 」 』 ） ) 】 改行 空白 } なら "" それ以外は J
```

- `L` が空白だけの `available` は、取れない入力欄と同じに扱う（左には何もない）。違いは、履歴のあとに区切りを入れること（入力欄が空になった＝境目だと分かっている）。右文脈は今どおり `available` の `right` から取る。
- 「L が空白だけ」で H を全部（2000 文字まで）使うのは、取れない入力欄の今の経路に揃えるため。`fill` で切るかは掃引で C1 と比べる（§7.2）。
- 長さはすべて文字（`char`）で数える。切るのは今の `tail_chars` と同じく文字の単位。
- `history`（`Context.history`。文書内キャッシュが使う）は今どおり全部を渡す。変えるのは `left_text` だけ。

### 3.3 境界例（`fill = 40`、`J = 「。」`）

| # | field | L | H | 結果の left | 理由 |
|---|---|---|---|---|---|
| 1 | Available | 「」 | 名詞や動詞など、品詞の話をした | 名詞や動詞など、品詞の話をした。 | 送信で空になったチャット。区切りを足す |
| 2 | Available | 「」 | 名詞や動詞など、品詞の話をした。 | 名詞や動詞など、品詞の話をした。 | 最後が「。」なので区切りを足さない |
| 3 | Available | 次は | 品詞の話をした次は | 品詞の話をした。次は | 重なり（L が H の接尾辞）。足りない 38 文字の中に、P の全部と区切りが入る |
| 4 | Available | 次は | 品詞の話をした次の | 次は | 一致しない（確定後に直した）。足さない |
| 5 | Available | 次は | 品詞の話をした | 次は | L が H の接尾辞でない（左が古い、別の欄）。足さない |
| 6 | Available | メモ：品詞の話をした次は | 品詞の話をした次は | メモ：品詞の話をした次は | L が H より長い（入力欄にもとから文がある）。足さない |
| 7 | Available | （41 文字以上） | 何でも | L | 足りない文字がない |
| 8 | Available | 品詞の話をした次は | 品詞の話をした次は | 品詞の話をした次は | P が空（履歴はすべて見えている） |
| 9 | Available | 次は（38 文字の文の末尾） | …次は | L | deficit ≤ 区切りの長さ |
| 10 | Unavailable | ― | 品詞の話をした | 品詞の話をした | 今と同じ（境目か分からないので区切りなし） |
| 11 | Available | 「 」（空白） | 空 | 「 」 | 同じアプリの履歴がない（アプリを替えた直後など）。**ほかのアプリからは借りない**（§5.2） |
| 12 | Available | を | 品詞を | 品詞。を | 同じ欄の続きなのに区切りが入る誤り（P = 品詞）。§3.4 の注意。`J = None` なら 品詞を |
| 13 | Unavailable | ― | 空 | 「」 | 今は X5 でほかのアプリの文を借りていた。**廃止して文脈なし**（§5.2） |

### 3.4 注意（受け入れる取りこぼしと誤り）

- 例 12 のように、**同じ入力欄の中で、左が短い（入力欄の先頭から数文字）**のときも E2 の一致は成り立つ。このとき P と L のあいだは本当は境目ではない（同じ欄の続き）のに区切りが入る。区切りの有無を見分けるには「入力欄の先頭がどこか」が要るが、周辺テキストの `left` が入力欄の先頭から始まるかは、アプリによって違い、分からない。区切りの掃引（§7）で「なし」と「。」を比べ、この誤りの重さを測る。
- 1 文字だけの左（「を」など）が、偶然、別の欄の履歴の末尾と一致することがある。左に入る文字のほとんどは ikari の確定なので、同じアプリの中ではほぼ正しい。最低の一致の長さは設けない（設けると、送信直後の 1 文字目で効かなくなる）。
- 入力欄に改行や英字（ABC で打った文字）を打つと、一致しなくなり足さない。左に文字があるので、多くは十分な文脈がある。目立つようなら O3 を測る。

## 4. E3: つなぎ目の区切り

| 案 | 内容 | 良い点 | 問題 |
|---|---|---|---|
| J0: なし | 連結するだけ | 今の取れない入力欄の経路と同じ。同じ文の続き（例 12）を壊さない | 「品詞の話をした」+「ひんしを」が 1 文に見える。送信の境目が LM に見えない |
| **J1（初期の候補）: 「。」** | 足した文の最後が句読点などでなければ「。」を入れる | zenz の学習データ（文書の直前の文）で、文の境目はふつう「。」で表れる。公用文の表記の既定とも合う | 例 12 のような同じ文の続きでは、文を切ってしまう |
| J2: 改行 | `\n` を入れる | 「別のメッセージ」に近い | zenz のトークナイザと学習データでの改行の扱いを確かめていない。未知の文字なら害になりうる |
| J3: 区切りのトークン（EE 系の私用の文字） | 文書の境目を表す新しい印 | 意味が明快 | zenz は学習していない。プロンプトの形式を学習時から外す（ADR 0015 D7 の理由と同じ） |

**決定**: 区切りを入れるのは、境目だと分かっている所（足した履歴と入力欄の左のあいだ、空になった入力欄の前）だけ。種類は J0・J1・J2 を掃引で比べて決める（§7）。J2 は、掃引の前に zenz-v3.2 small の語彙に改行があるかを確かめ、なければ比べない。J3 は採らない。

## 5. E4・E5: アプリをまたぐ文脈

### 5.1 案の比較

| 案 | 内容 | 判断 |
|---|---|---|
| **A1（採用）: 同じアプリの ID を使い回す** | shell-core が Bundle ID → セッションの ID を覚える（最大 64。古いものから忘れる）。engine の休止と再開（30 分）がそのまま効く | **採用**。proto も engine の寿命の決定も変えない。アプリの中で別の入力欄に移っても同じ履歴が続く（今もアプリを替えなければ同じ）が、E2 の一致で、左に文字のある欄には足さない |
| X5 を保つ（今の借用） | 入力欄に何もないとき、直近 2 分のほかのアプリの確定を借りる | **廃止**（ユーザー決定 2026-11-04）。ほかのアプリの文は、今の文書と文脈が同じだという保証がない |
| A2: X5 の「何もない」に `available` で左が空を加える | 借用の取りこぼしを直す | **不採用**。借用そのものを廃止するので要らない |
| A3: 左が足りないとき、ほかのアプリの文も前に足す | 書きかけの文書にも効く | **不採用**（2026-11-03）。理由は X5 と同じで、害の形は X1 |
| A4: 借りる窓を延ばす・設定で変える | 5 分など | **不採用**（2026-11-03）。借用を廃止するので対象がない |

### 5.2 廃止で変わる動き

- **アプリを替えた直後の変換は、文脈なしになる**。今は、その入力欄に何もなく（周辺テキストが取れない、または取れても左がない）、そのアプリの履歴もないとき、直近 2 分のほかのアプリの確定を左文脈にしていた（X5）。廃止のあとは、その場合の左文脈は空になる（§3.3 の例 11・13）。
  - ADR 0015 §0.6 の測定では、関係のある文が届けば 610 → 1093、無関係なら 610 → 617（LM あり）。廃止で失うのは「ほかのアプリの文が今の文と関係がある」場合の得で、得ていた場面の割合は実機で測っていない（S1 は入れていない）。
  - A1 により、**一度使ったアプリに 30 分以内に戻ったとき**は、そのアプリの履歴が続く。今より文脈が残る場面もある（A → B → A の A）。
- 除外するアプリ（パスワード管理、ターミナル）を設定する意味がなくなる。そのアプリの確定は、もともと同じアプリの中でしか使われない（同じセッションの履歴は、除外の設定に関係なく今も使っている）。
- 学習: 借りた文脈がなくなるので、「借りた文脈は学習では文脈なし」（Q7）の分岐が要らなくなる。今まで借りていた変換は「文脈なし」になり、学習の記録の上でも文脈なしのまま（記録の意味は変わらない）。

## 6. プライバシー

- **ほかのアプリで書いた文は、別のアプリの変換に使わない**。文脈は、そのアプリの中の確定と、そのアプリが返す周辺テキストだけ。アプリをまたいで文を運ぶ仕組み（「最近の確定」）をなくすので、「除外するアプリ」の設定も要らなくなる。
- 文脈は今どおり端末の中（engine のメモリと手元の LM）だけで使い、外へ送らない（NFR-17）。保存しない（D8）。ログに中身を書かない。
- 変わること: アプリに戻ったとき、**そのアプリで** 30 分以内に書いた文が文脈に残る（A1）。今もアプリを替えなければ 30 分残っている。shell-core が Bundle ID と ID の対応を最大 64 件メモリに持つ（保存しない。engine には今どおり Bundle ID を保存させない）。
- secure は今どおり何も入れない。secure が変わったら履歴をつながない（今の `begin_session`）。

## 7. 評価の整備と掃引の計画

### 7.1 形式（eval）

- **セット**（`eval/src/set.rs`）と **scenario**（`eval/src/scenario.rs`）に、任意の列を 2 つ足す。
  - `field`: `none`（周辺テキストが取れない）／`available`（`context` の列を入力欄の左とする）。**列がない、または空なら今と同じ**（`context` をそのまま `left_text` にする）。
  - `history`: 同じセッションの確定の表記の連結（1 つの文字列）。
  - `field` があるときは、eval は `compose_left` で `left_text` を作る。
- **`borrowed` の列を外す**（`Context.left_borrowed` がなくなるため）。これを使う scenario `eval/scenarios/b28-borrowed.tsv` は消す（Q7 の不変条件の確認で、確かめる対象がなくなる）。
- 引数: `--carry-fill N`、`--carry-joint none|period|newline`（既定は契約の既定）。
- **派生セット**（`eval/scripts/make_carry_sets.py`。`make_long_context_set.py` と同じ作り方。homophones-ext から作る）

| 名前 | 作り方 | 何を測るか |
|---|---|---|
| C1 carry-empty | field=available, L=空, H=元の文脈 | 送信で空になった入力欄。ext-full（1093）に近づくか。区切りの害（元の文脈は文の途中なので「。」は不利。区切りの最悪の場合） |
| C2 carry-overlap | L=元の文脈, H=関係のある前の文 + 元の文脈 | 重なりの除去と、関係のある前の文の効き |
| C3 carry-unrelated | L=元の文脈, H=別のケースの文脈（無関係・約 150 字）+ 元の文脈 | **害**。同じアプリでも話題が変わる場（チャット）で、F1 の上限で足したとき |
| C4 carry-mismatch | L=元の文脈, H=別のケースの文脈（L で終わらない） | 不変条件。ext-full と 1 件も違わないこと |
| C5 carry-empty-unrelated | L=空, H=別のケースの文脈 | 同じアプリの無関係な前の文だけが左にある場合。文脈なし（610）より悪くならないか（借用ではなく、同じアプリの履歴の害） |

- 関係のある前の文とチャットの形（前のメッセージが完結した文、次のメッセージの頭で同音語）は homophones-ext から作れない。**小さな手書きのセット `eval/sets/carry-chat.tsv`**（50〜100 行。Claude の下書き、ユーザーが確かめる。ADR 0015 Q8 と同じ扱い）を足す。「名詞や動詞など、品詞の話をした」→「ひんしを」をここに入れる。
- **scenario** `eval/scenarios/c182-carryover.tsv`（LM あり・なしで流す）:
  1. available・L 空・H「名詞や動詞など、品詞の話をした」・ひんしを → 品詞を が 1 位（LM あり）。
  2. 同じで field=none → 品詞を が 1 位（今の経路の回帰）。
  3. available・L「次は」・H「名詞や動詞など、品詞の話をした。次は」・ひんしを → 品詞を が 1 位（重なり）。
  4. available・L 空・H 空・ひんしを → 文脈なしの 1 位（アプリを替えた直後。廃止で変わる動きの記録。期待は付けず結果だけを報告に書く）。
  - LM なしの期待は、実装の PR で流して決める（LM なしで 品詞を が 1 位になるかは測っていない）。
- **単体テスト**: core の `compose_left` に §3.3 の表をそのまま入れる。engine に、secure で何も入れないこと、ほかのセッションの確定が左に入らないこと（今の借用のテストを、借りないことのテストに置き換える）。shell-core に、A → B → A で A の ID が戻ること、65 個目で最も古い対応を忘れること。

### 7.2 掃引（LM あり・なし。値は計測で決める）

- 軸: `carry_fill_chars` ∈ {0, 10, 20, 40} × `carry_joint` ∈ {none, period, newline（語彙にあれば）}。LM ありは zenz-v3.2 small、締め切り 3000ms（時間切れを除くため。ADR 0015 §4.7）。LM なしは `--ranker full` の LM を切ったもの。
- セット: C1〜C5、carry-chat、scenario、全セット（`eval/scripts/snapshot.sh`）。
- 決め方（案。ユーザーに表を出して決める）:
  1. C4 と全セットの 1 位が、`fill = 0` と比べて悪化しない（必須）。
  2. C5 が文脈なし（ext-empty）より悪くならない（LM あり。必須）。
  3. そのうえで、C1・C2・carry-chat の直った数 − C3 の崩れた数 が最大の組を推奨にする。C3 の崩れが得を上回る `fill` は採らない。
  4. LM ありと LM なしで推奨が分かれたら、LM ありを優先し、LM なしの差を併記する。
- 時間: F1 ではプロンプトが 40 文字を超えないので、LM の応答時間は変わらない見込み。`cargo xtask perf` を 1 回だけ流して確かめる。
- 測れないもの: A1 の効く頻度と、借用の廃止で失う得の量（どちらも時間とアプリの行き来に依る。実機のログは見ないと決まっている）。

## 8. 契約（変更する関数、型、定数、ファイル）

### 8.1 core

新しい `core/src/context_carry.rs`（`lib.rs` から公開）:

```rust
pub enum FieldLeft<'a> { Unavailable, Available(&'a str) }
pub enum CarryJoint { None, Period, Newline }
pub struct CarryParams { pub fill_chars: usize, pub joint: CarryJoint }
pub struct LeftSources<'a> {
    pub field: FieldLeft<'a>,
    pub history: &'a str,   // 同じセッションの確定の連結
}
pub enum LeftOrigin { Empty, Field, FieldWithHistory, History }
pub struct ComposedLeft { pub text: String, pub origin: LeftOrigin }
pub fn compose_left(src: &LeftSources<'_>, p: &CarryParams, max_chars: usize) -> ComposedLeft;
```

- `max_chars` は engine の `MAX_CONTEXT_CHARS`（2000）。`LeftOrigin` はテストと、将来のログの出どころの数え方（ADR 0015 §3.7）のため。今回はログに足さない。
- **`Context.left_borrowed` と `with_left_borrowed` を消す**。使っている分岐（`core/src/converter/conversion.rs` の学習の記録の解析、`core/src/converter/segment.rs`、`core/src/ranker/learning.rs` の引き当て）は、借りていないときの経路だけを残す。テスト（`converter/tests.rs`、`segment_tests.rs`、`ranker/learning.rs` の中）も合わせて消す。

### 8.2 engine

- `context_limits.rs`: `CARRY_FILL_CHARS: usize = 0`（契約。掃引で替える）、`CARRY_JOINT: CarryJoint = CarryJoint::None`（同）を足し、`ContextLimits` に `carry: CarryParams` を足す。**`BORROW_WINDOW`、`RECENT_COMMITS_TTL`、`RECENT_COMMITS_MAX_CHARS` と、`ContextLimits` のそれらのフィールドを消す**。テスト `defaults_follow_adr_0015` を改める。
- `fill` と区切りは**利用者の設定ではない**（`settings.tsv` に足さない）。engine の定数で、eval では `--carry-fill`・`--carry-joint` で替えて掃引する。
- `session.rs`: `build_context_borrowing` を `build_context`（`compose_left` を呼ぶ）にまとめ、`borrowed` の引数をなくす。`context_for` の借りる判定、`borrowed_left`、`RecentCommit`、`Inner.recent`、`push_recent`、`trim_recent`（`expire_context` からの呼び出しも）、`Session.share` を消す。`push_history` はセッションの履歴に積むだけになる。
- `lib.rs`: 「最近の確定」の持ち物（`recent`）を消す。
- テスト: `engine/src/tests/context.rs` の借用のテストを「借りない」テストに置き換える。`engine/src/tests/diag.rs` の `share_context` を消す。

### 8.3 proto、殻、設定アプリ（`share_context` と `context.shared_exclude` の削除。影響範囲）

`share_context` と `context.shared_exclude` は借用のためだけの仕組みなので、廃止して消す。**作者以外の利用者がまだいないので、互換は持たない**（ADR 0042 の方針）。

| 場所 | 消すもの | 互換の扱い |
|---|---|---|
| `proto/ikari/engine/v1/engine.proto` | `BeginSession.share_context = 4` とその説明 | 互換は持たないが、番号と名前は `reserved 4; reserved "share_context";` で塞ぐ（費用がなく、番号の再利用の事故を防ぐ）。殻とエンジンは同じアプリの中で一緒に入れ替わるので、古い殻と新しいエンジンが話すことはない |
| `proto/ikari/shell/v1/shell.proto` | 設定のスナップショットの `context_shared_exclude = 10` | 同じく `reserved` で塞ぐ |
| `proto/ikari/settings/v1/settings.proto` | 設定の編集の `add_shared_exclude = 17`、`remove_shared_exclude = 18`、`reset_shared_exclude = 19` | 同じく `reserved` で塞ぐ |
| 生成物 `macos/Sources/IkariProto/*.pb.swift` | 上の 3 つから作り直す | 手で直さない |
| shell-core | `settings/defaults.rs`（キー `context.shared_exclude` と既定の一覧）、`settings/parse.rs`・`settings/values.rs`・`settings/mod.rs`（読み、検証、今効いている値、編集）、`types.rs`（`SettingsSnapshot.context_shared_exclude`）、`machine/mod.rs`（`SessionInfo.share_context`、`set_share_context`）、`controller/mod.rs`（`shares_context`）、`scenarios/shell.json` とテスト | 作者の `settings.tsv` に残る `context.shared_exclude` の行は、未知のキーとして無視される（読み込みは止まらない。警告が出るかは実装で確かめ、出るなら作者に 1 行消してもらう） |
| shell-ffi | `convert.rs`、`settings_app.rs` の対応 | ― |
| 設定アプリ（Swift） | `IkariSettings/View/InputAndConversionTabs.swift` の除外するアプリの節（`SharedExcludeSection`）、`Model/SettingsModel.swift`・`SettingsValues.swift` の対応、`Model/SharedExcludeApps.swift`、`Fake/FakeShell.swift` の対応、テスト `SharedExcludeModelTests.swift` と `EmojiShortcodeModelTests.swift` の 2 か所 | 設定画面から「除外するアプリ」の節がなくなる。**足すものはなし** |

- 絵文字のショートコードの除外（`emoji.shortcode_exclude`）は別の設定で、消さない。
- 設定画面の UI を消すのは Swift だが、判断（何を共有するか）を消す変更で、判断は足さない。

### 8.4 shell-core（A1）

- `controller/mod.rs`: `session_ids: Vec<(String, u64)>`（Bundle ID → ID。最大 64。使うたびに末尾へ）を持ち、`make_client` はそこから ID を引き、なければ `next_session_id`。64 は engine の `SESSION_LIMIT` より小さく、日常に開くアプリの数より大きい。動作に効かない上限なので計測しない。

### 8.5 eval

§7.1 の列と引数。`borrowed` の列と `b28-borrowed.tsv` を消す。`make_carry_sets.py`、`carry-chat.tsv`、`c182-carryover.tsv`、`eval/tests/scenario_lm.rs` に scenario を足す、`snapshot.sh` に派生セットを足し、b28-borrowed を外す。

### 8.6 変えないもの

`lm/`（`context_chars = 40`）、学習の形式（`learning.tsv`）、`Context` の `left_text`・`right_text`・`history`、`BeginSession` のほかのフィールド、`EndSession`、D1 の休止（30 分）と同じセッションの履歴の量（2000 文字）。

## 9. 実装の進め方

1. 契約コミットと 2 つのレーンを 1 つの統合ブランチにまとめ、1 つの PR にする（§13）。
2. 掃引の PR: §7.2 を流し、報告を `eval/reports/` に置き、既定の値を替える。ユーザーに表で確認してからマージする。

## 10. 関係する ADR

- **ADR 0015**: D1（休止と再開）はそのまま。A1 で、アプリの行き来でも効くようになる。D7（LM の窓 40）は変えない。**D5（アプリをまたぐ「最近の確定」）、D6（除外）、X5、Q7（借りた文脈で学習しない）は、この ADR で廃止する**。統合の PR で、0015 に次を追記する（案）:
  - 冒頭の状態の行に「D5・D6・X5・Q7 は ADR 0046 で廃止（2026-11-04）。ほかのアプリの文は、今の文書と文脈が同じだという保証がないため」。
  - §1 の D5・D6 の行、§2.2 の X5 の行、付録の Q4〜Q7 の見出しの末尾に「（ADR 0046 で廃止）」。
  - D8 の「D5 でほかのアプリの文を左文脈にした変換は、学習では『文脈なし』として数える」に「（ADR 0046 で借用を廃止したので、該当なし）」。
  - 本文の測定（§0.6、§4）はそのまま残す（この ADR の根拠として引いている）。
- **ADR 0044**（学習の word の加点。マージ済み）: 独立。足した履歴は学習の文脈（Bag など）の特徴量にもなるので、文脈つきの学習の引き当てが今より増える。a-02 と 0044 のシナリオが変わらないことを全セットの確認に含める。
- **ADR 0045**（直前ボーナス。別ブランチで提案中）: どちらも「ひんしを」の例を直すが、効く場面が違う。0045 は「同じ読みで一度選んだあと」、この ADR は「選ぶ前に、文脈で 1 位にする」。0045 の直前の選択は engine 全体で 1 つで、アプリをまたいで効く（文は運ばず、読みと表記の対だけ）。この ADR で `share_context` を消すので、0045 が除外のアプリを見分ける手段もなくなる。0045 をアプリをまたいで効かせるかは、0045 の側で決める（この ADR の「ほかのアプリの文を使わない」と同じに揃えるかを、ユーザーに聞く点として 0045 に渡す）。
- **#178**（左文脈は直前ほど強く）: F1 で足すのは直前の文だけで、量も小さい。#178 の結論で LM の窓や話題の重みが変われば、`carry_fill_chars` を測り直す。

## 11. 懸念

- 区切りの誤り（§3.4 の例 12）と、F1 で話題に入る無関係な文の害（C3）は、掃引で測るまで大きさが分からない。害が大きければ `fill` を小さくするか F3（LM にだけ足す）を測る。
- 借用の廃止で、「ブラウザで調べて、空のエディタに書く」の最初の変換は文脈なしになる（§5.2）。失う量は測れていない。ユーザーの判断で受け入れる。
- Electron 系の入力欄が「`available` で左が空」になるかは、実機で確かめていない（#182 の推測。ユーザー決定で確かめずに進める）。E1 は左が空でも短くても効く形にしてある。
- 空の入力欄の先頭にカーソルがあり、右に文がある（文書の頭に書き足す）とき、同じアプリの履歴を左に置く。右文脈は今どおり渡る。

## 12. ユーザーの決定（2026-11-03〜04）

- **ほかのアプリからは借りない**（2026-11-04）。今の借用（X5、`BORROW_WINDOW`、「最近の確定」）も廃止する。理由: ほかのアプリの文は、今の文書と文脈が同じだという保証がない。これに伴い、A2（左が空の `available` の欄でも借りる）、A3（書きかけの文書に足す）、A4（借りる窓を延ばす・設定にする）は不採用。`share_context` と設定 `context.shared_exclude`（設定画面の除外するアプリの節）も消す。互換は持たない。
- **同じアプリの中の文脈は保つ**: E1・E2（左の不足を同じアプリの履歴で補い、重なりを除く）と A1（アプリに戻ったら 30 分以内の履歴を続ける）を入れる。文脈は端末の中だけで使い、外へ送らない。secure は今どおり。
- 実機のログは確かめずに進める（2026-11-03）。
- 値（`carry_fill_chars`、区切りの種類）は、掃引のあとに表で聞く（§7.2）。

## 13. 実装の分け方（契約コミット → 並行レーン 2 つ）

1 つの統合ブランチ（`feat/context-carryover`。Closes #182）に、契約コミットと 2 つのレーンをまとめて 1 つの PR にする。掃引と既定の値の変更は、そのあとの別の PR。

**契約コミット**（統合ブランチに先に入れる。レーンはこれを変えない。木がビルドできる最小の修正を含む）
- `core/src/context_carry.rs`（新規）: §8.1 の型と `compose_left` を**実装まで**入れ、§3.3 の境界例を単体テストにする。純関数で小さく、両方のレーンが同じ振る舞いに依存するので、スタブにしない。`core/src/lib.rs` で公開。
- `core/src/context.rs` ほか core の中: `left_borrowed` を消す（§8.1）。共有の型の変更なので契約に入れる。
- 呼び出し側をビルドが通るだけ直す: `engine/src/session.rs` の `ctx.left_borrowed = true` の 1 行と、`eval/src/scenario.rs` の `with_left_borrowed`（`borrowed` の列は読むが使わない状態にしておく。レーン B が列ごと消す）。
- `engine/src/context_limits.rs`: §8.2 の定数の追加と削除。既定は `fill = 0`。

**レーン A（借用の削除と A1。engine、shell-core、shell-ffi、proto、設定アプリ）**
- 持つファイル: `engine/src/session.rs`、`engine/src/lib.rs`、`engine/src/tests.rs`・`engine/src/tests/context.rs`・`engine/src/tests/diag.rs`、`proto/ikari/{engine,shell,settings}/v1/*.proto` と生成物 `macos/Sources/IkariProto/*.pb.swift`、`shell-core/src/`（`controller/`、`machine/`、`settings/`、`types.rs`、`engine/tests.rs`）、`shell-core/scenarios/shell.json`、`shell-core/tests/scenarios.rs`、`shell-ffi/src/`、`macos/Sources/IkariSettings/` と `macos/Tests/IkariSettingsTests/` の §8.3 のファイル。
- 内容: `compose_left` への置き換え、借用と「最近の確定」の削除（§8.2）、`share_context` と `context.shared_exclude` の削除（§8.3）、A1（§8.4）。proto を変えるので、Rust と Swift を同じレーンで揃える（分けると、片方だけではビルドが通らない）。Swift は消すだけ。macOS の `swift test` を流す。
- 触らない: `core/`、`eval/`。

**レーン B（eval）**
- 持つファイル: `eval/src/set.rs`、`eval/src/scenario.rs`、`eval/src/args.rs`、それらを呼ぶ `eval/src/main.rs`・`eval/src/run.rs` の配線、`eval/scripts/make_carry_sets.py`（新規）、`eval/sets/carry-*.tsv`（派生 C1〜C5 と手書きの carry-chat。carry-chat は Claude の下書きで、ユーザーの確認待ちと冒頭に書く）、`eval/scenarios/c182-carryover.tsv`（新規）と `eval/scenarios/b28-borrowed.tsv`（削除）、`eval/tests/scenario_lm.rs`、`eval/scripts/snapshot.sh`。
- 内容: §7.1 の列（`field`・`history`）と引数（`--carry-fill`・`--carry-joint`）。`borrowed` の列を消す。列がないセットとシナリオは今と同じに動くことをテストで確かめる。
- 触らない: `core/`、`engine/`、`shell-core/`、proto、Swift。

**統合（リード）**: 2 つのレーンを統合ブランチにまとめ、フルの `scripts/check.sh`（swift test を含む）、`eval/scripts/snapshot.sh`（`fill = 0` で、借用に頼っていたケースを除き全セットが変わらないこと。eval のセットは借用を通らないので、変わらないはず）、`c182-carryover.tsv` を LM あり・なしで流す（`fill = 40` を引数で与えて期待を確かめる）。ADR 0015 への追記（§10 の案）もここで行う。レーンの間に共有のファイルはないので、まとめるときに衝突は出ない見込み。
