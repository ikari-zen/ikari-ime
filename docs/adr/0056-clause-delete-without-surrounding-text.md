# ADR 0056: 周辺テキストが取れないアプリでの文節削除（直前の確定を覚えておき、確かめられないときはキーを奪って何もしない）

- 状態: 提案（2026-11-07）
- 日付: 2026-11-07
- 対象: shell-core（`machine/clause_delete.rs`、`machine/mod.rs` の確定の口と覚えを捨てる口、`clause.rs` の定数、`scenarios/clause-delete.json`）。段階 2 だけ: shell-core の `TextAccess` に読み出し 1 つ、Swift の `ClientAccess` にその実装（`string(from:actualRange:)` の 1 回）。proto、engine、core は変えない
- 前提: main = f703e54。ADR 0050（文節ごとの削除。D5 と §12 の Q1 を本 ADR が一部改める）、ADR 0053（取り消し。英字の直前の素通し）、ADR 0006 §9.5（拒否リスト）、ADR 0042（判断は Rust）
- タスク: #193 の続き（実機の報告 2026-11-07）。依存: 断った理由の診断ログ（別のタスク。本 ADR の §5 の `ClauseRefusal` を記録する）
- 判断の置き場所: **すべて Rust（shell-core）**。何を覚えるか、いつ捨てるか、断るときに奪うか渡すか、覚えで消してよいかの検査は shell-core が決める。Swift は段階 2 で、範囲の文字列を 1 回読む関数を足すだけ（OS の API でしか取れないため）

---

## 0. 現状の把握

### 0.1 分かったこと（コードと実機で確かめた）

- 実機（2026-11-07）: 確定の直後、キャレットが文末にある状態の Option+Delete が、メモでは文節を消し、VS Code では**文がまるごと消えた**。macOS の単語の削除は、空白のない日本語では句読点までをひとまとまりに消す。
- 確定後の削除は `committed_target`（`shell-core/src/machine/clause_delete.rs`）で対象を作り、作れなければ `idle_delete` が `PassThrough` を返す（ADR 0050 D5）。断る条件は次のとおりで、**どれも同じ `None` で、理由を区別していない**:
  - 拒否リスト（`clause::CLAUSE_DELETE_DENIED_BUNDLES` = Terminal と iTerm2）
  - `surrounding()` が `None`（Swift の `surroundingText()` は、`selectedRange` が `NSNotFound` か、左の 2000 文字の読み出しが nil のとき nil）
  - `caret` が `None`、選択が空でない、`is_available` が偽（左が空で文書の長さが 0）、`left_matches_caret` が偽（位置の検査）
  - 直前が英字（`ends_with_ascii`。ADR 0053 §6.3）
  - 切り出した文が空、`Reconvert` の失敗・時間切れ、区切りが文と食い違う
- shell-core は、**ikari が確定した文字列を覚えていない**。確定は `commit_converting`（文節の表記の列 `segments` がその場にある）、`commit_text`（1 文節）、ショートコードなどの口で `ShellAction::Commit` を返し、確定履歴はエンジンへ送るだけ（`CommitText`、`CommitSegments`。再変換の `pieces_from_history` はエンジンの側）。
- 覚えを捨てる口はすでにある。ADR 0053 の `clause_undo` は、`handle` の先頭（文節のキー以外）、`activate`、`deactivate`、`commit_composition`、`set_secure(true)`、`reconfigure`、英語のモードの口（`controller` の `forget_clause_undo`）で捨てている。同じ口に足せる。
- Swift から取れるもの（`ClientAccess.swift`）: `selectedRange`、`markedRange`、`length`、`string(from:actualRange:)`、`insertText(_:replacementRange:)`、`setMarkedText`。**キャレットを動かす API と、キーを合成して送る API は IMK にない**（ADR 0050 D10）。マウスのクリックは、変換中の文字列がない間は IMK に届かない。
- VS Code では、IMK の `attributes(forCharacterIndex:)` が絶対位置ではなく相対位置で答える（ADR 0004 の S0-3。`caretRects` の注記）。

