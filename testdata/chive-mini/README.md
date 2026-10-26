# chive-mini

vectors.bin のビルドと読み込みのテスト用の、人工の小さな word2vec テキスト（ADR 0002 §18）。chiVe の実データではなく、手で作った値で、ライセンス上の問題はない。

- `vectors.txt`: 20語、8次元。形式は chiVe と同じ（1行目は `語数 次元数`、以降は `語 値 …`）。
- 語のキーは `testdata/sudachi-mini` の辞書の lemma（Sudachi の正規化表記）に合わせてある。
- 話題が5つあり、それぞれ別の次元の組に偏っている（IT: 品詞/変換/仕様/使用/ユーザー、社会: 厚生/更生/公正/構成、回答: 回答/解答/校正、食: 柿/牡蠣/ラーメン/雀、動詞: 書く/進める）。k-means（k=5）で話題ごとに別のクラスタになる。
- `存在しない語1` と `存在しない語2` は、辞書の lemma と一致しない語（一致しないものは行にならないことのテスト用）。
- 行の並び（頻度順位）をテストするため、話題の語を混ぜた順で並べてある。

使い方:

```
cargo run -p ikari-dict-builder -- vectors \
  --chive testdata/chive-mini/vectors.txt --dict <辞書ディレクトリ> \
  --out <辞書ディレクトリ>/vectors.bin --clusters 5
```
