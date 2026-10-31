# LM の alpha と max_total の掃引（#135。じんめい が 人命 になる）

日付: 2026-10-31。**既定値（alpha=1.0、max_total=6000）は変えていない。** この報告は、値を決めるための計測だけ。

## 1. 条件

- ビルド: `ikari-eval`（`--features lm-metal`、release）。コミットは main 7f28f79 の上。
- 辞書: `data/build/dict`（実機の `~/Library/Application Support/ikari-ime/dict` の `system.dict` と同一。`cmp` で確認）。モデル: zenz-v3.2-small-Q5_K_M。読み取りのみ。
- 構成: `--ranker full`、`--repeat 1`、学習ストアは一時ディレクトリ（利用者の学習は混ざらない）。
- **LM の締め切りは `--lm-deadline-ms 3000 --lm-budget-ms 3000`。実機の 30ms + 6ms × 読みの文字数とは条件が違う**（実機の LM は時間切れで外れることがある）。この表は「LM が間に合ったとき」の値。時間切れは、alpha=2.0 / 2.5 の max_total=12000 で 2 / 1 件あった以外は 0（マシンの負荷による揺れ。load average は開始時 15 前後）。
- 評価セット: `eval/sets/` の全部（unreachable を除く 3389 ケース。37 ファイル、今回足した issue135、issue135-watch を含む）と、各組み合わせ。`zenz.alpha` を 1.0 / 1.25 / 1.5 / 1.75 / 2.0 / 2.5 / 3.0、`zenz.max_total` を 6000 / 12000 / 20000（`--ranker-config` の `zenz.alpha`、`zenz.max_total`）。`max_candidates`（32）は掃引していない。
- 基準は alpha=1.0、max_total=6000 の LM あり。表 A の「LM なし」は `--ranker full` で LM を付けない構成（参考）。
- 追加した評価セット: [issue135.tsv](../sets/issue135.tsv)（じんめい。人名が期待 3、人命が期待 4 の対）、[issue135-watch.tsv](../sets/issue135-watch.tsv)（かんさつしよう → 観察しよう。左文脈なし / 「#3はもう少し」/ 「もう少し」。悪化の見張り）。

## 2. 読み取れること（事実）

- 人名: alpha=1.5 以上で 3 文脈とも 人名 が 1 位になる（max_total は 6000 のままでよい。max_total を上げても人名の結果は変わらない）。alpha 1.0 / 1.25 では「めずらしい」「これはめずらしい」が 2 位（1 位は 人命）。「めずらしい名前の」は基準でも 1 位。
- 人命: 対の 4 文脈（災害で失われた、救助された、事故で多くの、何より尊い）は、LM なしを含む全組み合わせで 1 位 人命（alpha を 3.0 に上げても落ちない）。
- **悪化がゼロの組み合わせはない**（基準以外）。alpha を上げるほど、基準で 1 位だったケースの悪化が増える: 6000 では 17（1.25）、26（1.5）、39、49、60、71（3.0）。max_total=12000 / 20000 でも alpha=1.0 で 3 件（english-first 1、sahen-shiyou 2）。
- 全セット合計の top-1 は基準 2963（87.43%）。alpha を上げると下がる（1.5 / 6000 で -6、3.0 / 6000 で -25）。人名が直る alpha=1.5 の 3 点: 6000 で -6、12000 で -3、20000 で -3。alpha=1.0 で max_total を 12000 / 20000 に上げると +5（katakana-like +6、adverbs +1、place-names +1、sahen-shiyou -2、english-first -1）。
- 悪化が集まるセット: animal-kanji（基準 36 → 1.5 で 34、3.0 で 25）、formal-noun-split（30 → 27、3.0 で 25〜26）、hiragana（136 → 133）、homophones-ext の同音語（保障/補償、解答/回答 など）、katakana-like（フクロウ、ポロポロ のカタカナが勝つ）。動物の表記に正解は 1 つではない（animal-kanji の期待は固定した規則にしない方針）ので、そこの悪化は重みを下げて読む。
- **見張り（かんさつしよう）: この評価では再現しない。** LM あり全 21 組み合わせで、3 通りの左文脈とも 1 位 観察しよう（alpha を 3.0 に上げても変わらない）。逆に LM なし（`--ranker full`）では「#3はもう少し」「もう少し」で 2 位（1 位 観察使用）、左文脈なしは 1 位 観察しよう。リードの報告（実機・CLI で LM ありのとき 観察使用 が 1 位、cost では 観察しよう）と、LM の向きが逆になっている。この評価は学習ストアが空（一時ディレクトリ）なので、実機との違いは、利用者の学習やユーザー辞書（今回は読まない）が原因かもしれない。**推測**であって、確かめていない。このため、見張りのケースからは alpha のトレードオフを読み取れない。
- max_total と候補ごとの上限について: 表の範囲では、max_total を上げて両立する組み合わせは見つからない。max_total を 6000 → 12000 にすると katakana-like が上がる（alpha=1.0 で +6）一方、sahen-shiyou が下がる（167 → 165）。alpha が同じなら 12000 と 20000 はほぼ同じ（top-3 / top-5 に少し差があるだけ）。人名を直すのは alpha であり、max_total の上限は関係しない。候補ごとの加点の上限（`max_candidates`）は掃引していない。

## 3. 推薦の候補（決定ではない）

- 人名 3 文脈を直す最小の alpha は 1.5。根拠: 表 C で 3/3。代償は表 A、D: max_total=12000 / 20000 なら基準比 top-1 -3、1 位の悪化 32（改善 29）。6000 なら -6、悪化 26（改善 20）。
- 悪化が 0 の点はないので、「alpha だけで直す」場合は、上の悪化を受け入れるかどうかの判断が要る。悪化 1 位から 2 位への移動が大半（表 D）。
- 推測: alpha を 1.0 のまま、人名だけを直すには、LM の効かせ方とは別の手当て（例: 左文脈の語が 珍しい のとき）が要るかもしれない。ユーザー決定は alpha なので、ここでは掃引した範囲の事実だけ載せる。

## 4. 表

ケース数: 3389（unreachable を除く）

### 表 A: 全セットの合計

| 組み合わせ | top-1 | top-3 | top-5 | 基準との top-1 差 | 1 位の悪化 | 1 位の改善 | LM 呼び出し / 時間切れ |
| --- | --- | --- | --- | --- | --- | --- | --- |
| LM なし | 2691 (79.40%) | 3178 (93.77%) | 3294 (97.20%) | -272 | 328 | 56 | なし |
| a1.0_m6000 | 2963 (87.43%) | 3327 (98.17%) | 3352 (98.91%) | +0 | 0 | 0 | 6817 / 0 |
| a1.25_m6000 | 2956 (87.22%) | 3320 (97.96%) | 3351 (98.88%) | -7 | 17 | 10 | 6817 / 0 |
| a1.5_m6000 | 2957 (87.25%) | 3317 (97.88%) | 3350 (98.85%) | -6 | 26 | 20 | 6817 / 0 |
| a1.75_m6000 | 2950 (87.05%) | 3312 (97.73%) | 3350 (98.85%) | -13 | 39 | 26 | 6817 / 0 |
| a2.0_m6000 | 2948 (86.99%) | 3310 (97.67%) | 3350 (98.85%) | -15 | 49 | 34 | 6817 / 0 |
| a2.5_m6000 | 2945 (86.90%) | 3307 (97.58%) | 3346 (98.73%) | -18 | 60 | 42 | 6817 / 0 |
| a3.0_m6000 | 2938 (86.69%) | 3302 (97.43%) | 3344 (98.67%) | -25 | 71 | 46 | 6817 / 0 |
| a1.0_m12000 | 2968 (87.58%) | 3333 (98.35%) | 3354 (98.97%) | +5 | 3 | 8 | 6817 / 0 |
| a1.25_m12000 | 2959 (87.31%) | 3332 (98.32%) | 3354 (98.97%) | -4 | 22 | 18 | 6817 / 0 |
| a1.5_m12000 | 2960 (87.34%) | 3328 (98.20%) | 3354 (98.97%) | -3 | 32 | 29 | 6817 / 0 |
| a1.75_m12000 | 2951 (87.08%) | 3323 (98.05%) | 3355 (99.00%) | -12 | 47 | 35 | 6817 / 0 |
| a2.0_m12000 | 2952 (87.11%) | 3317 (97.88%) | 3354 (98.97%) | -11 | 57 | 46 | 6817 / 2 |
| a2.5_m12000 | 2949 (87.02%) | 3310 (97.67%) | 3353 (98.94%) | -14 | 70 | 56 | 6817 / 1 |
| a3.0_m12000 | 2941 (86.78%) | 3307 (97.58%) | 3350 (98.85%) | -22 | 81 | 59 | 6817 / 0 |
| a1.0_m20000 | 2968 (87.58%) | 3332 (98.32%) | 3353 (98.94%) | +5 | 3 | 8 | 6817 / 0 |
| a1.25_m20000 | 2959 (87.31%) | 3331 (98.29%) | 3354 (98.97%) | -4 | 22 | 18 | 6817 / 0 |
| a1.5_m20000 | 2960 (87.34%) | 3330 (98.26%) | 3353 (98.94%) | -3 | 32 | 29 | 6817 / 0 |
| a1.75_m20000 | 2951 (87.08%) | 3326 (98.14%) | 3357 (99.06%) | -12 | 47 | 35 | 6817 / 0 |
| a2.0_m20000 | 2952 (87.11%) | 3321 (97.99%) | 3356 (99.03%) | -11 | 57 | 46 | 6817 / 0 |
| a2.5_m20000 | 2949 (87.02%) | 3312 (97.73%) | 3354 (98.97%) | -14 | 70 | 56 | 6817 / 0 |
| a3.0_m20000 | 2941 (86.78%) | 3309 (97.64%) | 3353 (98.94%) | -22 | 81 | 59 | 6817 / 0 |