### 0.2 推測（確かめていない。§8 のスパイクと診断ログで決める）

- VS Code で断った理由は、次のどれか。H1: `selectedRange` が `NSNotFound`（周辺テキストなし）。H2: Monaco の隠れた入力欄が短く、左が空で長さ 0（`is_available` が偽）。H3: 相対位置で `left_matches_caret` が偽。H4: Option+Delete が IMK の keyDown に届かず、アプリが先に処理した。H5: `Reconvert` の時間切れ。**H4 なら本 ADR では直らない**（§9 の懸念 1）。
- Chrome の URL 欄、Claude のデスクトップアプリの入力欄、Slack も、H1〜H3 のどれかに当たる。

## 1. 問題の定義

1. 断るときに `PassThrough` すると、日本語の文ではアプリの単語の削除が**文をまるごと消す**。ADR 0050 の Q1 で (a) を選んだ理由（ターミナルや英語の文で、今の Option+Delete を失わない）は英語の文には正しいが、日本語の文では「意図しない大きな削除」になり、方針の 2（ユーザーの意図に忠実）に反する。
2. 周辺テキストが取れないアプリでは、キャレットの前に何があるかを ikari は文書から知れない。ただし、確定の直後なら、**その文字は ikari 自身が今入れたもの**で、shell-core は知りうる（覚えていないだけ）。
3. 覚えた文字で消すには、キャレットがまだその直後にあることを確かめる手段が要る。クリックは見えないので、キーだけでは確かめられない。

## 2. 決定の要約

| # | 決定 |
|---|---|
| D1 | shell-core が「**直前の確定**」（`CommitTail`）を覚える。ikari が続けて確定した文字列と文節の境界で、間に素通しのキー（`PassThrough`）、文書を書き換える動作（`Restore`、`DeleteText`）、フォーカスの変化がなかったもの。idle のときだけ使う。上限は 1 文（文の終わりの記号と改行で切る）かつ 64 文字（`MAX_RECONVERT_CHARACTERS`）。外へは出さない（§3） |
| D2 | 確定後の断りの理由を、**文書の中身が分かっているか**で 2 つに分ける（`ClauseRefusal`。§5）。「見えない」（周辺テキストなし、`caret` なし、`is_available` が偽、位置の検査に落ちた、拒否リスト）と、「分かっている」（直前が英字、選択がある、消すものがない、`Reconvert` が `Empty`、区切りの食い違い、時間切れ・エンジンが落ちた） |
| D3 | **段階 1**（契約を変えない）: 「見えない」で断るとき、`CommitTail` があれば `Consume`（キーを奪って何もしない）。なければ今どおり `PassThrough`。ikari が今入れた日本語の文を、アプリの単語の削除にまとめて消させない |
| D4 | 「分かっている」の中で、左が日本語なのに区切りが取れないとき（時間切れ、エンジンが落ちた、区切りの食い違い）は `Consume` に改める（今は `PassThrough`）。読みの入力中の時間切れ（ADR 0050 §3.1）と同じ扱い。直前が英字、`Empty`、選択あり、消すものがない、は今どおり `PassThrough` |
| D5 | **段階 2**（スパイクのあと。§8）: 「見えない」で断り、`CommitTail` があるとき、キャレットの直前の n 文字（UTF-16。n は削除の単位の長さ）を**読み戻して覚えと一致したときだけ**、その範囲を `DeleteText` で消す。読み戻せない・一致しないときは `Consume`。有効にするのは、スパイクで正しく消せると確かめたアプリ（許可リスト。殻ではなく shell-core の定数）だけ（§4、§10 の Q3） |
| D6 | 段階 2 の削除は、ADR 0053 の取り消しの段を積まない（検査に要る左右の文字が取れないため）。`clause.undo` はアプリへ渡る。取り消しはアプリの Cmd+Z に任せる（§8 の S1-d で確かめる） |
| D7 | 覚えで消すときの単位は ADR 0050 §4 と同じ関数（`clause::unit_range`。最小文字数、文の終わりの記号の扱いも同じ）で、境界は覚えた文節の境界、文頭は覚えの先頭とみなす |

