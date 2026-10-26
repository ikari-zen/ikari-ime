# sudachi-mini

ikari-ime のテスト用の小さな辞書 fixture（ADR 0001 §10.2）。CI で実辞書なしに、辞書のビルドと読み込み、変換の統合テストを動かすために使う。

## 出典とライセンス

- 出典: SudachiDict Core 20260723（`small_lex.csv`, `core_lex.csv`, `matrix.def`）
- 提供: Works Applications Co., Ltd.（https://github.com/WorksApplications/SudachiDict）
- ライセンス: Apache License 2.0
- 同梱する `lex.csv` の各行は、上記の辞書から**表記・読み・品詞・コストを変えずに**抜き出したもの。変更したのは左ID・右ID の2列だけ（下記）。`matrix.def` は実際の `matrix.def` の値を新しい ID 空間に写したもの。
- NOTICE 用の記載（配布物に含める場合）:

  > This product includes data derived from SudachiDict (https://github.com/WorksApplications/SudachiDict),
  > Copyright (c) Works Applications Co., Ltd., licensed under the Apache License, Version 2.0.
  > The entries in testdata/sudachi-mini were extracted from SudachiDict Core 20260723 and their
  > left/right connection IDs were remapped to a smaller ID space.

ライセンスの確認は人間が行う（spec の方針）。この fixture を含めてよいことはユーザーが確認済み。

## ファイル

| ファイル | 内容 |
| --- | --- |
| `lex.csv` | Sudachi と同じ19列の形式。218行（extract.py の140行 + extend.py の追加行） |
| `extend.py` | `lex.csv` に ADR 0002 §18 の行を**追加だけ**するスクリプト（下記） |
| `matrix.def` | 24 x 24 の全組み合わせ（`24 24` + 576行） |
| `extract.py` | 上の2ファイルを実データから作るスクリプト（標準ライブラリのみ） |

## 含まれる語

評価セット（ADR 0001 §6.1 の14件）の読みに関わる行。

- ひんし: 品詞、瀕死、浜死、賓師、ひん死、動詞（瀕し、貧し、ひんし）
- こうせい / かいとう: 構成、校正、更正、更生、厚生、公正、後世 / 回答、解答、解凍、回頭、会頭、怪盗
- しよう: 為る の意志形（しよう、為よう、仕よう）、仕様、使用、私用、試用、ほか
- の、この、は、に、と、が、を（助詞、連体詞「この」を含む）
- なった / 鳴った: 成っ、鳴っ、為っ、なっ、綯っ、生っ（読み「ナッ」）と、助動詞「た」
- 書き進める: 書き（動詞連用形、名詞、接尾辞）、書、かき、進める、進め、すすめる、勧める
- へんかん / する: 変換、返還、へんかん、為る、する の各活用形
- builder の除外規則の確認用: 読点（補助記号）、空白、左ID が -1 の行、読みに英字を含む行（`profile`）

fixture での順位には意味がない。**候補に出るかどうかと、形式の往復だけを見る**。

## v2 での追加行（extend.py）

既存の行は消さず、末尾に追加する（ほかのテストが既存の行に依存しているため）。

- ポリシー用: コウ（表記ゆれ）、セイ・ナッタ（人名）、4（数詞）、relax・hennkann（英小文字）、iPhone、スズメ（lemma 雀）、ユーザ/ユーザー、ラーメンなど
- 活用規則用: 下一段-マ行（書き進める、読み進める、受け止める、見つめる）、五段-ラ行の一般（乗る、取る、走る、鳴る、なる）、五段-ラ行の非自立可能（成る、終わる、ある、おる）の一族。終止形・連体形・連用形・未然形
- 追加行の左右ID は既存の粗いクラスの ID。辞書形ID（13列目）は、基準形の行がこの fixture にある場合だけ fixture の行番号（0始まり）に付け替える。既存の行の辞書形ID は元の辞書の行番号のまま（fixture の行を指さないので無害）
- `python3 testdata/sudachi-mini/extract.py <raw>` のあとに `python3 testdata/sudachi-mini/extend.py <raw>` の順で実行する。extend.py は同じ内容の行があれば足さない

## 抜き出しと付け替えの手順

```
python3 testdata/sudachi-mini/extract.py data/raw/sudachi-20260723
```

1. `extract.py` の `SELECTIONS` の規則で行を選ぶ。読み（カタカナ）ごとに、必ず含める表記と、品詞が重ならないコストの低い行を最大行数まで。左ID が -1 の行と、表記が ASCII だけの行は通常の選択からは除く（`EXTRA_FILTER_ROWS` で、除外規則の確認用に数行だけ別に入れる）。
2. 行の品詞を粗いクラス（例: `名詞-普通名詞`、`動詞-連用`、`助詞-格助詞`）にまとめ、クラス名の昇順に ID 1..23 を振る。**ID 0 は BOS/EOS**。左ID と右ID には同じクラス ID を入れる（左ID が -1 の行は -1 のまま）。
3. `matrix.def` は 24 x 24 の全組み合わせ。クラスごとに「そのクラスの行で最も多い元の左ID／右ID」を代表として選び、実際の `matrix.def` の `(代表の元の右ID, 代表の元の左ID)` の値をそのまま写す。BOS/EOS（ID 0）は元の ID 0。
   - 行は `a b cost`（前の語の右ID a、次の語の左ID b）。MeCab/Sudachi と同じ規約。
4. 出力は決定的（同じ入力から同じファイル）。

ID 空間は ADR の目安（0..16 程度）より少し大きい 24 になった。品詞クラスを粗くまとめすぎないことを優先した。

## 使い方

```
cargo run -p ikari-dict-builder -- \
  --lex testdata/sudachi-mini/lex.csv --matrix testdata/sudachi-mini/matrix.def \
  --out /tmp/mini-dict --label sudachi-mini
```
