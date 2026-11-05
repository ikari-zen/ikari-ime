# ADR 0050: 文節ごとの削除（Option+Delete ですぐ消す、Shift+Option+Delete で対象を見てから消す）

- 状態: 採用（2026-11-05 ユーザー決定。キー、最小文字数の数え方、途中のカーソルの扱い、§12 の Q1〜Q3 はすべて推奨どおり、最小文字数の設定範囲は 1〜10）
- 日付: 2026-11-05
- 対象: shell-core（`keymap.rs`、`settings/`、`types.rs`、`host.rs` の周辺テキストの型、`machine/`、新しい `clause.rs`、`scenarios/clause-delete.json`）、proto（`shell.proto` に動作 1 つと項目 2 つ、`engine.proto` に要求 1 つ）、core（`converter/segment.rs` に文節を取り除く操作）、engine（その要求の受け口）、Swift（動作の実行 1 つ、周辺テキストにキャレットの位置、設定画面の項目、削除対象の見た目）、プリセットの TSV 3 つ、`docs/phase2-contract.md` §3・§4
- 前提: main = 80bc52e。ADR 0006（文節のモード §8、再変換 §9、置き換え §9.5、最初の Space は全体の候補 §16.6）、ADR 0041（shell-core の契約）、ADR 0042（判断は Rust）
- タスク: #193
- 補足（2026-11-06）: 削除の「元に戻す」（`clause.undo`、既定 Ctrl+Z）と、英語のモードでの文節削除・英字の直前での素通しは [ADR 0053](0053-clause-delete-undo.md)。§7 の「英語のモード」の行は、0053 §10 の Q1 が決まったら改める。確定後の削除は、`replacementRange` を無視するアプリ（TextEdit）では空の変換中の文字列に置き換えて確定する（PR #199）
- 判断の置き場所: **すべて Rust（shell-core）**。どこからどこまでを消すか、最小文字数、文頭・文末、断る条件、プレビューの状態は shell-core が決める。Swift は、範囲を消す 1 回の `insertText`、キャレットの位置を周辺テキストに足すこと、削除対象の見た目（UI）だけを持つ。文節の区切りは core（エンジン経由）が返す

---

## 0. 現状の把握（researcher の調査とコードで確かめたこと）

- Swift は判断しない。NSEvent → `KeyInput` → shell-core。`Machine::dispatch`（`shell-core/src/machine/mod.rs`）は `bound_action` でキー割り当てを先に引く。割り当てにない Option 付きのキーは、idle なら `PassThrough`、読みの入力中と変換中は表示を確定してから渡す。
- 3 つのプリセット（`macos/Resources/keymap/{macos,ms-ime,atok}.tsv`）に `option+backspace`、`shift+option+backspace`、`option+left/right` はない。`shift+left/right` は変換中の区切り直し（`segment.shrink/expand`）で使っている。
- 読みの入力中（`InputState::Composing { kana, pending }`）に読みの中のカーソルはない（常に末尾）。区切りは持たない。読みが変わるたびに `Prefetch` が先行計算を作る。
- 変換中（`ConvertingState`）は、文節のモードなら `segments` と `focused`、最初の Space のあと（全体の候補）なら `whole_layout` に区切りがある。文節を取り除く操作はエンジンにない（`ResizeSegment` は区切りを動かすだけ）。
- 確定後（idle）の区切りを知る手段は `Reconvert`（`core/src/converter/reconvert.rs`）だけ。確定履歴 → 表記の解析の順で読みを求め、`group_words` で文節に分ける。エンジン往復は p99 2.7 ms（ADR 0006 §16.5）。64 文字まで（`MAX_RECONVERT_CHARACTERS`）、改行を含むと断る（`is_reconvertible`）。全部が読めない（英字だけなど）と `Empty`。
- エンジンへの要求は `dispatch` の中で同期に待つ（`EngineTransport::request`、上限つき）。返事を見てから `PassThrough` か `Consume` かを決められる。
- `ShellAction::Restore { text, replacing }` は Swift で `insertText(text, replacementRange:)` を 1 回呼ぶ（`ClientAccess.restore`）。意味は「変換中の範囲を元の表記に戻す」で、`markedRange` と違えばログを出す。変換中の文字列がない idle の削除にそのまま使うと、意味が合わず、毎回ログが出る。
- `SurroundingText`（shell-core の型）は `left`、`right`、`document_length` だけで、**キャレットの絶対位置がない**。`TextAccess::selection()` は空の選択では `None`。範囲を指定して消すには、キャレットの位置（UTF-16）が要る。Swift の `surroundingText()` は `client.selectedRange()` から left/right を作っているので、位置はそこにある。
- `setMarkedText` の `replacementRange` が効かないアプリは、再変換を始めない拒否リスト（ADR 0006 §9.5、殻の定数）がある。周辺テキストが取れないアプリ（ターミナル、一部の Electron）では `surroundingText()` が nil。
- 英語のモード（ADR 0026）とパスワード欄（secure）は、打鍵をアプリに渡す。