## 3. 直前の確定（`CommitTail`）

### 3.1 型（`shell-core/src/machine/clause_delete.rs`。外には出さない）

```rust
/// ikari が続けて確定した、キャレットの直前の文字列（ADR 0056 §3）。idle の間だけ意味を持つ
pub(super) struct CommitTail {
    /// 表示の文字列そのまま（続けて確定したものをつなぐ）
    text: String,
    /// 文節の境界（`text` の書記素の位置の昇順。0 と末尾を含む）
    bounds: Vec<usize>,
}
// InputStateMachine に `commit_tail: Option<CommitTail>` を足す
```

### 3.2 覚える・足す

- 覚えるのは、`Commit` を返す口の 1 か所（`handle` の中で、`dispatch` の返した動作の列を見る）ではなく、**確定を作る関数**で足す（`commit_converting` は `segments` の表記の長さを境界に、`commit_text`・ショートコードなど `Commit` を返すほかの口は全体を 1 文節に）。文節の情報がその関数にしかないため。
- 足すのは secure でないときだけ。続けて確定したら後ろにつなぐ（「今日は」を確定 →「雨が降る」を確定 → 覚えは「今日は雨が降る」、境界はそれぞれの文節）。
- 確定と同時に次の読みを始める打鍵（`[Commit, SetMarkedText]`）でも足す。そのあと読みの入力中・変換中の間は、文書は変わらない（変換中の文字列は文書の外）ので、覚えを保つ。Option+Delete は読み・変換の規則（ADR 0050 §3.1・§3.2）で扱い、覚えは使わない。
- 覚えるのは最後の 1 文だけ。文の終わりの記号（ADR 0050 §3.3 と同じ集合。続く閉じ括弧を含む）で終わる間は、記号を末尾に含めたまま持つ（「…ました。｜」で消すと「ました。」の単位になる。ADR 0050 §3.3 と同じ）。記号のあとに続けて確定したら、記号までを捨てる。64 文字を超えたら先頭を捨て、残りの先頭の境界を文頭とみなす。

### 3.3 捨てる

| 事象 | 理由 |
|---|---|
| 返した動作に `PassThrough` がある（idle の Backspace、矢印、Enter、Cmd 付きのキー、英語のモードの打鍵など） | アプリが文書かキャレットを動かした |
| `Restore`、`DeleteText`（再変換、文節削除を含む） | 文書が変わった（例外は段階 2 の覚えの経路の削除。§9.2 の 4） |
| `activate`、`deactivate`、`commit_composition`、`set_secure(true)`、`reconfigure`、モードの切り替え | ADR 0053 §4 と同じ |
| 時間 | 捨てない（ADR 0053 §7.4 と同じ。値を足さない） |

- マウスのクリックは見えない。段階 1 では、クリックのあとに覚えが残っていても、結果は「何も起きない」（`Consume`）だけで文書は壊れない。段階 2 は読み戻しで止める（§4）。

## 4. 段階 2: 覚えと読み戻しで消す

1. `committed_target` が「見えない」で断り、`CommitTail` があり、アプリが許可リストにある。
2. 覚えの末尾から単位を 1 つ作る（D7。`cursor` = 覚えの長さ）。n = 単位の UTF-16 の長さ。
3. `selectedRange` を取る（`caret`。長さ 0 でなければ `PassThrough`。今の規則）。`caret.location < n` なら `Consume`。
4. `[caret.location − n, caret.location)` の文字列を読み戻す（新しい `TextAccess::read`）。`actualRange` が違う、nil、覚えの単位と違う、のどれかなら `Consume`。
5. 一致したら `DeleteText { range: [caret.location − n, caret.location) }`。覚えから単位を除く（続けて押すと次の単位を消せる）。