### 表 B: セットごとの top-1 の数（基準との差が出たセットだけ。基準 → 各組み合わせ。LM なしは参考）

| セット | 件数 | LMなし | a1.0_m6000 | a1.25_m6000 | a1.5_m6000 | a1.75_m6000 | a2.0_m6000 | a2.5_m6000 | a3.0_m6000 | a1.0_m12000 | a1.25_m12000 | a1.5_m12000 | a1.75_m12000 | a2.0_m12000 | a2.5_m12000 | a3.0_m12000 | a1.0_m20000 | a1.25_m20000 | a1.5_m20000 | a1.75_m20000 | a2.0_m20000 | a2.5_m20000 | a3.0_m20000 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| abr-towns | 27 | 25 | 23 | 23 | 23 | 23 | 23 | 23 | 22 | 23 | 23 | 23 | 23 | 23 | 23 | 22 | 23 | 23 | 23 | 23 | 23 | 23 | 22 |
| adverb-verbs | 56 | 45 | 49 | 49 | 49 | 49 | 49 | 49 | 51 | 49 | 49 | 49 | 49 | 49 | 49 | 51 | 49 | 49 | 49 | 49 | 49 | 49 | 51 |
| adverbs | 174 | 155 | 170 | 170 | 170 | 170 | 170 | 170 | 170 | 171 | 171 | 171 | 171 | 171 | 171 | 171 | 171 | 171 | 171 | 171 | 171 | 171 | 171 |
| animal-kanji | 43 | 39 | 36 | 34 | 34 | 32 | 29 | 27 | 25 | 36 | 34 | 34 | 32 | 29 | 27 | 25 | 36 | 34 | 34 | 32 | 29 | 27 | 25 |
| brand-katakana | 16 | 3 | 11 | 11 | 11 | 12 | 12 | 12 | 12 | 11 | 11 | 11 | 12 | 12 | 12 | 12 | 11 | 11 | 11 | 12 | 12 | 12 | 12 |
| compound-affix | 100 | 99 | 99 | 99 | 99 | 99 | 99 | 98 | 97 | 99 | 99 | 99 | 99 | 99 | 98 | 97 | 99 | 99 | 99 | 99 | 99 | 98 | 97 |
| context-compound | 54 | 51 | 53 | 52 | 52 | 51 | 51 | 51 | 51 | 53 | 52 | 52 | 50 | 50 | 50 | 50 | 53 | 52 | 52 | 50 | 50 | 50 | 50 |
| counters | 102 | 98 | 102 | 102 | 102 | 102 | 102 | 101 | 101 | 102 | 102 | 102 | 102 | 102 | 101 | 101 | 102 | 102 | 102 | 102 | 102 | 101 | 101 |
| english-first | 48 | 36 | 37 | 37 | 37 | 37 | 37 | 37 | 37 | 36 | 36 | 36 | 36 | 36 | 36 | 36 | 36 | 36 | 36 | 36 | 36 | 36 | 36 |
| formal-noun-split | 36 | 29 | 30 | 28 | 27 | 27 | 26 | 26 | 26 | 30 | 28 | 27 | 27 | 26 | 25 | 25 | 30 | 28 | 27 | 27 | 26 | 25 | 25 |
| funhen | 26 | 6 | 14 | 13 | 13 | 14 | 14 | 14 | 14 | 14 | 13 | 13 | 14 | 14 | 14 | 14 | 14 | 13 | 13 | 14 | 14 | 14 | 14 |
| hiragana | 140 | 136 | 136 | 135 | 133 | 133 | 132 | 131 | 130 | 136 | 135 | 133 | 133 | 132 | 131 | 130 | 136 | 135 | 133 | 133 | 132 | 131 | 130 |
| homophones-ext | 1266 | 994 | 1119 | 1120 | 1122 | 1119 | 1122 | 1128 | 1126 | 1119 | 1120 | 1123 | 1120 | 1125 | 1132 | 1130 | 1119 | 1120 | 1123 | 1120 | 1125 | 1132 | 1130 |
| issue135 | 7 | 4 | 5 | 5 | 7 | 7 | 7 | 7 | 7 | 5 | 5 | 7 | 7 | 7 | 7 | 7 | 5 | 5 | 7 | 7 | 7 | 7 | 7 |
| katakana-like | 103 | 85 | 88 | 87 | 86 | 84 | 81 | 78 | 78 | 94 | 93 | 92 | 90 | 87 | 84 | 84 | 94 | 93 | 92 | 90 | 87 | 84 | 84 |
| path-loses | 39 | 22 | 36 | 36 | 36 | 37 | 37 | 37 | 37 | 36 | 36 | 36 | 37 | 37 | 37 | 37 | 36 | 36 | 36 | 37 | 37 | 37 | 37 |
| place-names | 167 | 146 | 157 | 157 | 156 | 155 | 155 | 154 | 153 | 158 | 158 | 157 | 156 | 156 | 155 | 154 | 158 | 158 | 157 | 156 | 156 | 155 | 154 |
| rare-homophones | 77 | 50 | 67 | 67 | 67 | 68 | 68 | 70 | 71 | 67 | 67 | 67 | 68 | 68 | 70 | 71 | 67 | 67 | 67 | 68 | 68 | 70 | 71 |
| rightctx | 58 | 30 | 42 | 42 | 43 | 42 | 43 | 43 | 42 | 42 | 42 | 43 | 42 | 43 | 43 | 42 | 42 | 42 | 43 | 42 | 43 | 43 | 42 |
| sahen-shiyou | 177 | 158 | 167 | 169 | 169 | 169 | 170 | 170 | 169 | 165 | 165 | 164 | 163 | 164 | 163 | 162 | 165 | 165 | 164 | 163 | 164 | 163 | 162 |
| segments | 145 | 105 | 108 | 108 | 108 | 107 | 107 | 107 | 107 | 108 | 108 | 108 | 107 | 107 | 108 | 107 | 108 | 108 | 108 | 107 | 107 | 108 | 107 |
| small-kana | 4 | 4 | 3 | 3 | 3 | 3 | 3 | 2 | 2 | 3 | 3 | 3 | 3 | 3 | 2 | 2 | 3 | 3 | 3 | 3 | 3 | 2 | 2 |
| verbs | 120 | 71 | 98 | 96 | 97 | 97 | 98 | 97 | 97 | 98 | 96 | 97 | 97 | 99 | 98 | 98 | 98 | 96 | 97 | 97 | 99 | 98 | 98 |

この表にないセットは、LM あり全組み合わせで基準と top-1 の数が同じ。

### 表 C: 人名・人命の対（issue135。読み じんめい。順位 / 1 位の表記）