## 1. 問題の定義

1. 変換中と確定後に、文節の単位で消す手段がない。読みごと消すか、1 文字ずつ消すしかなく、こまめに確定するほど（方針の 1）、確定した直後の言い直しが重くなる。
2. 確定後の区切りは、文書の中に残っていない。区切りを求め直す必要がある（ikari が確定した直後なら確定履歴で正確に、そうでなければ解析で）。
3. 消す前に範囲が分かる手段（プレビュー）がない。確定済みの文字列にハイライトを付ける既存の手段は、再変換の「変換中の文字列に置き換える」だけ。
4. idle で範囲を消す動作と、その範囲を作る材料（キャレットの位置）が契約にない。

## 2. 決定の要約

| # | 決定 |
|---|---|
| D1 | 操作を 2 つ足す。`clause.delete`（既定 `option+backspace`。直前の文節をすぐ消す）と `clause.delete_preview`（既定 `shift+option+backspace`。削除の対象をハイライトするモードに入る）。3 つのプリセットすべてに入れる。`keymap.<操作>` で変えられ、`none` で外せる（ユーザー決定） |
| D2 | 「削除の単位」は、core の文節を、最小文字数（設定 `clause.min_chars`、既定 3。表示の文字数＝書記素で数える）に満たなければ移動する側の隣と合わせたもの。文頭・文末に当たったら、満たなくてもそこで止める。カーソルが文節の途中なら、移動する側の次の境界までを最初の塊にする（ユーザー決定。§4） |
| D3 | 状態ごとの対象: 読みの入力中は末尾の単位、変換中（全体の候補）は末尾の単位、変換中（文節のモード）は**フォーカスしている文節そのもの**（最小文字数で広げない）、確定後はキャレットの直前の単位（§3） |
| D4 | 確定後の区切りは、既存の `Reconvert` の要求をそのまま使い、返事の区切りだけを読んで `Cancel` する（新しい要求は作らない。§6.2） |
| D5 | 確定後に消せないとき（周辺テキストがない、選択がある、拒否リスト、secure、英語のモード、区切りが取れない、エンジンが落ちた・時間切れ）は、**奪わずに `PassThrough`**。アプリの Option+Delete（単語の削除）がそのまま働く（§7。§12 の Q1） |
| D6 | プレビューは、押している間ではなく**モード**にする。`clause.delete_preview` で入り、←/→ で対象を動かし、Backspace（修飾キーは問わない。同じキーをもう一度押すのを含む）で消し、Esc で何もせずに抜ける。それ以外のキーは、抜けてからそのキーをふつうに扱う。修飾キーを離しても抜けない（`flagsChanged` を使わない） |
| D7 | 確定後のプレビューは、再変換の始まりと同じ手段（`setMarkedText` の `replacementRange`）で、**対象の単位だけ**を変換中の文字列にして見せる。対象を動かすときは元に戻して（`Restore`）、次の単位を同じように置く（§12 の Q2） |
| D8 | 契約: `ShellAction::DeleteText { range }` を足す（idle の削除）。`SurroundingText` にキャレットの範囲 `caret` を足す。`MarkedText` に削除対象の見た目の印 `delete_target` を足す。エンジンに `RemoveSegments` を足す（変換中の削除）。§5 |
| D9 | 学習には触れない。確定した時点の学習は消さない（消すのは内容の書き直しかもしれず、変換の誤りの証拠ではない。「学習は支援であって決定ではない」） |
| D10 | 確定後の「文節ごとの移動」（#193 の要件 3）は、この ADR では入れない。IMK にキャレットを動かす API がなく、実機のスパイク（§8 の S3）の結果で別の ADR にする。変換中の移動は今の ←/→（`segment.focus_left/right`）がそれに当たる（§12 の Q3） |

