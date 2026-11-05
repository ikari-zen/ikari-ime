# 読みそのままのかなの救済（ADR 0052、#207）の評価

日付: 2026-11-06。コード: `feat/kana-rescue`。辞書: `data/build/dict`。学習なし。基準は同じコードで `convert.kana_rescue_rank=0`（救済なし = main と同じ並び）。

## 1. kana-rescue セット（`eval/sets/kana-rescue.tsv`、74 行 = 37 の読み × 平仮名・片仮名）

合格条件は順位 30 以下。読みは、外来語らしい長い読みを救済なしで流し、かなが 30 位より後ろのものを選んだ（公開の語だけ。期待はユーザーの確認待ち）。ばーじょにんぐ は文脈なしと「APIの」の 2 通り。

| 構成 | 30 位より後ろ | 最大順位 | 動いた行 |
| --- | --- | --- | --- |
| LM なし 救済なし | 40 / 74 | 37 | - |
| LM なし 救済あり | 0 / 74 | 30 | 40 |
| LM あり（締め切り 3000ms、時間切れ 0）救済なし | 21 / 74 | 37 | - |
| LM あり 救済あり | 0 / 74 | 30 | 21 |

ばーじょにんぐ（平仮名 / 片仮名の順位）:

| 文脈 | LM なし 救済なし | LM なし 救済あり | LM あり 救済なし | LM あり 救済あり |
| --- | --- | --- | --- | --- |
| なし | 31 / 32 | 29 / 30 | 16 / 3 | 16 / 3 |
| APIの | 31 / 32 | 29 / 30 | 32 / 3 | 30 / 3 |

## 2. 全セットで 1 位が悪化しないこと（`eval/scripts/snapshot.sh --compare`、LM あり・なし、AJIMEE を含む）

- 基準は救済なし（`IKARI_SNAPSHOT_FLAGS="--ranker-config convert.kana_rescue_rank=0"`）で取った snapshot。
- 1 位の悪化: 0 件。LM の時間切れ: 0。シナリオ、CLI の golden、区切りのレポートはすべて一致。
- 全 18739 行で順位が変わったのは 2 行だけ。
  - 下がった: `issue185-full` の `i185-full-n`（30 位 → 32 位。ADR 0052 §3.3 の、29・30 位から押し出される型）。
  - 上がった: `adverbs-full` の `adv-pred-11`（どうしょうもない の平仮名。31 位 → 29 位）。
- snapshot の対象に kana-rescue セットは足していない（合否が 1 位ではなく順位 30 以下なので、1 位の集計に混ぜない）。§1 の流し方で確かめる。

## 3. 再現

```sh
cargo build --release -p ikari-eval --features lm-metal
# 救済なしは --ranker-config に convert.kana_rescue_rank=0 の 1 行のファイルを渡す
ikari-eval --ranker full --set eval/sets/kana-rescue.tsv --out-cases out.tsv --repeat 1 \
  --lm "$HOME/Library/Application Support/ikari-ime/models/zenz-v3.2-small-Q5_K_M.gguf" --lm-budget-ms 3000 --lm-deadline-ms 3000
```