| 組み合わせ | 災害で失われた（人命） | 救助された（人命） | 事故で多くの（人命） | 何より尊い（人命） | めずらしい（人名） | これはめずらしい（人名） | めずらしい名前の（人名） | 人名が 1 位 | 人命が 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| LM なし | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 3 / 人命 | 3 / 人命 | 3 / 人命 | 0/3 | 4/4 |
| a1.0_m6000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 2 / 人命 | 2 / 人命 | 1 / 人名 | 1/3 | 4/4 |
| a1.25_m6000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 2 / 人命 | 2 / 人命 | 1 / 人名 | 1/3 | 4/4 |
| a1.5_m6000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a1.75_m6000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a2.0_m6000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a2.5_m6000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a3.0_m6000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a1.0_m12000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 2 / 人命 | 2 / 人命 | 1 / 人名 | 1/3 | 4/4 |
| a1.25_m12000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 2 / 人命 | 2 / 人命 | 1 / 人名 | 1/3 | 4/4 |
| a1.5_m12000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a1.75_m12000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a2.0_m12000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a2.5_m12000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a3.0_m12000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a1.0_m20000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 2 / 人命 | 2 / 人命 | 1 / 人名 | 1/3 | 4/4 |
| a1.25_m20000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 2 / 人命 | 2 / 人命 | 1 / 人名 | 1/3 | 4/4 |
| a1.5_m20000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a1.75_m20000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a2.0_m20000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a2.5_m20000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |
| a3.0_m20000 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人命 | 1 / 人名 | 1 / 人名 | 1 / 人名 | 3/3 | 4/4 |

### 表 D: 基準（alpha=1.0、max_total=6000）で 1 位だったのに 1 位でなくなったケース


**a1.25_m6000**: 17 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 家庭 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |

**a1.5_m6000**: 26 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 2 | 時間を空ける |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |

**a1.75_m6000**: 39 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-kujira-oyogu |  | くじらがおよぐ | くじらが泳ぐ | 1 | 2 | クジラが泳ぐ |
| animal-kanji | fix-nezumi-hashiru |  | ねずみがはしる | ねずみが走る | 1 | 2 | ネズミが走る |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| context-compound | ctxc-chi-ka | 地 | か | 下 | 1 | 2 | か |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-03 | まず | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-09 | 不確かな | かてい | 仮定 | 1 | 2 | 過程 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-kousei-3-02 | 受刑者の | こうせい | 更生 | 1 | 2 | 構成 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-shoukai-2-06 | 警察に前科の | しょうかい | 照会 | 1 | 2 | 紹介 |
| homophones-ext | ext-tsuikyuu-3-08 | 疑問を徹底的に | ついきゅう | 追究|追求 | 1 | 2 | 追及 |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-11 |  | むしむし | むしむし | 1 | 2 | ムシムシ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| katakana-like | kl-guard-42 |  | ぐらぐら | ぐらぐら | 1 | 2 | グラグラ |
| place-names | place-012 |  | ながの | 長野 | 1 | 2 | ナガノ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-4 | 留学の | きかい | 機会 | 1 | 2 | 機械 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| segments | seg-brk-43 |  | ごじにはじまる | 五時に始まる | 1 | 2 | 5時に始まる |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 2 | 時間を空ける |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |

**a2.0_m6000**: 49 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-katatsumuri |  | かたつむり | かたつむり | 1 | 2 | カタツムリ |
| animal-kanji | fix-kujira-oyogu |  | くじらがおよぐ | くじらが泳ぐ | 1 | 2 | クジラが泳ぐ |
| animal-kanji | fix-nezumi-hashiru |  | ねずみがはしる | ねずみが走る | 1 | 2 | ネズミが走る |
| animal-kanji | fix-suzume |  | すずめ | すずめ | 1 | 2 | スズメ |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | keep-hitsuji |  | ひつじ | 羊 | 1 | 2 | ひつじ |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| context-compound | ctxc-chi-ka | 地 | か | 下 | 1 | 2 | か |
| context-compound | ctxc-hon-ya | 本 | や | 屋 | 1 | 2 | や |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| formal-noun-split | fns-leak-20 | 大きな | ものおき | 物置 | 1 | 2 | 物置き |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-26 | できる | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-03 | まず | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-09 | 不確かな | かてい | 仮定 | 1 | 2 | 過程 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-kousei-3-02 | 受刑者の | こうせい | 更生 | 1 | 2 | 構成 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-10 | 休み時間に | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-shoukai-2-06 | 警察に前科の | しょうかい | 照会 | 1 | 2 | 紹介 |
| homophones-ext | ext-tsuikyuu-3-08 | 疑問を徹底的に | ついきゅう | 追究|追求 | 1 | 2 | 追及 |
| katakana-like | kl-guard-02 |  | ぺたぺた | ぺたぺた | 1 | 2 | ペタペタ |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-11 |  | むしむし | むしむし | 1 | 2 | ムシムシ |
| katakana-like | kl-guard-14 |  | ぽつぽつ | ぽつぽつ | 1 | 2 | ポツポツ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| katakana-like | kl-guard-42 |  | ぐらぐら | ぐらぐら | 1 | 2 | グラグラ |
| katakana-like | kl-guard-48 |  | ぱくぱく | ぱくぱく | 1 | 2 | パクパク |
| place-names | place-012 |  | ながの | 長野 | 1 | 2 | ナガノ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-4 | 留学の | きかい | 機会 | 1 | 2 | 機械 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| segments | seg-brk-43 |  | ごじにはじまる | 五時に始まる | 1 | 2 | 5時に始まる |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 2 | 時間を空ける |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |

**a2.5_m6000**: 60 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-katatsumuri |  | かたつむり | かたつむり | 1 | 2 | カタツムリ |
| animal-kanji | fix-kujira-oyogu |  | くじらがおよぐ | くじらが泳ぐ | 1 | 2 | クジラが泳ぐ |
| animal-kanji | fix-nezumi |  | ねずみ | ねずみ | 1 | 2 | ネズミ |
| animal-kanji | fix-nezumi-hashiru |  | ねずみがはしる | ねずみが走る | 1 | 2 | ネズミが走る |
| animal-kanji | fix-suzume |  | すずめ | すずめ | 1 | 2 | スズメ |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | fix-tanuki-hashiru |  | たぬきがはしる | たぬきが走る | 1 | 2 | タヌキが走る |
| animal-kanji | keep-hitsuji |  | ひつじ | 羊 | 1 | 2 | ひつじ |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| compound-affix | cmp-pos-54 |  | ひかくか | 非核化 | 1 | 2 | 比較か |
| context-compound | ctxc-chi-ka | 地 | か | 下 | 1 | 2 | か |
| context-compound | ctxc-hon-ya | 本 | や | 屋 | 1 | 2 | や |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| counters | fix-san-shuu | 三 | しゅう | 週 | 1 | 2 | 州 |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| formal-noun-split | fns-leak-20 | 大きな | ものおき | 物置 | 1 | 2 | 物置き |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-26 | できる | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| hiragana | hira-kan-09 | 資料を | ください | 下さい | 1 | 2 | ください |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-03 | まず | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-09 | 不確かな | かてい | 仮定 | 1 | 2 | 過程 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-kousei-3-02 | 受刑者の | こうせい | 更生 | 1 | 2 | 構成 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 3 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-10 | 休み時間に | こうてい | 校庭 | 1 | 3 | 行程 |
| homophones-ext | ext-shoukai-2-06 | 警察に前科の | しょうかい | 照会 | 1 | 2 | 紹介 |
| homophones-ext | ext-tsuikyuu-3-08 | 疑問を徹底的に | ついきゅう | 追究|追求 | 1 | 2 | 追及 |
| katakana-like | kl-guard-01 |  | ぷんぷん | ぷんぷん | 1 | 2 | プンプン |
| katakana-like | kl-guard-02 |  | ぺたぺた | ぺたぺた | 1 | 2 | ペタペタ |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-05 |  | ぱちぱち | ぱちぱち | 1 | 2 | パチパチ |
| katakana-like | kl-guard-11 |  | むしむし | むしむし | 1 | 3 | ムシムシ |
| katakana-like | kl-guard-14 |  | ぽつぽつ | ぽつぽつ | 1 | 2 | ポツポツ |
| katakana-like | kl-guard-23 |  | ぺこぺこ | ぺこぺこ | 1 | 2 | ペコペコ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| katakana-like | kl-guard-42 |  | ぐらぐら | ぐらぐら | 1 | 2 | グラグラ |
| katakana-like | kl-guard-48 |  | ぱくぱく | ぱくぱく | 1 | 2 | パクパク |
| place-names | place-012 |  | ながの | 長野 | 1 | 2 | ナガノ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| place-names | place-089 |  | みやこ | 都 | 1 | 2 | みやこ |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-4 | 留学の | きかい | 機会 | 1 | 2 | 機械 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| segments | seg-brk-43 |  | ごじにはじまる | 五時に始まる | 1 | 2 | 5時に始まる |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 3 | 時間を空ける |
| small-kana | smallkana-02 |  | わりぃ | 悪ぃ | 1 | 2 | わりぃ |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |
| verbs | verb-noboru-02 | 階段を | のぼる | 上る | 1 | 2 | 登る |