## 3. 状態ごとの動き

### 3.1 読みの入力中（`Composing`）

| キー | 動き |
|---|---|
| `clause.delete` | 今の読みで `Convert` を送り（`Prefetch` の先行計算に当たれば待たない）、返事の区切り（`layout`）だけを読んで `Cancel` する。末尾から単位を 1 つ取り（D2）、その読みを `kana` から除く。未確定のローマ字（`pending`）は対象に含める（一緒に消す）。残りが空なら idle に戻る（Backspace で空にしたときと同じ）。区切りがない返事（1 文節）なら全部を消す |
| `clause.delete_preview` | 同じ区切りで単位を作り、変換中の文字列を単位ごとに分けて、末尾の単位を `delete_target` で見せる（§3.4） |

- 時間切れ・エンジンが落ちたとき: `Consume`（何もしない）。`PassThrough` にすると、今の規則で表示を確定してから渡すことになり、読みを確定してしまうため。
- 最小文字数は表示している読み（かな）の書記素で数える。読みの先頭が文頭（読みの始まりは常に文頭とみなす）。

### 3.2 変換中（`Converting`）

- **全体の候補を出している間**（ADR 0006 §16.6）: `clause.delete` は、←/→ で文節のモードに入るのと同じ手順で入り、末尾の単位（D2。最小文字数で広げる）を `RemoveSegments` で取り除く。残りは文節のモードのまま。全部が消えたら `Cancel` して idle。表示している全体の候補が区切りと合わない（0 番以外を選んでいる）ときも、← と同じく区切りの表記で文節のモードに入ってから消す（§11 の懸念 3）。
- **文節のモード**: `clause.delete` はフォーカスしている文節を取り除く。最小文字数で広げない（利用者が ←/→ と Shift+←/→ で対象を選び終えているため）。フォーカスは、取り除いた位置の左の文節（左端なら新しい左端）。残りの文節の選択（ページ、選んだ候補、字種変換の `Literal`）は保つ。
- 再変換の中（`ConversionOrigin::Reconversion`）でも同じ。Esc は今どおり元の表記を書き戻す（消した文節も戻る）。
- `clause.delete_preview`: 文節のモードに入り（全体の候補なら上と同じ手順）、プレビューのモードにする（フォーカスの見た目を `delete_target` にし、候補ウィンドウを閉じる）。モードの中の Backspace は「フォーカスしている文節を取り除く」（ふだんの「読みに戻る」ではない）。

### 3.3 確定後（idle）

`clause.delete` の手順:

1. 断る条件（§7）を見る。どれかに当たれば `PassThrough`。
2. 周辺テキスト（`left`、`right`、`caret`）から、キャレットを含む文を切り出す。左は直前の文頭まで、右は文末まで。合わせて 64 文字（既存の上限）を超えるときは、左を優先して右を削る（それでも左が 64 文字を超えるなら、キャレットから 64 文字まで）。
3. その文字列で `Reconvert` を送り、返事の区切り（各文節の表記の長さ）を読み、`Cancel` する。
4. キャレットの直前の単位（D2）を求め、その UTF-16 の範囲で `DeleteText` を返す。

