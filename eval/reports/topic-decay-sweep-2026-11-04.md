# topic.decay の掃引（#178、2026-11-04、main cb7da09）

方法: `ikari-eval --ranker full --user-dir <空> --ranker-config <topic.decay=値> --repeat 1`、eval/sets/*.tsv の 43 セット + AJIMEE（200）。carry-* の 6 セットだけ `--carry-fill 40 --carry-joint period`（snapshot と同じ）。LM ありは `--features lm-metal`、zenz-v3.2-small Q5_K_M、`--lm-budget-ms 3000 --lm-deadline-ms 3000`（時間切れ 0）。

## 1 位の一致数（LM なし。変化のあるセットだけ）
| set | 0.97 | 0.9 | 0.8 | 0.7 | 0.5 | 0.3 |
|---|---|---|---|---|---|---|
| ajimee（200） | 52 | 50 | 50 | 51 | 51 | 51 |
| homophones-ext | 994 | 994 | 994 | 991 | 991 | 984 |
| place-names | 146 | 146 | 145 | 145 | 145 | 145 |
| issue91 | 5 | 5 | 5 | 5 | 4 | 3 |
| issue135 | 4 | 4 | 4 | 4 | 4 | 5 |
| carry-chat | 51 | 50 | 50 | 50 | 46 | 45 |
| carry-empty | 883 | 883 | 881 | 878 | 878 | 872 |
| carry-empty-unrelated | 606 | 611 | 614 | 623 | 621 | 626 |
| carry-mismatch | 994 | 994 | 994 | 991 | 991 | 984 |
| carry-overlap | 1078 | 1075 | 1064 | 1050 | 1025 | 1009 |
| carry-unrelated | 816 | 840 | 882 | 905 | 951 | 970 |
| 全体 | 7171 | 7194 | 7225 | 7235 | 7249 | 7236 |
| 基準から 1 位が悪化 / 改善（件） | - | 25 / 48 | 54 / 108 | 88 / 152 | 147 / 225 | 196 / 261 |

ほかのセット（abr-towns、adverbs、hiragana、segments ほか）は全 decay で同じ。

## LM あり（3000ms）
| set | 0.97 | 0.8 | 0.7 |
|---|---|---|---|
| ajimee | 111 | 111 | 111 |
| homophones-ext | 1119 | 1120 | 1119 |
| carry-chat | 64 | 63 | 62 |
| carry-empty / -empty-unrelated / -mismatch / -overlap | 938 / 622 / 1119 / 1173 | 936 / 623 / 1120 / 1170 | 937 / 622 / 1119 / 1171 |
| carry-unrelated | 997 | 1022 | 1035 |
| 全体 | 7987 | 8009 | 8020 |
| 悪化 / 改善（件） | - | 29 / 51 | 36 / 69 |

## 悪化の具体例（LM なし。ケース ID。左文脈 / 読み / 期待 -> 1 位）
- 0.9 の ajimee: ajimee-1189（これら喪失領土や、/ せんりょうちにいた / 占領地 -> 占領値）、ajimee-2108（皮パン / をはいて… / 単気筒 -> 短期等）。0.8 以下でも同じ。
- 0.9 の carry-unrelated: ext-kouka-1-06（運動の / こうか / 効果 -> こうか）、ext-idou-1-10（資料を / いどう / 移動 -> 居どう）。
- 0.9 の carry-empty-unrelated: ext-hoshou-1-03、ext-kikou-2-07（機構 -> 木こう）など 7 件。
- 0.8 以下: place-136（雨の日は / かさ / 傘 -> カサ）。
- 0.7 以下: carry-empty の ext-kousei-2-08（校正 -> 構成）、ext-kaitou-2-10、ext-kikai-2-06、ext-katei-3-05 など。0.5 以下で carry-chat の chat-08-o、chat-15-o、chat-19-o。
- LM あり 0.7: ext-souzou-1-04/05（想像 -> 創造）、ext-shinkou-2-03、ext-katei-2-07、homophones-ext の ext-kaihou-3-01（介抱 -> 解放）、chat-12-e、chat-25-e。

## #178 の例（読み ひんし。T = 技術文、G = ゲーム文。T×3 + G は期待 瀕死、G×3 + T は期待 品詞）
上は LM なし。LM あり（3000ms）も同じ 1 位。長文は T×30（701 字）+ G、G×60（743 字）+ T。
| decay | T×3→G | G×3→T | T×30→G | G×60→T |
|---|---|---|---|---|
| 0.97 | 品詞 NG | 瀕死 NG | 品詞 NG | 瀕死 NG |
| 0.9 | 瀕死 | 瀕死 NG | 瀕死 | 瀕死 NG |
| 0.8 | 瀕死 | 瀕死 NG | 瀕死 | 品詞 |
| 0.7 | 瀕死 | 品詞 | 瀕死 | 品詞 |
| 0.5 | 瀕死 | 品詞 | 瀕死 | 品詞 |
1 文ずつの近い例: T→G（期待 瀕死）は 0.97 でも OK。G→T（期待 品詞）は 0.97、0.9 で NG（瀕死）、0.8 以下で OK。