**a3.0_m6000**: 71 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| abr-towns | abr-027 | 髪に | くし | 櫛 | 1 | 2 | くし |
| animal-kanji | fix-katatsumuri |  | かたつむり | かたつむり | 1 | 2 | カタツムリ |
| animal-kanji | fix-kujira-oyogu |  | くじらがおよぐ | くじらが泳ぐ | 1 | 2 | クジラが泳ぐ |
| animal-kanji | fix-nezumi |  | ねずみ | ねずみ | 1 | 2 | ネズミ |
| animal-kanji | fix-nezumi-hashiru |  | ねずみがはしる | ねずみが走る | 1 | 2 | ネズミが走る |
| animal-kanji | fix-suzume |  | すずめ | すずめ | 1 | 2 | スズメ |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | fix-tanuki-hashiru |  | たぬきがはしる | たぬきが走る | 1 | 2 | タヌキが走る |
| animal-kanji | keep-hitsuji |  | ひつじ | 羊 | 1 | 2 | ひつじ |
| animal-kanji | keep-tanuki-neiri |  | たぬきねいり | 狸寝入り | 1 | 2 | たぬき寝入り |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| animal-kanji | rem-kitsune-to-tanuki |  | きつねとたぬき | きつねとたぬき | 1 | 2 | キツネとタヌキ |
| compound-affix | cmp-pos-30 |  | きのうほう | 帰納法 | 1 | 2 | 機能法 |
| compound-affix | cmp-pos-54 |  | ひかくか | 非核化 | 1 | 2 | 比較か |
| context-compound | ctxc-chi-ka | 地 | か | 下 | 1 | 2 | か |
| context-compound | ctxc-hon-ya | 本 | や | 屋 | 1 | 2 | や |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| counters | fix-san-shuu | 三 | しゅう | 週 | 1 | 2 | 州 |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| formal-noun-split | fns-leak-20 | 大きな | ものおき | 物置 | 1 | 2 | 物置き |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-25 | 安い | ほうが | ほうが | 1 | 2 | 方が |
| hiragana | hira-fn-26 | できる | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| hiragana | hira-kan-09 | 資料を | ください | 下さい | 1 | 2 | ください |
| hiragana | hira-kan-20 |  | すべて | 全て | 1 | 2 | すべて |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-03 | まず | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-09 | 不確かな | かてい | 仮定 | 1 | 2 | 過程 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 4 | 過程 |
| homophones-ext | ext-kousei-3-02 | 受刑者の | こうせい | 更生 | 1 | 2 | 構成 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 3 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-10 | 休み時間に | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-shoukai-2-06 | 警察に前科の | しょうかい | 照会 | 1 | 2 | 紹介 |
| homophones-ext | ext-shuukan-3-04 | 新聞の | しゅうかん | 週刊 | 1 | 2 | 週間 |
| homophones-ext | ext-taishou-3-03 | 線 | たいしょう | 対称 | 1 | 2 | 対象 |
| homophones-ext | ext-tsuikyuu-3-08 | 疑問を徹底的に | ついきゅう | 追究|追求 | 1 | 2 | 追及 |
| katakana-like | kl-guard-01 |  | ぷんぷん | ぷんぷん | 1 | 2 | プンプン |
| katakana-like | kl-guard-02 |  | ぺたぺた | ぺたぺた | 1 | 2 | ペタペタ |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-05 |  | ぱちぱち | ぱちぱち | 1 | 2 | パチパチ |
| katakana-like | kl-guard-11 |  | むしむし | むしむし | 1 | 3 | ムシムシ |
| katakana-like | kl-guard-14 |  | ぽつぽつ | ぽつぽつ | 1 | 2 | ポツポツ |
| katakana-like | kl-guard-23 |  | ぺこぺこ | ぺこぺこ | 1 | 2 | ペコペコ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| katakana-like | kl-guard-42 |  | ぐらぐら | ぐらぐら | 1 | 2 | グラグラ |
| katakana-like | kl-guard-48 |  | ぱくぱく | ぱくぱく | 1 | 2 | パクパク |
| place-names | place-012 |  | ながの | 長野 | 1 | 2 | ナガノ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| place-names | place-089 |  | みやこ | 都 | 1 | 2 | みやこ |
| place-names | place-138 | 山の | ふもと | 麓 | 1 | 2 | ふもと |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-4 | 留学の | きかい | 機会 | 1 | 2 | 機械 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| rightctx | rc-long-kikai-f2 | あの | きかい | 機会 | 1 | 2 | 機械 |
| sahen-shiyou | ss-g-114 |  | じゅんびしようしょ | 準備仕様書 | 1 | 2 | 準備しようしょ |
| segments | seg-brk-43 |  | ごじにはじまる | 五時に始まる | 1 | 4 | 5時に始まる |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 2 | 時間を空ける |
| small-kana | smallkana-02 |  | わりぃ | 悪ぃ | 1 | 2 | わりぃ |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |
| verbs | verb-noboru-02 | 階段を | のぼる | 上る | 1 | 2 | 登る |

**a1.0_m12000**: 3 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 4 | はるしねーしょんがおきる |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 2 | 運動しようちゅう |

**a1.25_m12000**: 22 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 3 | はるしねーしょんがおきる |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 家庭 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-124 |  | そうじしようちゅう | 掃除使用中 | 1 | 3 | 掃除しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 3 | 運動しようちゅう |
| sahen-shiyou | ss-g-126 |  | じゅんびしようちゅう | 準備使用中 | 1 | 3 | 準備し用中 |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |

**a1.5_m12000**: 32 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 3 | はるしねーしょんがおきる |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-124 |  | そうじしようちゅう | 掃除使用中 | 1 | 3 | 掃除しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 3 | 運動しようちゅう |
| sahen-shiyou | ss-g-126 |  | じゅんびしようちゅう | 準備使用中 | 1 | 3 | 準備しようちゅう |
| sahen-shiyou | ss-g-127 |  | そうだんしようちゅう | 相談使用中 | 1 | 2 | 相談しようちゅう |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 2 | 時間を空ける |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |

**a1.75_m12000**: 47 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-kujira-oyogu |  | くじらがおよぐ | くじらが泳ぐ | 1 | 2 | クジラが泳ぐ |
| animal-kanji | fix-nezumi-hashiru |  | ねずみがはしる | ねずみが走る | 1 | 2 | ネズミが走る |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| context-compound | ctxc-chi-ka | 地 | か | 下 | 1 | 2 | か |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| context-compound | ctxc-te-gami | 手 | がみ | 紙 | 1 | 2 | が見 |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 3 | はるしねーしょんがおきる |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-03 | まず | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-09 | 不確かな | かてい | 仮定 | 1 | 2 | 過程 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-kousei-3-02 | 受刑者の | こうせい | 更生 | 1 | 2 | 構成 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-shoukai-2-06 | 警察に前科の | しょうかい | 照会 | 1 | 2 | 紹介 |
| homophones-ext | ext-tsuikyuu-3-08 | 疑問を徹底的に | ついきゅう | 追究|追求 | 1 | 2 | 追及 |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-11 |  | むしむし | むしむし | 1 | 2 | ムシムシ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| katakana-like | kl-guard-42 |  | ぐらぐら | ぐらぐら | 1 | 2 | グラグラ |
| place-names | place-012 |  | ながの | 長野 | 1 | 2 | ナガノ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-4 | 留学の | きかい | 機会 | 1 | 2 | 機械 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| sahen-shiyou | ss-g-120 |  | れんらくしようちゅう | 連絡使用中 | 1 | 2 | 連絡しようちゅう |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-124 |  | そうじしようちゅう | 掃除使用中 | 1 | 3 | 掃除しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 3 | 運動しようちゅう |
| sahen-shiyou | ss-g-126 |  | じゅんびしようちゅう | 準備使用中 | 1 | 3 | 準備しようちゅう |
| sahen-shiyou | ss-g-127 |  | そうだんしようちゅう | 相談使用中 | 1 | 3 | 相談しようちゅう |
| segments | seg-brk-43 |  | ごじにはじまる | 五時に始まる | 1 | 2 | 5時に始まる |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 2 | 時間を空ける |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |

**a2.0_m12000**: 57 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-katatsumuri |  | かたつむり | かたつむり | 1 | 2 | カタツムリ |
| animal-kanji | fix-kujira-oyogu |  | くじらがおよぐ | くじらが泳ぐ | 1 | 2 | クジラが泳ぐ |
| animal-kanji | fix-nezumi-hashiru |  | ねずみがはしる | ねずみが走る | 1 | 2 | ネズミが走る |
| animal-kanji | fix-suzume |  | すずめ | すずめ | 1 | 2 | スズメ |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | keep-hitsuji |  | ひつじ | 羊 | 1 | 2 | ひつじ |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| context-compound | ctxc-chi-ka | 地 | か | 下 | 1 | 2 | か |
| context-compound | ctxc-hon-ya | 本 | や | 屋 | 1 | 2 | や |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| context-compound | ctxc-te-gami | 手 | がみ | 紙 | 1 | 2 | が見 |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 3 | はるしねーしょんがおきる |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| formal-noun-split | fns-leak-20 | 大きな | ものおき | 物置 | 1 | 2 | 物置き |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-26 | できる | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-03 | まず | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-09 | 不確かな | かてい | 仮定 | 1 | 2 | 過程 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-kousei-3-02 | 受刑者の | こうせい | 更生 | 1 | 2 | 構成 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-10 | 休み時間に | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-shoukai-2-06 | 警察に前科の | しょうかい | 照会 | 1 | 2 | 紹介 |
| homophones-ext | ext-tsuikyuu-3-08 | 疑問を徹底的に | ついきゅう | 追究|追求 | 1 | 2 | 追及 |
| katakana-like | kl-guard-02 |  | ぺたぺた | ぺたぺた | 1 | 2 | ペタペタ |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-11 |  | むしむし | むしむし | 1 | 2 | ムシムシ |
| katakana-like | kl-guard-14 |  | ぽつぽつ | ぽつぽつ | 1 | 2 | ポツポツ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| katakana-like | kl-guard-42 |  | ぐらぐら | ぐらぐら | 1 | 2 | グラグラ |
| katakana-like | kl-guard-48 |  | ぱくぱく | ぱくぱく | 1 | 2 | パクパク |
| place-names | place-012 |  | ながの | 長野 | 1 | 2 | ナガノ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-4 | 留学の | きかい | 機会 | 1 | 2 | 機械 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| sahen-shiyou | ss-g-120 |  | れんらくしようちゅう | 連絡使用中 | 1 | 2 | 連絡しようちゅう |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-124 |  | そうじしようちゅう | 掃除使用中 | 1 | 3 | 掃除しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 3 | 運動しようちゅう |
| sahen-shiyou | ss-g-126 |  | じゅんびしようちゅう | 準備使用中 | 1 | 3 | 準備しようちゅう |
| sahen-shiyou | ss-g-127 |  | そうだんしようちゅう | 相談使用中 | 1 | 3 | 相談しようちゅう |
| segments | seg-brk-43 |  | ごじにはじまる | 五時に始まる | 1 | 2 | 5時に始まる |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 2 | 時間を空ける |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |

**a2.5_m12000**: 70 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-katatsumuri |  | かたつむり | かたつむり | 1 | 2 | カタツムリ |
| animal-kanji | fix-kujira-oyogu |  | くじらがおよぐ | くじらが泳ぐ | 1 | 2 | クジラが泳ぐ |
| animal-kanji | fix-nezumi |  | ねずみ | ねずみ | 1 | 2 | ネズミ |
| animal-kanji | fix-nezumi-hashiru |  | ねずみがはしる | ねずみが走る | 1 | 2 | ネズミが走る |
| animal-kanji | fix-suzume |  | すずめ | すずめ | 1 | 2 | スズメ |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | fix-tanuki-hashiru |  | たぬきがはしる | たぬきが走る | 1 | 2 | タヌキが走る |
| animal-kanji | keep-hitsuji |  | ひつじ | 羊 | 1 | 2 | ひつじ |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| compound-affix | cmp-pos-54 |  | ひかくか | 非核化 | 1 | 2 | 比較か |
| context-compound | ctxc-chi-ka | 地 | か | 下 | 1 | 2 | か |
| context-compound | ctxc-hon-ya | 本 | や | 屋 | 1 | 2 | や |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| context-compound | ctxc-te-gami | 手 | がみ | 紙 | 1 | 2 | が見 |
| counters | fix-san-shuu | 三 | しゅう | 週 | 1 | 2 | 州 |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 3 | はるしねーしょんがおきる |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 3 | もの暗示 |
| formal-noun-split | fns-leak-13 |  | くせじ | 癖字 | 1 | 2 | くせじ |
| formal-noun-split | fns-leak-20 | 大きな | ものおき | 物置 | 1 | 2 | 物置き |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-26 | できる | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| hiragana | hira-kan-09 | 資料を | ください | 下さい | 1 | 2 | ください |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-03 | まず | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-09 | 不確かな | かてい | 仮定 | 1 | 2 | 過程 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-kousei-3-02 | 受刑者の | こうせい | 更生 | 1 | 2 | 構成 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-10 | 休み時間に | こうてい | 校庭 | 1 | 3 | 行程 |
| homophones-ext | ext-shoukai-2-06 | 警察に前科の | しょうかい | 照会 | 1 | 2 | 紹介 |
| homophones-ext | ext-tsuikyuu-3-08 | 疑問を徹底的に | ついきゅう | 追究|追求 | 1 | 2 | 追及 |
| katakana-like | kl-guard-01 |  | ぷんぷん | ぷんぷん | 1 | 2 | プンプン |
| katakana-like | kl-guard-02 |  | ぺたぺた | ぺたぺた | 1 | 2 | ペタペタ |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-05 |  | ぱちぱち | ぱちぱち | 1 | 2 | パチパチ |
| katakana-like | kl-guard-11 |  | むしむし | むしむし | 1 | 3 | ムシムシ |
| katakana-like | kl-guard-14 |  | ぽつぽつ | ぽつぽつ | 1 | 2 | ポツポツ |
| katakana-like | kl-guard-23 |  | ぺこぺこ | ぺこぺこ | 1 | 2 | ペコペコ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| katakana-like | kl-guard-42 |  | ぐらぐら | ぐらぐら | 1 | 2 | グラグラ |
| katakana-like | kl-guard-48 |  | ぱくぱく | ぱくぱく | 1 | 2 | パクパク |
| place-names | place-012 |  | ながの | 長野 | 1 | 2 | ナガノ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| place-names | place-089 |  | みやこ | 都 | 1 | 2 | みやこ |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-4 | 留学の | きかい | 機会 | 1 | 2 | 機械 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| sahen-shiyou | ss-g-119 |  | かくにんしようちゅう | 確認使用中 | 1 | 2 | 確認しようちゅう |
| sahen-shiyou | ss-g-120 |  | れんらくしようちゅう | 連絡使用中 | 1 | 2 | 連絡しようちゅう |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-124 |  | そうじしようちゅう | 掃除使用中 | 1 | 3 | 掃除しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 3 | 運動しようちゅう |
| sahen-shiyou | ss-g-126 |  | じゅんびしようちゅう | 準備使用中 | 1 | 3 | 準備しようちゅう |
| sahen-shiyou | ss-g-127 |  | そうだんしようちゅう | 相談使用中 | 1 | 3 | 相談しようちゅう |
| segments | seg-brk-43 |  | ごじにはじまる | 五時に始まる | 1 | 2 | 5時に始まる |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 3 | 時間を空ける |
| small-kana | smallkana-02 |  | わりぃ | 悪ぃ | 1 | 2 | わりぃ |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |
| verbs | verb-noboru-02 | 階段を | のぼる | 上る | 1 | 2 | 登る |

