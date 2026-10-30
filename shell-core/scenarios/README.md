# 共通のシナリオ（JSON）

殻の振る舞いを、OS と言語に依存しない形で書いたもの（ADR 0041 §5。`docs/adr/0041-shell-state-machine-in-rust.md`。PR #136）。
同じファイルを次の 3 つが読む。

| 読み手 | 場所 | いつまで |
| --- | --- | --- |
| Rust の実行器 | `shell-core/tests/scenarios.rs`（`cargo test -p ikari-shell-core`） | ずっと。M1-a では `Shell` が stub なので、照合の `scenarios_match_on_rust` は `#[ignore]`。形の検査 `scenarios_are_well_formed` は流す |
| Swift の実行器 | `macos/Tests/IkariShellCoreTests/ScenarioRunnerTests.swift`（`swift test`） | 移行の間だけ（M5 で消す）。今の Swift の状態機械で全部通ること |
| 性能の計測 | `cargo xtask perf shell`（L-P） | ずっと |

形の正は Rust の serde の型（`shell-core/src/scenario.rs` と `types.rs`）。知らない欄は誤りにする（綴りの誤りで黙って通らないように）。

## ファイルの形

```json
{
  "description": "何のシナリオか",
  "defaults": { "settings": [...], "client": {...}, "engine": {...}, "surrounding": {...} },
  "scenarios": [
    {
      "name": "ファイルの中で一意の名前",
      "from": "移した元のテスト（ファイル名 型名.関数名）",
      "setup": { "settings": ["paging.size\t2"] },
      "steps": [
        {"type_text": "hinsi"},
        {"press": "space", "expect": [...], "engine": [...]}
      ]
    }
  ]
}
```

### 準備（`defaults` と `setup`）
シナリオの `setup` は、ファイルの `defaults` を**欄ごとに**上書きする（`engine` を書けば、`engine` 全体が置き換わる）。どちらにもない欄は次の既定。

| 欄 | 中身 | 既定 |
| --- | --- | --- |
| `settings` | `settings.tsv` の行（`key<TAB>value`。JSON では `\t`） | 空（すべて既定値） |
| `client` | `{"bundle_id": …, "secure": …}`。実行器は最初に `Activate` を送り、そこまでのエンジンへの送信は比べない。`secure` は `press`・`type_text` のキーに入る | `com.apple.TextEdit`、false |
| `engine` | 替え玉のエンジンの台本（下） | ページなし、失敗なし |
| `surrounding` | 最初の周辺テキスト（下） | 取れない |

### 替え玉のエンジン（`engine`）
- `pages`: `convert` は `pages[0]`、`page`（`index` = i）は `pages[i]` を返す。ないときは `{"engine": "bad_request"}`。各ページは `conversion_id`、`page_index`、`candidates`、`has_next`（`lm_applied`、`confident` は省けば false）。
- `segment_page`・`resize_segment`・`reconvert` は `{"engine": "bad_request"}`、`postal_lookup` は空、`hello` は成功。文節と再変換の台本は、それらのシナリオを移すときに欄を足す。
- `failures`: `[{"request": "page", "nth": 1, "error": "timeout"}]` のように、ある種類の要求の `nth` 回目（1 から）だけを失敗させる。`error` は `"unavailable"`、`"timeout"`、`"invalid_reply"`、`{"engine": "<ErrorCode の snake_case>"}`。返事を待つ要求（`convert`、`page`、`segment_page`、`resize_segment`、`reconvert`、`postal_lookup`）に使う。

### 周辺テキスト（`surrounding`）
`{"left": …, "right": …, "document_length": …, "available": …}`。`available: false` はホストが取れなかった（`TextAccess::surrounding` が `None`）。`document_length` を省けば `left` と `right` の長さの和。
「左が空で文書の長さも 0 なら取れていない」の判定と、2000 文字への切り詰めは殻（`shell-core`）が行う。

### 手順（`steps`）
1 つの手順に、`event`・`press`・`type_text` のどれか 1 つを書く。

| 欄 | 中身 |
| --- | --- |
| `event` | `ShellEvent` をそのまま（例: `{"type": "key", "key": {"kind": "space"}, "modifiers": {"shift": true}}`）。Swift の実行器はキーだけを扱う |
| `press` | キー 1 つ。`修飾+…+キー`。修飾は `shift`、`ctrl`、`option`、`cmd`。キーは `space`、`enter`、`escape`、`backspace`、`left`、`right`、`up`、`down`、`other`、`f1`〜`f20`、`jis:<番号>`、それ以外は 1 文字（例: `shift+A`、`cmd+c`、`ctrl+;`、`2`） |
| `type_text` | 文字ごとに、修飾なしの文字のキーを打つ |
| `surrounding` | この手順から先の周辺テキスト（次に書くまで続く） |
| `selection` | この手順だけの選択範囲 `{"range": {"location", "length"}, "text"}`（省けば選択なし） |
| `expect` | この手順のイベントが返した `ShellAction` を**つないだ列**。順序つきで完全一致。省けば比べない |
| `engine` | この手順でエンジンへ送った要求の列。`type` と書いた欄だけを比べ、列の長さは一致させる。`[]` は「何も送らない」。省けば比べない |
| `note` | 人のための注釈 |

`ShellAction` の書き方は `types.rs` の serde の形（`{"type": "set_marked_text", "text": "ひんし", "highlighted_length": 3}`、`{"type": "commit", "text": "品詞"}` など）。既定値の欄（`highlighted_length: 0`、`underlined: true`、候補の `kind: "dictionary"` など）は省く。

### エンジンへの要求の書き方
`type` は `engine.proto` の `Envelope.body` の欄の名前（`prefetch`、`convert`、`page`、`commit`、`commit_text`、`cancel`、`segment_page`、`resize_segment`、`commit_segments`、`reconvert`、`shutdown`、`postal_lookup`、`hello`、`begin_session`、`end_session`、管理の中継は `relay`）。欄の名前も `engine.proto` に合わせる（`reading`、`page_size`、`conversion_id`、`candidate_index`、`context: {left, right, available}` など）。ほかに `timeout_ms`（返事を待つ要求の待ちの上限）を比べられる。
- `session_id` と `context_rev` は実行器ごとに値が違うので、シナリオに書かない。
- `diag` と `emoji_table` は比べない（片道・非同期で、順が決まらない）。`prefetch` は `page_size` を持たない（`engine.proto` に合わせる）。

## 書くときの約束
- **ローマ字は、Swift の替え玉（`macos/Tests/IkariShellCoreTests/Support/Doubles.swift` の `FakeComposer`）と本物の合成器（`ikari_input`）で同じ結果になる綴りだけを使う**。Swift の実行器は替え玉で、Rust の実行器は本物で流すため。かな入力の `jis:<番号>` も替え玉の表にある番号だけ。字種変換は F6〜F8（ひらがな、全角カタカナ、半角カタカナ）だけを使う（替え玉の F9・F10 は本物と違う）。
- 振る舞いを変えたら、まずシナリオを足すか直す（ADR 0041 §8）。
- 期待は、今の振る舞いを確かめてから書く（Swift の実行器が通ること）。

## 今あるもの（M1-a）
| ファイル | 中身 | 件数 |
| --- | --- | --- |
| `composing.json` | 合成中のキー | 10 |
| `conversion.json` | 変換中の Space、ページ送り、数字、Esc / BackSpace、確定 | 24 |
| `script-keys.json` | 字種変換のキー（F6〜F8、プリセットの Ctrl のキー、上書き） | 15 |
| `ascii-run.json` | Shift＋英字の一時的な英数 | 7 |
