# 難読字の動詞の別表記を後ろに回す（rare_kanji_variant。#119）

日付: 2026-10-30。`かつぐ` の 1 位が 舁ぐ（難読字）で、担ぐ が 2 位だった。ポリシーに述語 `rare_kanji_variant` を足し、値は掃引で決めた。

## 1. 原因（測った値）

- 元の辞書の終止形のコストは 舁ぐ 9597 < 担ぐ 10264（差 667）。2 つは Sudachi の同じ正規化表記（lemma = 担ぐ）で、品詞も連接も同じ。lemma が同じなので chiVe の頻度（`lemma_rank`）も同じ加算になり、コストの差がそのまま順位になる。
- 活用しない語の別表記は `variant_surface`（+1500）で後ろに回してあるが、活用する語（動詞・形容詞）は対象外だった（ADR 0009 §0.2 の 1）。
- LM あり（締め切り 2000ms、時間切れ 0）の `full` では、基準の時点で 担ぐ / 担いで が 1 位。LM が間に合わないとき（LM なしの `cost` / `full`）の並びを直す変更。

## 2. 掃引（LM なし = `cost` と `full`、LM あり = `full` + zenz 2000ms）

全 `eval/sets/*.tsv`（`cost` と `full`。LM ありは homophones、homophones-ext、hiragana、verbs と issue のセット）と AJIMEE（topic）の、基準（main b36d647）に対する 1 位の悪化の件数。`ikari-eval` を `--repeat 1` で流した。

### 2.1 広い案: 活用する動詞・形容詞の辞書形の表記が lemma と違うもの全部（採らなかった）

| 値 | 1 位の悪化 | 1 位の改善 | かつぐ（単独・文脈）の順位 | 悪化した例 |
| --- | --- | --- | --- | --- |
| 700 | 1 | 15 | 2 → 1 | AJIMEE: 囚われず が 捕らわれず に負ける |
| 1000 | 1 | 16 | 2 → 1 | 同上 |
| 1500 | 3 | 16 | 2 → 1 | 同上、adverb-verbs の すぐ行ける、まだ行ける（行ける が いける に負ける） |
| 3000 | 5 | 16 | 2 → 1 | 同上、small-kana の 悪ぃ |

悪化 0 の点がない。Sudachi は 囚われる を 捕らわれる、行ける を いける などに正規化していて、正しい表記が選ばれる場面がある（異字同訓や送り仮名のゆれ）。lemma との違いだけでは、難読字と、よく使う別表記を分けられない。

### 2.2 採った案: 辞書形の表記が chiVe の語彙にない漢字（1 字）を含むもの

難読字（舁 は chiVe の語彙にない。囚 49750 位、捕 50388 位、担 24782 位）だけに当てる。異字同訓の表（`kun.tsv`）にある lemma は対象にしない。

| 値 | 当たった行 | 1 位の悪化 | 1 位の改善 | かつぐ（`cost` / `full`、単独・文脈） | かついで（単独・文脈） |
| --- | --- | --- | --- | --- | --- |
| 1000 | 40306 | 0 | 3 | 2 → 1 | 4 → 4（full の文脈つき 3 → 3） |
| 3000 | 40306 | 0 | 3 | 2 → 1 | 4 → 3（full の文脈つき 3 → 2） |
| **1500（採用）** | 40306 | **0**（snapshot） | 下の §3 | 2 → 1 | 4 → 3（full の文脈つき 3 → 2） |

- 下限は 舁ぐ と 担ぐ の差 667。700〜3000 は同じ結果（悪化 0）。値は、対応する `variant_surface` と同じ 1500。
- LM あり（2000ms）: かつぐ / かついで とも、基準から 1 位が 担ぐ / 担いで（変化なし）。
- かついで の 1 位は かついで（かつ + いで の分割）のまま。これは別の問題（分割の経路）で、この変更では直らない。

## 3. snapshot（`eval/scripts/snapshot.sh --lm … --ajimee`。基準は main b36d647 の辞書）

- 全構成（homophones、ext dev / test、hiragana、katakana、context-compound、adverbs、verbs、compound-affix、new-words、rare-homophones、formal-noun-split、counters、toshiyou、sahen-shiyou、path-loses、segments、AJIMEE。LM なし・あり）の 1 位の悪化 0。
- 順位が上がったもの（LM なしの構成。`ikari-eval compare` の「改善」）: ext dev 4（topic・full とも）、ext test 2（うち 1 件が 1 位に。1 位 201 → 202）、adverbs 2、verbs 1、new-words 2、sahen-shiyou 2、path-loses 1、AJIMEE（topic）3。LM ありの構成（homophones、ext、hiragana、verbs、AJIMEE）は順位の変化なし（AJIMEE の 1 位 111 / 200 のまま）。
- 学習のシナリオ（LM なし・あり）14 本は一致。LM の時間切れ 0。warm p99 の悪化なし。CLI golden の差は 38 行で、難読字の動詞（酤い、仕る、寐 など）が下位に下がるものだけ。
- 辞書の `grammar.bin` の sha256 は基準と同じ（品詞の統計は変わらない）。

## 4. 再現

```sh
scripts/build-dict.sh --data <data のディレクトリ>   # chive.tsv の rare_kanji_variant を 0 にするか消すと基準
eval/scripts/snapshot.sh <出力> --lm <zenz の gguf> --ajimee
ikari-eval --dict <辞書> --ranker cost --set eval/sets/issue119.tsv
```