**a3.0_m12000**: 81 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| abr-towns | abr-027 | 髪に | くし | 櫛 | 1 | 2 | くし |
| animal-kanji | fix-katatsumuri |  | かたつむり | かたつむり | 1 | 2 | カタツムリ |
| animal-kanji | fix-kujira-oyogu |  | くじらがおよぐ | くじらが泳ぐ | 1 | 2 | クジラが泳ぐ |
| animal-kanji | fix-nezumi |  | ねずみ | ねずみ | 1 | 2 | ネズミ |
| animal-kanji | fix-nezumi-hashiru |  | ねずみがはしる | ねずみが走る | 1 | 2 | ネズミが走る |
| animal-kanji | fix-suzume |  | すずめ | すずめ | 1 | 2 | スズメ |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | fix-tanuki-hashiru |  | たぬきがはしる | たぬきが走る | 1 | 2 | タヌキが走る |
| animal-kanji | keep-hitsuji |  | ひつじ | 羊 | 1 | 2 | ひつじ |
| animal-kanji | keep-tanuki-neiri |  | たぬきねいり | 狸寝入り | 1 | 2 | たぬき寝入り |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| animal-kanji | rem-kitsune-to-tanuki |  | きつねとたぬき | きつねとたぬき | 1 | 2 | キツネとタヌキ |
| compound-affix | cmp-pos-30 |  | きのうほう | 帰納法 | 1 | 2 | 機能法 |
| compound-affix | cmp-pos-54 |  | ひかくか | 非核化 | 1 | 2 | 比較か |
| context-compound | ctxc-chi-ka | 地 | か | 下 | 1 | 3 | か |
| context-compound | ctxc-hon-ya | 本 | や | 屋 | 1 | 2 | や |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| context-compound | ctxc-te-gami | 手 | がみ | 紙 | 1 | 2 | が見 |
| counters | fix-san-shuu | 三 | しゅう | 週 | 1 | 2 | 州 |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 3 | はるしねーしょんがおきる |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 3 | もの暗示 |
| formal-noun-split | fns-leak-13 |  | くせじ | 癖字 | 1 | 2 | くせじ |
| formal-noun-split | fns-leak-20 | 大きな | ものおき | 物置 | 1 | 2 | 物置き |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-25 | 安い | ほうが | ほうが | 1 | 2 | 方が |
| hiragana | hira-fn-26 | できる | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| hiragana | hira-kan-09 | 資料を | ください | 下さい | 1 | 2 | ください |
| hiragana | hira-kan-20 |  | すべて | 全て | 1 | 2 | すべて |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-03 | まず | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-09 | 不確かな | かてい | 仮定 | 1 | 2 | 過程 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-kousei-3-02 | 受刑者の | こうせい | 更生 | 1 | 2 | 構成 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-10 | 休み時間に | こうてい | 校庭 | 1 | 3 | 行程 |
| homophones-ext | ext-shoukai-2-06 | 警察に前科の | しょうかい | 照会 | 1 | 2 | 紹介 |
| homophones-ext | ext-shuukan-3-04 | 新聞の | しゅうかん | 週刊 | 1 | 2 | 週間 |
| homophones-ext | ext-taishou-3-03 | 線 | たいしょう | 対称 | 1 | 2 | 対象 |
| homophones-ext | ext-tsuikyuu-3-08 | 疑問を徹底的に | ついきゅう | 追究|追求 | 1 | 2 | 追及 |
| katakana-like | kl-guard-01 |  | ぷんぷん | ぷんぷん | 1 | 2 | プンプン |
| katakana-like | kl-guard-02 |  | ぺたぺた | ぺたぺた | 1 | 2 | ペタペタ |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-05 |  | ぱちぱち | ぱちぱち | 1 | 2 | パチパチ |
| katakana-like | kl-guard-11 |  | むしむし | むしむし | 1 | 3 | ムシムシ |
| katakana-like | kl-guard-14 |  | ぽつぽつ | ぽつぽつ | 1 | 2 | ポツポツ |
| katakana-like | kl-guard-23 |  | ぺこぺこ | ぺこぺこ | 1 | 2 | ペコペコ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| katakana-like | kl-guard-42 |  | ぐらぐら | ぐらぐら | 1 | 2 | グラグラ |
| katakana-like | kl-guard-48 |  | ぱくぱく | ぱくぱく | 1 | 2 | パクパク |
| place-names | place-012 |  | ながの | 長野 | 1 | 2 | ナガノ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| place-names | place-089 |  | みやこ | 都 | 1 | 2 | みやこ |
| place-names | place-138 | 山の | ふもと | 麓 | 1 | 2 | ふもと |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-4 | 留学の | きかい | 機会 | 1 | 2 | 機械 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| rightctx | rc-long-kikai-f2 | あの | きかい | 機会 | 1 | 2 | 機械 |
| sahen-shiyou | ss-g-114 |  | じゅんびしようしょ | 準備仕様書 | 1 | 2 | 準備しようしょ |
| sahen-shiyou | ss-g-119 |  | かくにんしようちゅう | 確認使用中 | 1 | 2 | 確認しようちゅう |
| sahen-shiyou | ss-g-120 |  | れんらくしようちゅう | 連絡使用中 | 1 | 2 | 連絡しようちゅう |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-124 |  | そうじしようちゅう | 掃除使用中 | 1 | 3 | 掃除しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 2 | 運動しようちゅう |
| sahen-shiyou | ss-g-126 |  | じゅんびしようちゅう | 準備使用中 | 1 | 3 | 準備しようちゅう |
| sahen-shiyou | ss-g-127 |  | そうだんしようちゅう | 相談使用中 | 1 | 3 | 相談しようちゅう |
| segments | seg-brk-43 |  | ごじにはじまる | 五時に始まる | 1 | 2 | 5時に始まる |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 3 | 時間を空ける |
| small-kana | smallkana-02 |  | わりぃ | 悪ぃ | 1 | 2 | わりぃ |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |
| verbs | verb-noboru-02 | 階段を | のぼる | 上る | 1 | 2 | 登る |

**a1.0_m20000**: 3 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 12 | はるしねーしょんがおきる |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 2 | 運動しようちゅう |

**a1.25_m20000**: 22 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 9 | はるしねーしょんがおきる |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 家庭 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-124 |  | そうじしようちゅう | 掃除使用中 | 1 | 3 | 掃除しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 3 | 運動しようちゅう |
| sahen-shiyou | ss-g-126 |  | じゅんびしようちゅう | 準備使用中 | 1 | 3 | 準備し用中 |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |

**a1.5_m20000**: 32 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 7 | はるしねーしょんがおきる |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-124 |  | そうじしようちゅう | 掃除使用中 | 1 | 3 | 掃除しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 3 | 運動しようちゅう |
| sahen-shiyou | ss-g-126 |  | じゅんびしようちゅう | 準備使用中 | 1 | 3 | 準備しようちゅう |
| sahen-shiyou | ss-g-127 |  | そうだんしようちゅう | 相談使用中 | 1 | 2 | 相談しようちゅう |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 2 | 時間を空ける |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |

**a1.75_m20000**: 47 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-kujira-oyogu |  | くじらがおよぐ | くじらが泳ぐ | 1 | 2 | クジラが泳ぐ |
| animal-kanji | fix-nezumi-hashiru |  | ねずみがはしる | ねずみが走る | 1 | 2 | ネズミが走る |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| context-compound | ctxc-chi-ka | 地 | か | 下 | 1 | 2 | か |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| context-compound | ctxc-te-gami | 手 | がみ | 紙 | 1 | 2 | が見 |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 5 | はるしねーしょんがおきる |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-03 | まず | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-09 | 不確かな | かてい | 仮定 | 1 | 2 | 過程 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-kousei-3-02 | 受刑者の | こうせい | 更生 | 1 | 2 | 構成 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-shoukai-2-06 | 警察に前科の | しょうかい | 照会 | 1 | 2 | 紹介 |
| homophones-ext | ext-tsuikyuu-3-08 | 疑問を徹底的に | ついきゅう | 追究|追求 | 1 | 2 | 追及 |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-11 |  | むしむし | むしむし | 1 | 2 | ムシムシ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| katakana-like | kl-guard-42 |  | ぐらぐら | ぐらぐら | 1 | 2 | グラグラ |
| place-names | place-012 |  | ながの | 長野 | 1 | 2 | ナガノ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-4 | 留学の | きかい | 機会 | 1 | 2 | 機械 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| sahen-shiyou | ss-g-120 |  | れんらくしようちゅう | 連絡使用中 | 1 | 2 | 連絡しようちゅう |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-124 |  | そうじしようちゅう | 掃除使用中 | 1 | 3 | 掃除しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 3 | 運動しようちゅう |
| sahen-shiyou | ss-g-126 |  | じゅんびしようちゅう | 準備使用中 | 1 | 3 | 準備しようちゅう |
| sahen-shiyou | ss-g-127 |  | そうだんしようちゅう | 相談使用中 | 1 | 3 | 相談しようちゅう |
| segments | seg-brk-43 |  | ごじにはじまる | 五時に始まる | 1 | 2 | 5時に始まる |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 2 | 時間を空ける |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |

**a2.0_m20000**: 57 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-katatsumuri |  | かたつむり | かたつむり | 1 | 2 | カタツムリ |
| animal-kanji | fix-kujira-oyogu |  | くじらがおよぐ | くじらが泳ぐ | 1 | 2 | クジラが泳ぐ |
| animal-kanji | fix-nezumi-hashiru |  | ねずみがはしる | ねずみが走る | 1 | 2 | ネズミが走る |
| animal-kanji | fix-suzume |  | すずめ | すずめ | 1 | 2 | スズメ |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | keep-hitsuji |  | ひつじ | 羊 | 1 | 2 | ひつじ |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| context-compound | ctxc-chi-ka | 地 | か | 下 | 1 | 2 | か |
| context-compound | ctxc-hon-ya | 本 | や | 屋 | 1 | 2 | や |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| context-compound | ctxc-te-gami | 手 | がみ | 紙 | 1 | 2 | が見 |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 5 | はるしねーしょんがおきる |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 2 | もの暗示 |
| formal-noun-split | fns-leak-20 | 大きな | ものおき | 物置 | 1 | 2 | 物置き |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-26 | できる | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-03 | まず | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-09 | 不確かな | かてい | 仮定 | 1 | 2 | 過程 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-kousei-3-02 | 受刑者の | こうせい | 更生 | 1 | 2 | 構成 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-10 | 休み時間に | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-shoukai-2-06 | 警察に前科の | しょうかい | 照会 | 1 | 2 | 紹介 |
| homophones-ext | ext-tsuikyuu-3-08 | 疑問を徹底的に | ついきゅう | 追究|追求 | 1 | 2 | 追及 |
| katakana-like | kl-guard-02 |  | ぺたぺた | ぺたぺた | 1 | 2 | ペタペタ |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-11 |  | むしむし | むしむし | 1 | 2 | ムシムシ |
| katakana-like | kl-guard-14 |  | ぽつぽつ | ぽつぽつ | 1 | 2 | ポツポツ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| katakana-like | kl-guard-42 |  | ぐらぐら | ぐらぐら | 1 | 2 | グラグラ |
| katakana-like | kl-guard-48 |  | ぱくぱく | ぱくぱく | 1 | 2 | パクパク |
| place-names | place-012 |  | ながの | 長野 | 1 | 2 | ナガノ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-4 | 留学の | きかい | 機会 | 1 | 2 | 機械 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| sahen-shiyou | ss-g-120 |  | れんらくしようちゅう | 連絡使用中 | 1 | 2 | 連絡しようちゅう |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-124 |  | そうじしようちゅう | 掃除使用中 | 1 | 3 | 掃除しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 3 | 運動しようちゅう |
| sahen-shiyou | ss-g-126 |  | じゅんびしようちゅう | 準備使用中 | 1 | 3 | 準備しようちゅう |
| sahen-shiyou | ss-g-127 |  | そうだんしようちゅう | 相談使用中 | 1 | 3 | 相談しようちゅう |
| segments | seg-brk-43 |  | ごじにはじまる | 五時に始まる | 1 | 2 | 5時に始まる |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 2 | 時間を空ける |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |

**a2.5_m20000**: 70 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| animal-kanji | fix-katatsumuri |  | かたつむり | かたつむり | 1 | 2 | カタツムリ |
| animal-kanji | fix-kujira-oyogu |  | くじらがおよぐ | くじらが泳ぐ | 1 | 2 | クジラが泳ぐ |
| animal-kanji | fix-nezumi |  | ねずみ | ねずみ | 1 | 2 | ネズミ |
| animal-kanji | fix-nezumi-hashiru |  | ねずみがはしる | ねずみが走る | 1 | 2 | ネズミが走る |
| animal-kanji | fix-suzume |  | すずめ | すずめ | 1 | 2 | スズメ |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | fix-tanuki-hashiru |  | たぬきがはしる | たぬきが走る | 1 | 2 | タヌキが走る |
| animal-kanji | keep-hitsuji |  | ひつじ | 羊 | 1 | 2 | ひつじ |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| compound-affix | cmp-pos-54 |  | ひかくか | 非核化 | 1 | 2 | 比較か |
| context-compound | ctxc-chi-ka | 地 | か | 下 | 1 | 2 | か |
| context-compound | ctxc-hon-ya | 本 | や | 屋 | 1 | 2 | や |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| context-compound | ctxc-te-gami | 手 | がみ | 紙 | 1 | 3 | が見 |
| counters | fix-san-shuu | 三 | しゅう | 週 | 1 | 2 | 州 |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 4 | はるしねーしょんがおきる |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 3 | もの暗示 |
| formal-noun-split | fns-leak-13 |  | くせじ | 癖字 | 1 | 2 | くせじ |
| formal-noun-split | fns-leak-20 | 大きな | ものおき | 物置 | 1 | 2 | 物置き |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-26 | できる | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| hiragana | hira-kan-09 | 資料を | ください | 下さい | 1 | 2 | ください |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-03 | まず | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-09 | 不確かな | かてい | 仮定 | 1 | 2 | 過程 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-kousei-3-02 | 受刑者の | こうせい | 更生 | 1 | 2 | 構成 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-10 | 休み時間に | こうてい | 校庭 | 1 | 3 | 行程 |
| homophones-ext | ext-shoukai-2-06 | 警察に前科の | しょうかい | 照会 | 1 | 2 | 紹介 |
| homophones-ext | ext-tsuikyuu-3-08 | 疑問を徹底的に | ついきゅう | 追究|追求 | 1 | 2 | 追及 |
| katakana-like | kl-guard-01 |  | ぷんぷん | ぷんぷん | 1 | 2 | プンプン |
| katakana-like | kl-guard-02 |  | ぺたぺた | ぺたぺた | 1 | 2 | ペタペタ |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-05 |  | ぱちぱち | ぱちぱち | 1 | 2 | パチパチ |
| katakana-like | kl-guard-11 |  | むしむし | むしむし | 1 | 3 | ムシムシ |
| katakana-like | kl-guard-14 |  | ぽつぽつ | ぽつぽつ | 1 | 2 | ポツポツ |
| katakana-like | kl-guard-23 |  | ぺこぺこ | ぺこぺこ | 1 | 2 | ペコペコ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| katakana-like | kl-guard-42 |  | ぐらぐら | ぐらぐら | 1 | 2 | グラグラ |
| katakana-like | kl-guard-48 |  | ぱくぱく | ぱくぱく | 1 | 2 | パクパク |
| place-names | place-012 |  | ながの | 長野 | 1 | 2 | ナガノ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| place-names | place-089 |  | みやこ | 都 | 1 | 2 | みやこ |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-4 | 留学の | きかい | 機会 | 1 | 2 | 機械 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| sahen-shiyou | ss-g-119 |  | かくにんしようちゅう | 確認使用中 | 1 | 2 | 確認しようちゅう |
| sahen-shiyou | ss-g-120 |  | れんらくしようちゅう | 連絡使用中 | 1 | 2 | 連絡しようちゅう |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-124 |  | そうじしようちゅう | 掃除使用中 | 1 | 4 | 掃除しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 3 | 運動しようちゅう |
| sahen-shiyou | ss-g-126 |  | じゅんびしようちゅう | 準備使用中 | 1 | 3 | 準備しようちゅう |
| sahen-shiyou | ss-g-127 |  | そうだんしようちゅう | 相談使用中 | 1 | 3 | 相談しようちゅう |
| segments | seg-brk-43 |  | ごじにはじまる | 五時に始まる | 1 | 2 | 5時に始まる |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 3 | 時間を空ける |
| small-kana | smallkana-02 |  | わりぃ | 悪ぃ | 1 | 2 | わりぃ |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |
| verbs | verb-noboru-02 | 階段を | のぼる | 上る | 1 | 2 | 登る |

**a3.0_m20000**: 81 件