```rust
// shell-core/src/host.rs（段階 2 の契約。段階 1 では足さない）
pub trait TextAccess {
    // …今の 2 つ…
    /// 文書の範囲の文字列（UTF-16 の範囲）。アプリが別の範囲を返した、取れないときは `None`
    fn read(&mut self, range: Utf16Range) -> Option<String>;
}
```

- 読み戻しは、`selectedRange` と `string(from:)` が**同じ座標**（絶対でも相対でも）で答えることを確かめる。`insertText` の `replacementRange` も同じ座標かは、読み戻しでは確かめられない。だから許可リストにする（S1-d で確かめたアプリだけ）。
- 今の `surroundingText()` は左の 2000 文字を読めないと全体を nil にする。短い範囲なら読めるアプリがあるかもしれない（推測。S1-c）。`read` を `surrounding()` と別にするのはこのため。
- Swift の `ClientAccess` は `string(from:actualRange:)` を 1 回呼び、`actualRange` が要求と同じときだけ返す（`selectedText()` と同じ形。判断はしない）。

## 5. 断りの理由（`ClauseRefusal`。shell-core の中の型）

```rust
/// 確定後の文節削除を断った理由（ADR 0056 §5）。診断ログ（別のタスク）がこの名前をそのまま記録する
pub(super) enum ClauseRefusal {
    // 見えない（文書の中身が分からない）
    Denied, NoSurrounding, NoCaret, Unavailable, CaretMismatch,
    // 分かっている
    Selection, AsciiBefore, NothingToDelete, Empty, LayoutMismatch, EngineFailed,
}
impl ClauseRefusal {
    fn is_blind(self) -> bool; // 見えない側か
}
// committed_target は Result<CommittedPreview, ClauseRefusal> を返す
```

| 理由 | 段階 1 | 段階 2 |
|---|---|---|
| 見えない、覚えあり | `Consume` | 許可リストなら §4（だめなら `Consume`）。ほかは `Consume` |
| 見えない、覚えなし | `PassThrough`（§10 の Q2） | 同じ |
| `Selection`、`AsciiBefore`、`NothingToDelete`、`Empty` | `PassThrough`（今どおり） | 同じ |
| `LayoutMismatch`、`EngineFailed` | `Consume`（D4） | 同じ |

- プレビュー（`clause.delete_preview`）も同じ表。段階 2 でも、覚えの経路のプレビューは作らない（`setMarkedText` の `replacementRange` も同じ座標の問題を持つ。要るときに別に決める）。`Consume` する。
- 英語のモード（ADR 0053 U9）でも同じ表。英語のモードの打鍵は `PassThrough` なので、英語で打った直後は覚えがなく、今どおりアプリの単語の削除になる。

## 6. 検討した案

| 案 | 中身 | 良い点 | 悪い点 | 結論 |
|---|---|---|---|---|
| 1 | 断るときは常に `Consume` | 意図しない削除が起きない。実装が 1 行 | 英語の文、ターミナル、選択の削除など、アプリの Option+Delete が要る場面まで失う。ADR 0050 Q1 の理由を全部捨てる | 採らない。「見えない かつ 覚えあり」と「左が日本語と分かっている」に絞って採る（D3、D4） |
| 2 | ikari の確定を覚え、キャレットが確定の直後と確かめられるときだけ消す | VS Code などで文節削除が使える | キャレットの位置の確かめに、座標が揃うアプリが要る。クリックは見えない | **段階 2 として採る**。確かめは読み戻し、有効は許可リスト（D5） |
| 3 | 今のまま `PassThrough` | 変えない | 今回の害（文がまるごと消える）が残る | 採らない |
| 4a | 合成したキー（Backspace を n 回。`CGEventPost`）で消す | 座標が要らない。ターミナルでも消せる | アクセシビリティの権限が要る（こちらから求めない。NFR-20）。合成した打鍵が利用者の打鍵と前後しうる。クリックのあとは違う場所を消す（読み戻しができない）。判断が Swift に寄る | 採らない |
| 4b | 確定を遅らせて、直前の確定を変換中の文字列のまま持つ | 文書に入る前なので確実に消せる | こまめに確定するコンセプト（方針の 1）に反する。ほかのアプリへの切り替えで確定の扱いが要る | 採らない |
| 4c | 日本語のモードなら、覚えがなくても「見えない」は `Consume` | クリックや貼り付けのあとでも、日本語の文をまとめて消さない | 日本語のモードのまま英語のコードや英文を直すとき、アプリの単語の削除を失う（英語のモードに替えれば使える） | §10 の Q2 で聞く（推奨はしない） |
| 4d | 覚えの経路で、読み戻しなしに `DeleteText` | 読み戻しの契約が要らない | クリックのあとに違う位置を消す。方針の 2 に反する最悪の結果 | 採らない |

