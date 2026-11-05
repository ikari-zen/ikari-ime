# 共通のシナリオ（JSON）

殻の振る舞いを、OS と言語に依存しない形で書いたもの（ADR 0041 §5。`docs/adr/0041-shell-state-machine-in-rust.md`。PR #136）。
同じファイルを次の 2 つが読む（移行の間は Swift の実行器も読んでいたが、M5 で Swift の状態機械と一緒に消した）。

| 読み手 | 場所 | いつまで |
| --- | --- | --- |
| Rust の実行器 | `shell-core/tests/scenarios.rs`（`cargo test -p ikari-shell-core`） | ずっと。照合の `scenarios_match_on_rust`（全件）と形の検査 `scenarios_are_well_formed` |
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
      "rust_only": "（省略可）Rust の実行器だけで流す理由",
      "pending": "（省略可）まだ通らない理由。形の検査だけして照合を飛ばす。実装するレーンが外す",
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
- `pages` の各ページに `layout`（`ConvertReply.layout`。下の区切りの形）を付けられる。`convert` の返事だけに付ける。
- `segment_pages`: `[{"segment": 1, "page_index": 0, "candidates": [...], "has_next": false}]`。`segment_page` の要求の `segment` と `index` が合うものを返す。
- `resizes`: `[{"segment": 0, "delta": -1, "layout_rev": 0, "layout": {...}}]`。`resize_segment` の要求の `segment`、`delta`（と、書けば `layout_rev`）が合うものを返す。
- `reconvert`: 区切り 1 つ。`reconvert` の返事。
- 区切りの形: `{"conversion_id": 11, "layout_rev": 0, "segments": [{"reading": "はじめて", "surface": "初めて"}], "focused": 0, "focused_page": {"page_index": 0, "candidates": [...], "has_next": true}}`（`layout_rev`、`focused`、`focused_page` は省略可）。
- `emoji_table`: `[{"name": "eyes", "emoji": "👀"}]`（名前の UTF-8 のバイト順で昇順。エンジンの検証と同じ）。Rust は `Activate` で取り、Swift は表を状態機械に渡す。
- `postal`: `{"1000001": ["東京都千代田区千代田"]}`。`postal_lookup` の返事（なければ空）。
- 書いていない要求は `{"engine": "bad_request"}`（`postal_lookup` は空、`hello` は成功）。
- `failures`: `[{"request": "page", "nth": 1, "error": "timeout"}]` のように、ある種類の要求の `nth` 回目（1 から）だけを失敗させる。`error` は `"unavailable"`、`"timeout"`、`"invalid_reply"`、`{"engine": "<ErrorCode の snake_case>"}`。返事を待つ要求（`convert`、`page`、`segment_page`、`resize_segment`、`reconvert`、`postal_lookup`）に使う。

### 周辺テキスト（`surrounding`）
`{"left": …, "right": …, "document_length": …, "available": …}`。`available: false` はホストが取れなかった（`TextAccess::surrounding` が `None`）。`document_length` を省けば `left` と `right` の長さの和。
「左が空で文書の長さも 0 なら取れていない」の判定と、2000 文字への切り詰めは殻（`shell-core`）が行う。

### 手順（`steps`）
1 つの手順に、`event`・`press`・`type_text`・`host` のどれか 1 つを書く。

| 欄 | 中身 |
| --- | --- |
| `event` | `ShellEvent` をそのまま（例: `{"type": "key", "key": {"kind": "space"}, "modifiers": {"shift": true}}`）。Swift の実行器はキーだけを扱う |
| `press` | キー 1 つ。`修飾+…+キー`。修飾は `shift`、`ctrl`、`option`、`cmd`。キーは `space`、`enter`、`escape`、`backspace`、`left`、`right`、`up`、`down`、`other`、`f1`〜`f20`、`jis:<番号>`、それ以外は 1 文字（例: `shift+A`、`cmd+c`、`ctrl+;`、`2`） |
| `type_text` | 文字ごとに、修飾なしの文字のキーを打つ |
| `surrounding` | この手順から先の周辺テキスト（次に書くまで続く） |
| `selection` | この手順だけの選択範囲 `{"range": {"location", "length"}, "text"}`（省けば選択なし） |
| `expect` | この手順のイベントが返した `ShellAction` を**つないだ列**。順序つきで完全一致。省けば比べない |
| `engine` | この手順でエンジンへ送った要求の列。`type` と書いた欄だけを比べ、列の長さは一致させる。`[]` は「何も送らない」。省けば比べない |
| `host` | `ShellEvent` で表せない OS とのやりとり（`Shell` の公開の関数）。`{"type": "input_mode_changed", "mode_id": …}`、`{"type": "commit_composition"}`、`{"type": "absorb_mode_key"}`、`{"type": "panel_shown", "side": "above"}`、`{"type": "panel_hidden"}`。Swift の実行器は `commit_composition`・`panel_shown`・`panel_hidden` だけを扱う |
| `mode_request` | この手順のあとに `Shell::take_mode_request` が返すモード（`japanese`、`english`、`none`）。省けば比べない |
| `note` | 人のための注釈 |