- 選択が空でない（`caret.length > 0`）ときは `PassThrough`（アプリが選択を消す）。
- 文頭: 文書の先頭（`left` が上限より短く、切られていない）、または文の終わりの記号（`。．.！？!?`。後ろに続く閉じ括弧 `」』）)` を含む）と改行の直後。キャレットの直前が文の終わりの記号なら、その記号はその前の文の末尾の単位に含める（「…ました。｜」で消すと「ました。」の単位が消える）。文末は対称に決める。
- 確定後のプレビュー（`clause.delete_preview`）は §3.4。

### 3.4 プレビューのモード（3 つの状態で共通の規則）

| キー（モードの中） | 動き |
|---|---|
| ← / →（修飾キーは問わない） | 対象を左右の単位へ。文頭・文末（確定後は切り出した文の端、読みの入力中は読みの端）で止まる |
| Backspace（修飾キーは問わない。`clause.delete` と `clause.delete_preview` を含む） | 対象を消してモードを抜ける。読みの入力中と変換中は残りの状態に戻る。確定後は `Restore { text: "", replacing: 対象の範囲 }`（変換中の文字列にした対象を空で置き換える） |
| Esc | 何もせずに抜ける。確定後は `Restore { text: 元の表記 }` で戻す |
| それ以外のキー | 抜けてから（確定後は元に戻してから）、そのキーを今の状態でふつうに扱う |
| フォーカスが外れた（IMK の `commitComposition`） | 確定後は変換中の文字列（＝元の表記）が確定されるだけで、文書は変わらない（ADR 0006 §9.5 と同じ） |

- 単位は、カーソル（確定後はキャレット、ほかは末尾）から外へ向かって作る。左へは境界を 1 つずつたどり、最小文字数に届くか文頭に当たるまで合わせる。右も同じ（文末）。カーソルが文節の途中なら、左右それぞれの最初の塊は「次の境界まで」（D2）。一度作った単位はモードの間は変えない（←/→ で行き来しても同じ範囲に戻る）。
- 確定後に対象を動かすとき: `Restore`（今の対象を元の表記で戻す）→ `SetMarkedText`（次の対象。`replacement_range` = その範囲、`delete_target` = true）。範囲の位置は、前の対象を戻したあとの文書で計算する（戻すので位置はずれない）。

## 4. 削除の単位の規則（`shell-core/src/clause.rs` の純粋な関数）

```text
入力: text（文、表示の文字列）、bounds（core の文節の境界。text の書記素の位置の昇順）、
      cursor（text の中の書記素の位置）、min（clause.min_chars）、dir（Left | Right）
1. 境界の列を B とする（0 と len(text) を含む）。cursor が境界でなければ、それも列に入れる（途中のカーソル）
2. Left: start = cursor。B の中で start より小さい最大の境界へ動かすのを、
   (cursor − start) >= min になるか start = 0（文頭）になるまで繰り返す
   Right: 対称（end = len(text) が文末）
3. 返すのは [start, cursor)（Right は [cursor, end)）
```

- 例: 「俺は｜正しい」、Left → 「俺は」（2 < 3 だが文頭）。「俺は正しい｜」、Left → 「正しい」。
- 例: 「今日は雨が｜降る」（区切り 今日は／雨が／降る）、Left → 「雨が」（2 < 3）→ 「今日は雨が」（5）。
- 例: 「正し｜い」（文節の途中）、Left → 「正し」（2 < 3、文頭）。Right → 「い」（1 < 3、文末）。
- 数える単位は書記素（Swift の `String.count` と同じ。`grapheme_count`）。範囲を返すときに UTF-16 に変える。
- 文節のモードのフォーカスの削除（§3.2）はこの関数を通さない。

## 5. 契約の変更（シグネチャ）

### 5.1 shell-core: キー割り当てと設定

```rust
// shell-core/src/keymap.rs
pub enum KeymapAction {
    // …今の 15 個…
    /// 直前の文節をすぐ消す（ADR 0050）。既定は `option+backspace`
    ClauseDelete,
    /// 削除の対象をハイライトするモードに入る（ADR 0050）。既定は `shift+option+backspace`
    ClauseDeletePreview,
}
// ALL は 17 個。name(): "clause.delete"、"clause.delete_preview"
```