| セット | id | 左文脈 | 読み | 期待 | 基準の順位 | 新しい順位 | 新しい 1 位 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| abr-towns | abr-027 | 髪に | くし | 櫛 | 1 | 2 | くし |
| animal-kanji | fix-katatsumuri |  | かたつむり | かたつむり | 1 | 2 | カタツムリ |
| animal-kanji | fix-kujira-oyogu |  | くじらがおよぐ | くじらが泳ぐ | 1 | 2 | クジラが泳ぐ |
| animal-kanji | fix-nezumi |  | ねずみ | ねずみ | 1 | 2 | ネズミ |
| animal-kanji | fix-nezumi-hashiru |  | ねずみがはしる | ねずみが走る | 1 | 2 | ネズミが走る |
| animal-kanji | fix-suzume |  | すずめ | すずめ | 1 | 2 | スズメ |
| animal-kanji | fix-suzume-2wa |  | すずめが2わ | すずめが2羽 | 1 | 2 | スズメが2羽 |
| animal-kanji | fix-tanuki-hashiru |  | たぬきがはしる | たぬきが走る | 1 | 2 | タヌキが走る |
| animal-kanji | keep-hitsuji |  | ひつじ | 羊 | 1 | 2 | ひつじ |
| animal-kanji | keep-tanuki-neiri |  | たぬきねいり | 狸寝入り | 1 | 2 | たぬき寝入り |
| animal-kanji | rem-fukurou |  | ふくろう | ふくろう | 1 | 2 | フクロウ |
| animal-kanji | rem-kitsune-to-tanuki |  | きつねとたぬき | きつねとたぬき | 1 | 2 | キツネとタヌキ |
| compound-affix | cmp-pos-30 |  | きのうほう | 帰納法 | 1 | 2 | 機能法 |
| compound-affix | cmp-pos-54 |  | ひかくか | 非核化 | 1 | 2 | 比較か |
| context-compound | ctxc-chi-ka | 地 | か | 下 | 1 | 3 | か |
| context-compound | ctxc-hon-ya | 本 | や | 屋 | 1 | 2 | や |
| context-compound | ctxc-juuyou-shi | 重要 | し | 視 | 1 | 2 | し |
| context-compound | ctxc-te-gami | 手 | がみ | 紙 | 1 | 5 | が見 |
| counters | fix-san-shuu | 三 | しゅう | 週 | 1 | 2 | 州 |
| english-first | ef-hallucination-02 | LLMの | はるしねーしょんがおきる | ハルシネーションが起きる | 1 | 3 | はるしねーしょんがおきる |
| formal-noun-split | fns-guard-03 |  | ものをおく | 物を置く | 1 | 2 | ものを置く |
| formal-noun-split | fns-guard-07 |  | ものはためし | 物は試し | 1 | 2 | ものは試し |
| formal-noun-split | fns-leak-10 |  | ものあんじ | 物案じ | 1 | 3 | もの暗示 |
| formal-noun-split | fns-leak-13 |  | くせじ | 癖字 | 1 | 2 | くせじ |
| formal-noun-split | fns-leak-20 | 大きな | ものおき | 物置 | 1 | 2 | 物置き |
| funhen | fh-ketsuron-c | 少ない例から全体を決めてしまうのは、早まった一般化だ。 | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| funhen | fh-ketsuron-s |  | けつろんづけてしまう | 結論付けてしまう | 1 | 2 | 結論づけてしまう |
| hiragana | hira-conj-15 | 資料を送付します。 | おって | おって | 1 | 2 | 追って |
| hiragana | hira-fn-25 | 安い | ほうが | ほうが | 1 | 2 | 方が |
| hiragana | hira-fn-26 | できる | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-fn-27 | 見る | かぎり | かぎり | 1 | 2 | 限り |
| hiragana | hira-kan-08 | お茶を | ください | 下さい | 1 | 2 | ください |
| hiragana | hira-kan-09 | 資料を | ください | 下さい | 1 | 2 | ください |
| hiragana | hira-kan-20 |  | すべて | 全て | 1 | 2 | すべて |
| homophones-ext | ext-hoshou-2-06 | 障害者の | ほしょう | 保障 | 1 | 2 | 補償 |
| homophones-ext | ext-kaitou-2-03 | 試験の | かいとう | 解答 | 1 | 2 | 回答 |
| homophones-ext | ext-katei-3-03 | まず | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-08 | そのように | かてい | 仮定 | 1 | 2 | 家庭 |
| homophones-ext | ext-katei-3-09 | 不確かな | かてい | 仮定 | 1 | 2 | 過程 |
| homophones-ext | ext-katei-4-07 | 三年間の | かてい | 課程 | 1 | 3 | 過程 |
| homophones-ext | ext-kousei-3-02 | 受刑者の | こうせい | 更生 | 1 | 2 | 構成 |
| homophones-ext | ext-koushou-2-07 | 遺跡の | こうしょう | 考証 | 1 | 2 | 公称 |
| homophones-ext | ext-koutei-3-04 | 雨上がりの | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-05 | 運動会の | こうてい | 校庭 | 1 | 2 | 行程 |
| homophones-ext | ext-koutei-3-10 | 休み時間に | こうてい | 校庭 | 1 | 3 | 行程 |
| homophones-ext | ext-shoukai-2-06 | 警察に前科の | しょうかい | 照会 | 1 | 2 | 紹介 |
| homophones-ext | ext-shuukan-3-04 | 新聞の | しゅうかん | 週刊 | 1 | 2 | 週間 |
| homophones-ext | ext-taishou-3-03 | 線 | たいしょう | 対称 | 1 | 2 | 対象 |
| homophones-ext | ext-tsuikyuu-3-08 | 疑問を徹底的に | ついきゅう | 追究|追求 | 1 | 2 | 追及 |
| katakana-like | kl-guard-01 |  | ぷんぷん | ぷんぷん | 1 | 2 | プンプン |
| katakana-like | kl-guard-02 |  | ぺたぺた | ぺたぺた | 1 | 2 | ペタペタ |
| katakana-like | kl-guard-04 |  | ふくろう | 梟|ふくろう | 1 | 2 | フクロウ |
| katakana-like | kl-guard-05 |  | ぱちぱち | ぱちぱち | 1 | 2 | パチパチ |
| katakana-like | kl-guard-11 |  | むしむし | むしむし | 1 | 3 | ムシムシ |
| katakana-like | kl-guard-14 |  | ぽつぽつ | ぽつぽつ | 1 | 2 | ポツポツ |
| katakana-like | kl-guard-23 |  | ぺこぺこ | ぺこぺこ | 1 | 2 | ペコペコ |
| katakana-like | kl-guard-37 |  | ぽろぽろ | ぽろぽろ | 1 | 2 | ポロポロ |
| katakana-like | kl-guard-42 |  | ぐらぐら | ぐらぐら | 1 | 2 | グラグラ |
| katakana-like | kl-guard-48 |  | ぱくぱく | ぱくぱく | 1 | 2 | パクパク |
| place-names | place-012 |  | ながの | 長野 | 1 | 2 | ナガノ |
| place-names | place-083 |  | さくら | 桜 | 1 | 2 | さくら |
| place-names | place-089 |  | みやこ | 都 | 1 | 2 | みやこ |
| place-names | place-138 | 山の | ふもと | 麓 | 1 | 2 | ふもと |
| rare-homophones | rh-neg-25 |  | さいかいをだっしゅつ | 最下位を脱出 | 1 | 2 | 再会を脱出 |
| rightctx | rc-long-dis-4 | 留学の | きかい | 機会 | 1 | 2 | 機械 |
| rightctx | rc-long-dis-6 | 週末は | こうえん | 公園 | 1 | 2 | 講演 |
| rightctx | rc-long-kikai-f2 | あの | きかい | 機会 | 1 | 2 | 機械 |
| sahen-shiyou | ss-g-114 |  | じゅんびしようしょ | 準備仕様書 | 1 | 2 | 準備しようしょ |
| sahen-shiyou | ss-g-119 |  | かくにんしようちゅう | 確認使用中 | 1 | 2 | 確認しようちゅう |
| sahen-shiyou | ss-g-120 |  | れんらくしようちゅう | 連絡使用中 | 1 | 2 | 連絡しようちゅう |
| sahen-shiyou | ss-g-123 |  | べんきょうしようちゅう | 勉強使用中 | 1 | 3 | 勉強しようちゅう |
| sahen-shiyou | ss-g-124 |  | そうじしようちゅう | 掃除使用中 | 1 | 5 | 掃除しようちゅう |
| sahen-shiyou | ss-g-125 |  | うんどうしようちゅう | 運動使用中 | 1 | 3 | 運動しようちゅう |
| sahen-shiyou | ss-g-126 |  | じゅんびしようちゅう | 準備使用中 | 1 | 4 | 準備しようちゅう |
| sahen-shiyou | ss-g-127 |  | そうだんしようちゅう | 相談使用中 | 1 | 4 | 相談しようちゅう |
| segments | seg-brk-43 |  | ごじにはじまる | 五時に始まる | 1 | 2 | 5時に始まる |
| segments | seg-part-12 |  | じかんをあける | 時間を開ける | 1 | 3 | 時間を空ける |
| small-kana | smallkana-02 |  | わりぃ | 悪ぃ | 1 | 2 | わりぃ |
| verbs | verb-arawasu-02 | 姿を | あらわした | 現した | 1 | 2 | 表した |
| verbs | verb-kaku-09 |  | かく | 書く | 1 | 2 | 各 |
| verbs | verb-noboru-02 | 階段を | のぼる | 上る | 1 | 2 | 登る |

### 表 E: 見張りのケース（かんさつしよう、期待 観察しよう。順位 / 1 位の表記）

| 組み合わせ | 左文脈「」 | 左文脈「#3はもう少し」 | 左文脈「もう少し」 |
| --- | --- | --- | --- |
| LM なし | 1 / 観察しよう | 2 / 観察使用 | 2 / 観察使用 |
| a1.0_m6000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a1.25_m6000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a1.5_m6000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a1.75_m6000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a2.0_m6000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a2.5_m6000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a3.0_m6000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a1.0_m12000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a1.25_m12000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a1.5_m12000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a1.75_m12000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a2.0_m12000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a2.5_m12000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a3.0_m12000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a1.0_m20000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a1.25_m20000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a1.5_m20000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a1.75_m20000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a2.0_m20000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a2.5_m20000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
| a3.0_m20000 | 1 / 観察しよう | 1 / 観察しよう | 1 / 観察しよう |