`ShellAction` の書き方は `types.rs` の serde の形（`{"type": "set_marked_text", "text": "ひんし", "highlighted_length": 3}`、`{"type": "commit", "text": "品詞"}` など）。既定値の欄（`highlighted_length: 0`、`underlined: true`、候補の `kind: "dictionary"` など）は省く。

### エンジンへの要求の書き方
`type` は `engine.proto` の `Envelope.body` の欄の名前（`prefetch`、`convert`、`page`、`commit`、`commit_text`、`cancel`、`segment_page`、`resize_segment`、`remove_segments`、`commit_segments`、`reconvert`、`shutdown`、`postal_lookup`、`hello`、`begin_session`、`end_session`、管理の中継は `relay`）。欄の名前も `engine.proto` に合わせる（`reading`、`page_size`、`conversion_id`、`candidate_index`、`context: {left, right, available}` など）。ほかに `timeout_ms`（返事を待つ要求の待ちの上限）を比べられる。
- `session_id` と `context_rev` は実行器ごとに値が違うので、シナリオに書かない。
- `diag` と `emoji_table` は比べない（片道・非同期で、順が決まらない）。`prefetch` は `page_size` を持たない（`engine.proto` に合わせる）。

### Rust だけで流すシナリオ（`rust_only`）
（移行の間の印。Swift の実行器は M5 で消したので、今はすべて Rust の実行器で流れる。印と理由は記録として残す）
殻の判断（`IkariInputController`・`AppContext` の、セッション、英語のモード、モードの切り替え、メニューの命令）は、Swift では
`IkariIME`（AppKit）にあって、テストから呼べない。これらは `rust_only` に理由を書き、Swift の実行器は飛ばす。
Swift の替え玉が本物と違うもの（かな入力の合成器が文字を受け取る、キーリピートを渡せない）も同じ扱いにする。

## 期待の書き方（記録）
1. 期待を省いたシナリオ（`{"name": …, "steps": […]}`）を、`{"<ファイル名>": ScenarioFile}` の形の JSON に書く。
2. `IKARI_SCENARIO_RECORD_IN=<その JSON> IKARI_SCENARIO_RECORD_OUT=<出力> cargo test -p ikari-shell-core --test scenarios record_expectations -- --ignored`
   で、手順ごとの動作とエンジンへの送信（`session_id`、`context_rev` を除く）を書き出す（`EngineLink` が要る）。
3. 期待を埋めてファイルに置き、記録した動作が意図どおりかを読んで確かめる（移行の間は、Swift の実行器が通ることを今の振る舞いと同じである証拠にしていた。M5 で Swift の実行器を消したので、今は Rust の実行器だけ）。

## 書くときの約束
- 移行の間に書いたシナリオは、Swift の替え玉の合成器と本物の合成器（`ikari_input`）で同じ結果になる綴りと、F6〜F8 だけを使っている（Swift の実行器は M5 で消した。今は本物の合成器だけなので、この制約はない）。
- 振る舞いを変えたら、まずシナリオを足すか直す（ADR 0041 §8）。
- 期待は、今の振る舞いを確かめてから書く。

## 今あるもの（M1-a、L-A2）
| ファイル | 中身 | 件数 |
| --- | --- | --- |
| `composing.json` | 合成中のキー | 10 |
| `conversion.json` | 変換中の Space、ページ送り、数字、Esc / BackSpace、確定 | 24 |
| `script-keys.json` | 字種変換のキー（F6〜F8、プリセットの Ctrl のキー、上書き） | 15 |
| `ascii-run.json` | Shift＋英字の一時的な英数 | 7 |
| `keymap-presets.json` | キー割り当てのプリセット | 2 |
| `segments.json` | 文節の移動、区切り直し、文節ごとの選択と確定（ADR 0006 §8.2、§16） | 32 |
| `reconversion.json` | 再変換と、Esc で元に戻す（ADR 0006 §9、§16.3）。1 件は `rust_only` | 27 |
| `candidate-window.json` | ↑↓、3 件から 9 件への展開、番号キー、上に出たときの向き（ADR 0014） | 15 |
| `confident.json` | 確信が高い変換の 1 位だけの表示（#69） | 11 |
| `trailing-n.json` | 末尾の n を「ん」にする（FR-07） | 6 |
| `shortcode.json` | 日本語のモードの絵文字のショートコード（ADR 0027）。1 件は `rust_only` | 10 |
| `shell.json` | 殻の判断（セッション、英語のモード、モードの切り替え、メニュー）。全部 `rust_only` | 18 |
| `clause-delete.json` | 文節ごとの削除（ADR 0050 §9）。先書き。`pending` の印を、レーン A が外す | 24 |