- 3 つのプリセットの TSV に `option+backspace	clause.delete` と `shift+option+backspace	clause.delete_preview` を足す。Swift の `allCases`（設定画面の並び）と `settings/defaults.rs` の `KEYMAP_ACTIONS` も同じ順で足す。
- 設定 `clause.min_chars`（`settings/defaults.rs` に `KEY_CLAUSE_MIN_CHARS`、`values.rs` に整数の値）。既定 3（ユーザー決定）。範囲の案は 1〜10（1 = 下限なし。§12 の確認事項）。`docs/phase2-contract.md` §3 に足す。
- `dispatch` の中の扱い: `bound_action` が 2 つの操作を返したら、`is_script_conversion` と同じ並びで、状態ごとの関数（§3）へ。英語のモードと secure では割り当てを引く前に今どおり素通しにする（新しい分岐を足さない。どこで素通しにしているかを実装で確かめる）。

### 5.2 shell-core: 型と動作（`types.rs`、`shell.proto`）

```rust
pub struct SurroundingText {
    // …今の 3 項目…
    /// キャレット（選択）の範囲（UTF-16。`client.selectedRange()`）。取れなければ None。
    /// 文脈（Prefetch・Convert の `SurroundingText`）には送らない（engine.proto は変えない）
    pub caret: Option<Utf16Range>,
}

pub struct MarkedText {
    // …今の項目…
    /// フォーカスしている文節（`focused`）を削除の対象として見せる（ADR 0050 §3.4）。既定 false
    pub delete_target: bool,
}

pub enum ShellAction {
    // …
    /// 確定済みの文字列の範囲を消す（ADR 0050 §3.3）。変換中の文字列がないときだけ返す。
    /// macOS は `client.insertText("", replacementRange: range)` を 1 回
    DeleteText { range: Utf16Range },
}
```

- `DeleteText` を `Restore { text: "" }` で代用しない理由: `Restore` は「変換中の範囲を戻す」で、Swift が `markedRange` と比べてログを出す。意味の違うものを同じ動作に載せると、Swift の側で状態を推し量る判断が要る。動作を 1 つ足しても Swift の実装は 1 行。
- プレビューで対象を消すとき（変換中の文字列がある）は `Restore { text: "" }` を使う（意味が合う）。
- `TextAccess` の trait は変えない（`surrounding()` の戻り値が 1 項目増えるだけ）。

### 5.3 engine と core: 変換中の文節を取り除く

```proto
// engine.proto
message RemoveSegments { uint64 conversion_id = 1; uint32 layout_rev = 2; uint32 first = 3; uint32 count = 4; }
// 返事は SegmentLayout（layout_rev を 1 進める。focused は first − 1、first が 0 なら 0）
```

```rust
// core/src/converter/segment.rs
impl Segmented {
    /// 文節 [first, first + count) を取り除く。残りの文節の候補の列（`lists`）と 0 番（`heads`）は保つ。
    /// 全部を取り除く指定は `SegmentError::Empty`（殻は Cancel して idle に戻る）
    pub fn remove(&mut self, first: usize, count: usize) -> Result<(), SegmentError>;
}
```

- `whole`（読み全体の変換）は古くなるが、文節のモードの確定は `CommitSegments` なので使わない。まだ作っていない文節の候補の列は、取り除いたあとの並びで作る。
- 殻は、手元の `segments` からも同じ範囲を除き、`layout_rev` を返事のものに替える。`StaleLayout` が返ったら今の規則（ADR 0006 §5.2）に従う。

### 5.4 確定後の区切り: `Reconvert` の流用（新しい要求は作らない）

- `Reconvert { session_id, surface: 切り出した文, context: 文の左 }` → 返事の `layout.segments[].surface` の長さを境界にする → すぐに `Cancel { conversion_id }`。
- 区切りの材料は、確定履歴（`pieces_from_history`。ikari が確定した直後なら、そのときの文節どおり）→ 表記の解析の順（ADR 0006 §9.3）。
- `Empty`（英字だけなど、全部が読めない）は `PassThrough`（アプリの単語の削除が働く。英語の文にはこちらが合う）。
- 診断（diag）では、この往復を再変換に数えない（別の印を付ける）。