- 段階に分ける理由: 段階 1 だけで今回の害（確定の直後に文がまるごと消える）は止まり、文書を消す新しい経路を作らない（壊しうるのは「何も起きない」だけ）。段階 2 は消す経路を足すので、アプリごとの実機の確認を前提にする。

## 7. リスク

1. **違う位置を消す**（段階 2）: `selectedRange`・`string(from:)` と `insertText` の座標が食い違うアプリ。読み戻しでは見つからない。許可リストで、S1-d を通ったアプリだけにする。許可リストにないアプリは段階 1（`Consume`）のまま。
2. **クリックのあとの覚え**: 段階 1 では無害（`Consume`）。段階 2 は読み戻しで止める。クリックの先のキャレットの前に、偶然同じ文字列がある場合は消す（IMK にクリックを知る手段がないので防げない）。消すのは覚えた単位とまったく同じ文字列で、利用者がいま見ている位置なので、害は小さいと判断する（推測）。
3. **ほかのアプリで壊れる**: 段階 1 の変更は「`PassThrough` だったものが `Consume` になる」だけで、文書を変えない。ただし、拒否リスト（ターミナル）で日本語を確定した直後の Option+Delete は、シェルの単語の削除が効かなくなる（ikari が奪う）。
4. **H4（キーが IMK に届かない）**: 本 ADR は効かない。S1-a で届かないと分かったアプリは、文書（使い方の注意）に書くだけにする。

## 8. 実機のスパイク（ADR 0050 §8 の S1 に足す。検証版の差し替えはユーザーの了解を取ってから）

対象: VS Code（エディタと統合ターミナル）、Chrome の URL 欄、Claude のデスクトップアプリの入力欄、Slack、Terminal、iTerm2。比べるためにメモ。手順は「日本語を 2〜3 文節で確定 → そのまま Option+Delete」と、「確定 → 文の途中をクリック → Option+Delete」。

| # | 確かめること | 決まるもの |
|---|---|---|
| S1-a | Option+Delete が IMK の keyDown に届くか。届いたなら断った理由（`ClauseRefusal`。診断ログ） | H1〜H5 のどれか。H4 のアプリは §7 の 4 |
| S1-b | 確定の直後の `selectedRange`（`NSNotFound`、相対、絶対）と `length()` の値。クリックのあとに値が変わるか | 段階 2 の読み戻しが成り立つか |
| S1-c | `string(from: [caret − n, caret))` が確定した文字列を返すか。`actualRange` が要求と同じか。2000 文字の読み出しとの違い | `TextAccess::read` を足す価値 |
| S1-d | `insertText("", replacementRange: [caret − n, caret))` がちょうどその範囲を消すか。1 回の Cmd+Z で戻るか | 許可リストに入れるか（D5、D6） |
| S1-e | 段階 1 の検証版で、確定の直後の Option+Delete が何も起こさないこと、英語のモードで打った直後はアプリの単語の削除のままなこと | 段階 1 のマージ |

