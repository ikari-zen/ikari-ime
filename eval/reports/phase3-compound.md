# 機能語の装飾表記・飾りの記号・漢数字のゼロ・複合語のコスト（2026-10-13）

実機と CLI で見つかった 2 件の修正の前後。ADR 0002 §22.2 に決定を記録した。

- 「ことだよね」の候補に「ことだYOね」「ことだよね〜」「ことだよ〜ね」が上位に出る。
- 「ぐたいれい」が「具体〇」になる（Sudachi の「〇」は読み れい の数詞で、コストが安い）。「具体礼」も「具体例」より上になる。

## 1. 原因

| 候補 | 原因 |
| --- | --- |
| ことだYOね | Sudachi の助詞 `yo`（読み ヨ）。`ascii_other`（3000）だけでは足りない |
| ことだよね〜、ことだよ〜ね | Sudachi の助詞 `ね〜`（読み ネ）、`よ〜`（読み ヨ）。ユーザーが入力していない「〜」を足している（基本原則の1つ目に反する）ので、後ろに回すのでなく変換に使わない |
| 具体〇 | `〇`（名詞,数詞、読み れい、コスト 1386）。`numeric_surface` は算用数字だけが対象だった |
| 具体礼 | `具体例` は 1 語の項目だが、Sudachi は解析のために複合語のコストを高くしている（具体例 15422 > 具体 6762 + 礼 4397）。分割した経路が安い。具体〇 と 具体例 の差は 8491 で、LM の加点の上限 6000 でも覆せない。具体礼 との差は 1708 |

## 2. 変更

1. 固定の規則 `ExclusionRule`（`IME_EXCLUDED`。ポリシーの値ではなくコードに固定。汎用の `exclude` は復活させていない）
   - `wave_dash_without_long_vowel`: 2文字以上で 〜 ～ を含み、読みに「ー」がない。**178 件**
   - `decoration_mark`: 2文字以上で ！？!?。…‥♪♫♬ を含む（固有名詞は対象外）。**3 件**
2. `function_word_notation`（+10000）: 助詞・助動詞・接尾辞の、2文字以上で ASCII の英字か 〜 ～ を含む表記。**9 件**
3. `numeric_surface` に 〇（U+3007）を足した。一、二 などは対象外
4. `frequent_compound`（-8000。chive.tsv）: 名詞,普通名詞で、表記が lemma と同じ、漢字だけの 3 文字以上、lemma の chiVe の順位が 3 万位より上。**2480 件**

## 3. 結果

CLI（`ikari --dict data/build/dict --ranker full`、LM なし）:

| 読み | 前 | 後 |
| --- | --- | --- |
| ぐたいれい | 具体〇 / 具タイ〇 / 具体礼 / 具対〇 / 具体令 / … 具体例は 8 位 | **具体例** / 具体礼 / 具体令 / 具体霊 / 具体れい / 具体零 |
| ことだよね | ことだよね / ことだYOね / ことだよね〜 / ことだよ〜ね / 事だよね | ことだよね / **事だよね** / ことだヨね / 琴だよね / コトだよね |

評価セット（`--ranker full`、LM なし、split=all。1 位正解。`ikari-eval compare` で前後のケース別の順位を比べた）:

| セット | 前 | 後 | 悪化したケース |
| --- | ---: | ---: | ---: |
| homophones | 12 / 14 | 12 / 14 | 0 |
| homophones-ext | 973 / 1266 | 973 / 1266 | 0（1 件は順位が 15 → 14 に改善） |
| hiragana | 135 / 140 | 135 / 140 | 0 |
| katakana-suffix | 114 / 145 | 128 / 145 | 0（改善 16） |
| segments | 83 / 145 | 93 / 145 | 0（改善 11） |
| 区切りの F1（`ikari-eval segments`） | 0.983 | 0.989 | seg-brk-14 が一致に |
| 再変換の読みの推定（`ikari-eval reading`） | 354 / 363 | 354 / 363 | 同じ |

後の split 別: homophones-ext dev 776/1004、test 197/262。katakana-suffix dev 103/115、test 25/30。segments dev 73/118、test 20/27。hiragana dev 111/114、test 24/26。

`frequent_compound` の値の振り（他の述語は固定）: 

| 値 | katakana-suffix | segments | 区切りの F1 | 具体例 |
| ---: | ---: | ---: | ---: | --- |
| -2000 | 122 | 85 | 0.989 | 2 位以下 |
| -3500 | 127 | 88 | 0.989 | 1 位 |
| -5000 | 128 | 91 | 0.989 | 1 位 |
| **-8000** | 128 | 93 | 0.989 | 1 位 |
| -12000 | 130 | 93 | 0.978 | 1 位 |
| -16000 | 130 | 93 | 0.978 | 1 位 |

-12000 から区切りの F1 が下がるので、悪化しない範囲の中で -8000 にした。homophones、ext、hiragana は全値で同じ。
注意: この値の振りは katakana-suffix と segments の全件で見た（dev だけに絞っていない）。どの値でも悪化したケースはない。

## 4. 規則に当たった項目の確認（抜き取り）

- 規則1 の 178 件: 品詞は感動詞 55、副詞 47、普通名詞 20、終助詞 12、形状詞 8 など。ランダム 40 件（ぐ〜んと、ダ〜メ、う〜ん、や〜っと、びみょ〜、そ〜っと、な〜るほど、シュギョ〜 …）はすべて話し言葉の引き伸ばしや飾りで、本物の語は混ざっていない。固有名詞は 2 件（うるさか〜、デ〜ストロ〜ン）で、どちらも飾り。読みに「ー」がある項目（あま〜い/アマーイ など 165 件）は対象外。
- 規則2 の 3 件: シャ…ァァン、な…ん、Buongiorno!。☆印（ほしじるし）は本物の語なので、☆★ は規則から外した。固有名詞の yahoo!、ミニモニ。、ヤフオク! は対象外。
- `function_word_notation` の 9 件: じゃ〜、だ〜、っちゅ〜、nize、ね〜（ない）、m・kg/s2、やす〜い など。
- 名詞の英字表記（PC、API）は下がらない（単体テスト `function_words_with_ascii_or_wave_dash_go_far_back_but_nouns_do_not`）。

## 5. 再現

```sh
scripts/build-dict.sh --rebuild
ikari --dict data/build/dict --ranker full ぐたいれい
cargo run --release -p ikari-eval -- --ranker full --set eval/sets/katakana-suffix.tsv --out-cases /tmp/after.tsv
cargo run --release -p ikari-eval -- segments --ranker full --split all
cargo run --release -p ikari-eval -- reading --ranker full
```

基準の辞書は、ポリシーから `function_word_notation` と `frequent_compound` を除き、固定の規則と 〇 の追加を戻して作る（コミット前の `dict-builder`）。