## 6. 検討した案

### 6.1 プレビューの見せ方（確定後）

- **案 A（採用。D7）**: 対象の単位だけを変換中の文字列にする（再変換の始まりと同じ）。良い点: 新しい UI が要らない。再変換と同じアプリで動き、同じ拒否リストが使える。見たままの位置にハイライトが出る。悪い点: 対象を動かすたびに元の表記を書き戻す（リッチテキストの書式が落ちうる、アプリの「元に戻す」の履歴に残りうる）。S2 で確かめる。
- **案 B**: 文書には触らず、候補ウィンドウと同じ殻の UI に「…俺は［正しい］」のように文を出す。良い点: 文書を書き換えないので、書式と「元に戻す」に影響しない。悪い点: 新しい UI。見ている位置と別の場所に出る。S2 で案 A の害が大きければこちらに替える（§12 の Q2）。
- **案 C**: アプリの選択（青い反転）を対象に合わせる。IMK に選択を変える API がないので採らない。

### 6.2 確定後の区切りの求め方

- **案 A（採用。D4）**: `Reconvert` を流用して `Cancel`。契約を増やさない。確定履歴を使うので、ikari の確定の直後は文節が正確。
- **案 B**: 区切りだけを返す新しい要求（`SplitClauses`）。意味がきれいで、エンジンに変換の状態を残さない。今は要求 1 つと `Cancel` で足りるので YAGNI。流用が性能や診断で問題になったら替える。
- **案 C**: 殻だけで字種の境目（漢字 → ひらがな）から区切る。往復がないが、ひらがなの続く文で崩れ、変換の文節と食い違う。採らない（区切りの判断が 2 か所になる）。

### 6.3 読みの入力中の区切り

- **案 A（採用）**: `Convert` の返事の区切りを読み、`Cancel`。Space を押したときに見える区切りと同じで、先行計算に当たれば待たない。
- **案 B**: 辞書だけの最良の経路で区切る新しい要求。LM を待たないが、Space のあとの区切りと食い違いうる。採らない。

### 6.4 変換中の削除

- **案 A（採用）**: エンジンに `RemoveSegments`。残りの文節の選択を保てる。
- **案 B**: 殻だけで文節を除き、残りの読みを読みの入力中に戻す。エンジンを変えないが、選んだ候補が全部消える。採らない。
- **案 C**: 残りの読みで変換し直す。LM の往復が要り、選んだ候補が消える。採らない。

### 6.5 プレビューを「押している間」にする

- 修飾キーを離したら消す（#193 の最初の案）には `flagsChanged` が要り、Swift の入口を広げる。2026-11-05 の決定（←/→ で選び、Delete で消す）はモードで足りるので採らない（D6）。離したら抜けることを望むなら、S4 を足して改める。

### 6.6 消せないときに `Consume` する

- 何も起きないので「ikari が奪った」ことは分かるが、アプリの単語の削除も使えなくなる。D5 の `PassThrough` は、ikari の削除とアプリの単語の削除が、アプリによって入れ替わる（§11 の懸念 1）。§12 の Q1。

## 7. 断る条件（確定後。どれかに当たれば `PassThrough`）

| 条件 | 理由 |
|---|---|
| secure、英語のモード | 今の規則（素通し） |
| 周辺テキストが取れない、`caret` が None | 範囲を作れない（ターミナル、一部の Electron） |
| 選択が空でない | アプリが選択を消すのが自然 |
| 拒否リスト（ADR 0006 §9.5 の定数）に入っているアプリ | `replacementRange` が効かない、または位置がずれる |
| `caret.location` と `left` の長さが合わない（`left` が上限で切られていないのに、`left` の UTF-16 の長さ ≠ `caret.location`） | 相対位置などを返すアプリで、違う場所を消さないため |
| 切り出した文が空、または改行の直後でキャレットの直前に字がない | 消すものがない（アプリの Option+Delete は改行をまたいで消す。それに任せる） |
| `Reconvert` が `Empty`、時間切れ、エンジンが落ちた | 区切りがない |