- 段階 1 は S1-a（断った理由）が分かれば十分で、S1-b〜d を待たない。段階 2 は S1-b〜d のあと。

## 9. 実装の進め方と懸念点

### 9.1 進め方

1. **依存**: 断った理由の診断ログ（別のタスク）。`ClauseRefusal` の型は段階 1 の最初のコミットで入れ、診断ログはその名前を記録する（順はどちらが先でもよい。型を先に入れたほうが名前が揃う）。
2. **段階 1**（code-editor 1 つ。Sonnet。持つファイル: `shell-core/src/machine/clause_delete.rs`、`shell-core/src/machine/mod.rs`、`shell-core/src/clause.rs`、`shell-core/scenarios/clause-delete.json`）: `ClauseRefusal`、`CommitTail` と捨てる口、§5 の表。シナリオを先に書く。
   - 「確定 → Option+Delete（周辺テキストなし）→ `Consume`」「確定 → 矢印（素通し）→ Option+Delete（周辺テキストなし）→ `PassThrough`」「確定 → 拒否リストのアプリで Option+Delete → `Consume`」「英語のモードで打った直後 → `PassThrough`」「周辺テキストあり、`Reconvert` の時間切れ → `Consume`」「直前が英字 → `PassThrough`」「続けて 2 回確定 → 覚えがつながる」「確定と同時に読みを始める → 読みの入力中の Option+Delete は読みの規則」。
3. **段階 2**（S1 のあと。shell-core のレーンと Swift の 1 関数。契約のコミットで `TextAccess::read` と許可リストの定数（空）を先に入れる）。
4. 統合: `scripts/check.sh`（フル）、シナリオ、`swift test`。

### 9.2 懸念点

1. H4 なら、段階 1 も段階 2 も効かない。S1-a を先にする。
2. `Consume` は「押したのに何も起きない」に見える。段階 1 だけの間、VS Code などでは文節削除が使えないことを文書（使い方の注意）に書く。
3. ADR 0050 の Q1（`PassThrough`）を、日本語の文の場面に限って改める。英語の文と、覚えがない場面は今どおり。
4. ふつうの経路（周辺テキストあり）の `DeleteText` では覚えを捨てる（そのアプリでは覚えを使わない）。段階 2 の覚えの経路の `DeleteText` だけは、覚えから単位を除いて保つ（続けて押すと次の単位を消せる。§4 の 5）。

## 10. ユーザーに聞くこと（推奨つき）

- **Q1. 周辺テキストが取れないアプリで、ikari が確定した直後の Option+Delete**: (a) キーを奪って何もしない（段階 1）。左が日本語と分かっているのに区切りが取れないとき（時間切れなど）も同じ、(b) 今どおりアプリへ渡す（文がまるごと消えうる）。**推奨は (a)**。意図しない大きな削除を止めるのが先で、「何も起きない」は Backspace で続けられる。ADR 0050 の Q1 の決定を、この場面に限って改める。
- **Q2. 覚えがない（クリック、貼り付け、矢印のあと）ときに、周辺テキストが取れないアプリでの Option+Delete**: (a) 今どおりアプリへ渡す、(b) 日本語のモードなら奪って何もしない（英語のモードはアプリへ渡す）。**推奨は (a)**。キャレットの前が日本語か英語のコードか、ikari には分からない。日本語のモードのまま VS Code で英語のコードを直す人の、単語の削除を奪わない。(b) にすると日本語の文の大きな削除は残らず止まる。
- **Q3. 段階 2（覚えと読み戻しで実際に消す）**: (a) スパイクで正しく消せると確かめたアプリだけで有効にする（許可リスト）、(b) 読み戻しが一致すればどのアプリでも消す、(c) 段階 2 はしない。**推奨は (a)**。読み戻しでは削除の座標までは確かめられず、違う位置を消すのが最悪の結果のため。