- `replacementRange` を無視するアプリでは、`insertText("")` はキャレットに空を入れるだけで、文書は壊れない。危ないのは**違う位置を消す**アプリで、S1 で拒否リストに入れる。
- 読みの入力中と変換中は、区切りが取れないとき `Consume`（§3.1）。

## 8. 実機のスパイク（捨てるコード。ログを出す検証版と手順書）

| # | 確かめること | 決まるもの |
|---|---|---|
| S1 | アプリごとに、(a) Option+Delete と Shift+Option+Delete が IMK の keyDown に届くか、(b) `selectedRange` の位置と `surroundingText` の left が合うか、(c) `insertText("", replacementRange:)` が正しい範囲を 1 回の「元に戻す」で消すか。対象: TextEdit、メモ、Pages、Safari、Chrome、VS Code、Slack、Notion、Xcode、ターミナル、iTerm2 | 拒否リストの追加。§7 の位置の検査で足りるか |
| S2 | 確定済みの単位を変換中の文字列にして戻す（案 A）を、←/→ で 10 回動かしてから Esc したとき、書式（太字、リンク）と「元に戻す」の履歴がどうなるか。Pages、メモ、Notion、Google ドキュメント、Slack | プレビューの案（A のままか B に替えるか。§12 の Q2） |
| S3 | `insertText("", replacementRange: {位置, 0})` でキャレットが動くか（D10 の移動のため。この ADR では実装しない） | 確定後の文節の移動の ADR を書けるか |
| S4（要るときだけ） | Shift+Option を離したことを `flagsChanged` で取れるか | §6.5 を改めるときだけ |

- S1 は確定後の削除（レーン A の idle の部分）のマージの前に要る。読みの入力中と変換中の削除は S1 を待たない。
- smoke と同じく、検証版の差し替えはユーザーの了解を取ってから行う（実機の ikari を入れ替えるため）。

## 9. シナリオの先書き（`shell-core/scenarios/clause-delete.json`）

契約のコミットで、次の場面を書く（エンジンの返事は `reconversion.json` と同じ形で書く）。実装の前は失敗してよい（`ignore` の印の付け方は既存のシナリオの形に従う）。

1. idle: 「俺は正しい｜」→ Option+Delete → `DeleteText`（「正しい」の範囲）。
2. idle: 「俺は｜正しい」→ Option+Delete →「俺は」を消す（文頭の例外）。
3. idle: 「今日は雨が｜降る」→「今日は雨が」（最小文字数で広げる）。`clause.min_chars = 1` なら「雨が」。
4. idle: 文節の途中「正し｜い」→「正し」。
5. idle: 「…ました。｜」→「ました。」。
6. idle の断り: 選択あり、周辺テキストなし、`caret` なし、拒否リスト、secure、`Reconvert` が `Empty`、時間切れ → すべて `PassThrough`。
7. 読みの入力中: 「きょうはあめがふる」→ 末尾の単位を消す。区切りが 1 つなら idle へ。時間切れは `Consume`。
8. 変換中（全体の候補）: Option+Delete → 文節のモードで末尾の単位を `RemoveSegments`。全部消えたら idle。
9. 変換中（文節のモード）: フォーカスの文節を消す（最小文字数で広げない）。残りの選択が保たれる。
10. プレビュー（確定後）: 入る → ← → → → Backspace。Esc で `Restore`（元の表記）。文字キーで戻してから合成を始める。
11. プレビュー（読みの入力中、変換中）: `delete_target` の付いた `SetMarkedText`、Backspace で消す。
12. キー割り当て: `keymap.clause.delete = none` のとき Option+Delete は今どおり（idle は素通し）。別のキーに替えられる。

## 10. 実装の進め方（レーンとファイルの境界）

1. **契約のコミット**（リードまたは code-editor 1 つ）: §5 の型と名前、proto（`shell.proto` の `DeleteText`・`SurroundingText.caret`・`MarkedText.delete_target`、`engine.proto` の `RemoveSegments`）、設定のキーと既定、プリセット 3 つ、Swift の `allCases`、エンジンの `RemoveSegments` の受け口のスタブ（`BAD_REQUEST`）、§9 のシナリオ、`docs/phase2-contract.md`。既定の動きは今と同じ（`dispatch` はまだ 2 つの操作を扱わない）。
2. **並行実装**（契約は変えない）:

| レーン | 持つファイル | 中身 |
|---|---|---|
| A（shell-core。Sonnet、つまずいたら Opus） | `shell-core/src/clause.rs`（新規）、`shell-core/src/machine/*`、`shell-core/scenarios/clause-delete.json` | §3・§4・§7。単位の関数の単体テストを先に。idle の部分は S1 の結果（拒否リスト）を待ってマージ |
| B（core と engine） | `core/src/converter/segment.rs`、`engine/src/` の要求の受け口とそのテスト | §5.3 の `Segmented::remove` と `RemoveSegments` |
| C（Swift。入口と UI だけ） | `macos/Sources/IkariIME/ClientAccess.swift`、`ShellDriver.swift`、`MarkedTextAttributes.swift`、設定画面のキー割り当てと最小文字数の項目 | `DeleteText` の実行、`caret` を埋める、`delete_target` の見た目（UI。打ち消し線か背景色かは C で決めて、スクリーンショットで確かめる） |
| S（スパイク。先に） | 使い捨てのブランチ | §8 の S1・S2 |

3. **統合**: `scripts/check.sh`（フル）、シナリオ、`swift test`。実機の確認はユーザーの了解を取ってから。

## 11. 懸念点

1. **D5 の副作用**: 消せないアプリでは、同じ Option+Delete がアプリの単語の削除になる。利用者からは「アプリによって消える範囲が違う」と見える。代わりに、ターミナルなどで今の Option+Delete を失わない。
2. **区切りの質**: ikari が確定した直後でない文（貼り付けた文、ほかの IME で打った文）は、表記の解析で区切る。再変換の読みの一致率（ADR 0006 §16.5。97.5%）と同じ程度の精度で、漢字の続く複合語などで崩れうる。
3. **全体の候補で 0 番以外を選んでいるとき**: 文節のモードに入ると表示が区切りの表記に替わる（← と同じ）。消す前に表示が変わって見える。← と揃えるのを優先した。
4. **値**: `clause.min_chars` の範囲（1〜10）は案。`Reconvert` の流用の待ち時間は再変換と同じ上限を使い、新しい値を足さない。
5. **ADR 0006 §16 の表との関係**: 文節のモードのプレビューの中だけ、Backspace の意味が「読みに戻る」から「文節を消す」に替わる。モードの外の Backspace は変えない。

## 12. ユーザーに聞くこと（推奨つき）

- **Q1. 確定後に消せないアプリ（ターミナル、拒否リスト、英字だけの文など）での Option+Delete**: (a) 奪わずにアプリへ渡す（アプリの単語の削除が働く）、(b) 奪って何もしない。**推奨は (a)**。ターミナルや英語の文で、今の Option+Delete を失わないため。
- **Q2. 確定後のプレビューの見せ方**: (a) 対象の単位を変換中の文字列にして、文の中でハイライトする（再変換と同じ手段）、(b) 文書に触らず、候補ウィンドウのような小窓に文を出して対象を示す。**推奨は (a)**。ただしスパイク S2 で書式や「元に戻す」の履歴が崩れるアプリが多ければ (b) に替える。
- **Q3. 確定後の「文節ごとの移動」（#193 の要件 3）**: この ADR からは外し、スパイク S3（キャレットを動かせるか）の結果を見て別の ADR にする。それまで移動のキーは足さない。**推奨はこのとおり分ける**（削除だけ先に使える）。
- 確認だけ: 最小文字数の設定の範囲を 1〜10（1 = 下限なし）にしてよいか。
