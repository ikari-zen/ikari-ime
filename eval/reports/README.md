# eval のレポートの索引

ADR が根拠として参照しているレポートは、リンクを壊さないために場所を変えない。参照がなく、もう再現もできないものだけを `archive/` に置く。
今のコードの振る舞いが変わっていないことは、レポートの数値の比較ではなく `eval/scripts/snapshot.sh`（`scripts/check.sh --eval <基準>`）で確かめる。

区分:

- **現行の基準**: 今の回帰の数値や、`--baseline` に使うファイルの出どころ。
- **判断の根拠（凍結）**: ADR が参照している。当時の数値を残すもので、書き直さない。
- **archive**: 参照がなく、再現できない。

| ファイル | 日付 | 区分 | 何の数値か | 今のコードで再現できるか |
| --- | --- | --- | --- | --- |
| [katakana-k1-sweep.md](katakana-k1-sweep.md) | 2026-10-29 | 判断の根拠（凍結） | K1（未知語のカタカナの上限 `base + per_char × 文字数`）の掃引（[ADR 0037](../../docs/adr/0037-unknown-katakana-cost.md) §7 の 3）。格子 404 点 + 外の 56 点を LM なしで、悪化 0 の上位 8 点を LM ありで測定。全セット・AJIMEE・疑似の守る側で 1 位の悪化 0 の点、狙う群の 1 位の増加、ばーじょにんぐ の順位、ノイズ（守る群 6864 読み）、test 1 回。既定値は決めていない | 再現できる（§10 の手順。`eval/scripts/katakana_k1_sweep_*`。S0 の絞った辞書が要る） |
| [katakana-s0.md](katakana-s0.md) | 2026-10-29 | 判断の根拠（凍結） | 辞書にないカタカナ語の候補のコスト（[ADR 0037](../../docs/adr/0037-unknown-katakana-cost.md) §6.1 S0）。leave-one-out の疑似の未知語 8698 件、本物の未知語、守る群 6858 読みの「辞書の最小コスト ÷ 文字数」の分布、K1 の格子の見込み、K6（並びの表）の分離（AUC、サイズ、判定の時間）、LM ありの並べ替えが 31〜32 位に掛かるか。値は決めていない | 再現できる（§10 の手順。`eval/scripts/katakana_s0_*`。除外表で絞った辞書を作る） |
| [path-loses-250.md](path-loses-250.md) | 2026-10-25 | 判断の根拠（凍結） | 経路が負ける型（看守役、異常気象、設計仕様変更、推し活）のコスト補正（[ADR 0035](../../docs/adr/0035-path-loses-cost-adjust.md) の規則 F・F2・H）の掃引。全 30 セットの 1 位（LM なし・あり 2000ms）、path-loses セット（39 件）、学習のシナリオ、当たった行数（#250） | 採用の点は再現できる（`scripts/build-dict.sh`）。ほかの点はスパイクのコード（コミットしていない）で、定数を書き換えて作る |
| [prefetch-s1.md](prefetch-s1.md) | 2026-10-29 | 判断の根拠（凍結） | 先行計算の左文脈の伸び（[ADR 0038](../../docs/adr/0038-prefetch-context-tolerance.md) S1）。文脈の末尾から確定の単位を取り除いたセットで、X（今）・Y（案 1）・Z（案 4 の上限）の 1 位を比較。切り方 A・B、セット別、単位の長さ別、`commit_chars` の重み付き。Y 対 X は基準を満たさない | 再現できる（`eval/scripts/prefetch_s1/`。実機の dict と zenz が要る。コマンドはレポートの付録） |
| [phase2-q.md](phase2-q.md) | 2026-10-12 | 現行の基準 | フェーズ2の変換の質と締め切り。§9 の表が今の回帰の数値（homophones、ext、hiragana、AJIMEE。LM なし／あり） | 再現できる。ただし §8 の `eos_weight` の実験は、削除前のコミット 6a58b73 で |
| [phase3-segments.md](phase3-segments.md) | 2026-10-13 | 現行の基準（雛形。数値は lane C のマージ後） | 文節の区切りの F1、打鍵数（今の UI と文節の UI）、再変換の読みの推定の一致率（[ADR 0006](../../docs/adr/0006-segment-editing.md) §7）。segments セット（145 件） | lane C のマージ後に再現できる（`ikari-eval segments` / `ops` / `reading`。コマンドはレポートの §2） |
| [phase3-compound.md](phase3-compound.md) | 2026-10-13 | 現行の基準 | 機能語の装飾表記（YO、〜）、読みに対応しない記号の固定の規則、漢数字のゼロ、複合語のコスト（具体例）の前後。5つのセット、区切りの F1、再変換の読みの推定 | 再現できる（`scripts/build-dict.sh`。基準の辞書は上の変更を戻して作る） |
| [phase3-context-compound.md](phase3-context-compound.md) | 2026-10-13 | 判断の根拠（凍結） | 左文脈の最後の語 + 候補 が辞書の1語になるときの加点（[ADR 0007](../../docs/adr/0007-left-context-compound.md) のスパイク）。規則の形と値の dev / test、全セットの snapshot、セットにない入力 135 件、速さ | 規則を実装したあと再現できる（スパイクのコードはコミットしていない） |
| [phase3-hyphen.md](phase3-hyphen.md) | 2026-10-13 | 現行の基準 | 和語・漢語の途中のハイフン（す‐ぐ）の固定の規則（`hyphen_inside_native_word`）。ハイフンとダッシュの類の調査（476 行）と snapshot の前後（全セット一致） | 再現できる（`scripts/build-dict.sh`。基準の辞書は main f9ebf3a の `dict-builder` で作る） |
| [phase3-suffix.md](phase3-suffix.md) | 2026-10-13 | 現行の基準 | 接尾の「化」、副詞「こう」の分割、文脈のあとの1文字の接尾（器/機、板/版、次/時）の原因の分解。`frequent_compound` の順位の上限（3万 → 4万）の調整と、採らなかった規則（`sahen_suffix`、`short_kana_adverb`）の実験。全セットの前後 | 再現できる（`scripts/build-dict.sh`。基準の辞書は `COMPOUND_MAX_RANK` を 30_000 に戻して作る） |
| [phase3-adverb-noun.md](phase3-adverb-noun.md) | 2026-10-13 | 判断の根拠（凍結） | 副詞の直後の名詞（そう時期、こう班、もう府）のスパイク。並びへの一律の加算と、1語で覆う名詞があるときだけの加算の比較。全セットと自然な入力 174 件の前後、速さ（[ADR 0008](../../docs/adr/0008-adverb-noun-connection.md) の根拠） | スパイクのコードはコミットしていない。案 C の本実装のあとに、本実装で取り直す |
| [phase3-verb-notation.md](phase3-verb-notation.md) | 2026-10-14 | 判断の根拠（凍結） | 同音の動詞の表記（異字同訓）の原因の測定と、辞書のコストを書き換えたスパイク（表外の表記、lemma の中の既定、読みの組の既定、機能語のカタカナ）。verbs.tsv と全セットの LM なし・あり（[ADR 0009](../../docs/adr/0009-verb-notation.md) の根拠） | スパイクのコードはコミットしていない（§1.2 の手順で再現できる）。本実装で取り直した値は §5 |
| [phase3-compound-split.md](phase3-compound-split.md) | 2026-10-14 | 判断の根拠（凍結） | 辞書に1語で入っている熟語（形態素）が、同音の語＋1文字の接辞（携帯＋素）に負ける問題のスパイク（[ADR 0011](../../docs/adr/0011-compound-over-affix-split.md) の根拠）。原因の分解、辞書から探した 3457 語、`frequent_compound` の条件とラティスの辺の案の比較、全セットと LM ありの前後、Full 版の確認 | スパイクのコードはコミットしていない。本実装のあとに取り直す |
| [phase3-learning-no-context.md](phase3-learning-no-context.md) | 2026-10-14 | 判断の根拠（凍結） | 文脈の記録がないときの学習（Unconditional の重み、back-off、短期の記憶など）の試作と、学習のシナリオ（LM あり・なし）、使いながら学習する流し方（homophones-ext ほか）の前後（[ADR 0013](../../docs/adr/0013-learning-without-context.md) の根拠） | 試作のコードはコミットしていない。本実装で取り直した値は [ADR 0013](../../docs/adr/0013-learning-without-context.md) §11（一部、辞書の変更で値が違う） |
| [phase3-full-dictionary.md](phase3-full-dictionary.md) | 2026-10-14 | 判断の根拠（凍結） | SudachiDict Full を system.dict にする案と、chiVe にない語のベクトルを Sudachi の分割から作る案（分解）の試作。全セット、new-words、文脈つきの複合語、性能、左文脈の解析の変化（全件）、頻度上位 3000 語と自然な入力 120 件の固有名詞の混じり方（LM なし・既定の締め切り・2000ms、全件の一覧）（[ADR 0012](../../docs/adr/0012-new-word-dictionary.md) §3.4、§12〜§14 の根拠） | 試作のコードはコミットしていない。本実装のあとに取り直す |
| [phase3-katakana.md](phase3-katakana.md) | 2026-10-12 | 現行の基準 | カタカナの普通名詞（メイ、ココ）が接尾の漢字（名）に勝つ問題の修正（`katakana_analysis`）。katakana-suffix セット（145 件）の前後と、回帰の数値（homophones、ext、hiragana、2000 文字の p99） | 再現できる（`scripts/build-dict.sh` で辞書を作り直す。基準の辞書はポリシーから `katakana_analysis` の行を除いて作る） |
| [funhen-138.md](funhen-138.md) | 2026-10-23 | 現行の基準 | 記事「糞変換シリーズ」の例（[funhen.tsv](../sets/funhen.tsv)、22 行 = 11 例 × 単独・文脈）の今の結果（LM なし 6 / 22、LM あり 11 / 22）と、B57〜B61 のどれに当たるかの見立て。期待はユーザーの確認待ち（#138） | 再現できる（`ikari-eval --ranker full --set eval/sets/funhen.tsv`。LM ありは §2） |
| [phase1-hiragana.md](phase1-hiragana.md) | 2026-10-11 | 現行の基準 | ひらがな／漢字の表記（公用文）。hiragana セットの基準 135 / 140（[ADR 0002](../../docs/adr/0002-phase1-architecture.md)） | 再現できる |
| [phase0-baseline-cases.tsv](phase0-baseline-cases.tsv) | 2026-10-11 | 現行の基準 | `--baseline` に使うケース別の順位（h-01 / a-01 の回帰の確認。[phase1-gates.md](phase1-gates.md) が使う） | そのまま使う |
| [phase0-baseline.md](phase0-baseline.md) | 2026-10-11 | 判断の根拠（凍結） | フェーズ0の基準（cost のみ。homophones 3 / 14）。[ADR 0002](../../docs/adr/0002-phase1-architecture.md) の前提 | 再現できる（`--ranker cost`） |
| [phase1-quality.md](phase1-quality.md) | 2026-10-11 | 判断の根拠（凍結） | 変換の質の調査と調整（ext の dev で値を決めた過程）。[ADR 0003](../../docs/adr/0003-topic-ranker-decision.md) の根拠 | 一部。`rare_lemma` は削除前のコミット 6a58b73 で |
| [phase1-topic.md](phase1-topic.md) | 2026-10-11 | 判断の根拠（凍結） | 話題スコアの比較実験（chiVe と zenz）。ADR 0003 の根拠 | 再現できる（LM は `--features lm-metal`） |
| [phase1-gates.md](phase1-gates.md) | 2026-10-11 | 判断の根拠（凍結） | フェーズ1の完了条件の確認（[ADR 0002](../../docs/adr/0002-phase1-architecture.md) §17.4） | 再現できる |
| [phase1-ajimee.md](phase1-ajimee.md) | 2026-10-11 | 判断の根拠（凍結） | AJIMEE-Bench（200 件）の結果。ADR 0003 の根拠 | 再現できる（`scripts/fetch-data.sh --only ajimee`） |
| [phase1-lm-compare.md](phase1-lm-compare.md) | 2026-10-11 | 判断の根拠（凍結） | kanakanji-edge-33m と zenz-v3.2 の比較。[ADR 0003 §7.1](../../docs/adr/0003-topic-ranker-decision.md)-1 の根拠 | `ikari-eval gen` と `--lm-kind` は削除済み。削除前のコミット 6a58b73 で。edge は `zenz.model_kind` で選べる |
| [archive/phase1-r1-policy.md](archive/phase1-r1-policy.md) | 2026-10-11 | archive | R1 の部分評価（`--bake-policy` と `dict-r1-baked`。暫定） | できない（つまみを削除した） |
| [archive/dict-builder-phase1-policy.md](archive/dict-builder-phase1-policy.md) | 2026-10-11 | archive | 辞書ポリシーのレポート（除外 21224 件の時点） | できない（ポリシーを変えた）。今のレポートは `data/build/dict-policy-report.md`（`scripts/build-dict.sh` が毎回出す） |

評価セットの確認の記録（[REVIEW.md](../sets/REVIEW.md)）は、評価セットと一緒に `eval/sets/` に置いている。

## LM ありの評価の測り方（ADR 0021 L3）

- `ikari-eval` は、`--lm` を付けると、最初の採点の前に LM を 1 回暖める（モデルの読み込みと Metal の初期化。`ZenzRanker::warm_up`）。暖機は呼び出し数・時間切れの統計に入らず、評価の数字に混ざらない。
- 締め切りは `--lm-budget-ms N --lm-deadline-ms N` で指定する（新しい欄は足していない）。省略すると、変換を始めた瞬間から `30ms + 6ms × 読みの文字数` で数える。これは実機のエンジンより短い（実機は打っている間の先行計算で LM を最大 300ms 待つ）。
  - 回帰の比較（`snapshot.sh`）: `2000`（時間切れ 0 を確かめる）。
  - エンジンを模す: 「Space を押すまでの間」の p10 = 100ms なら `--lm-budget-ms 100 --lm-deadline-ms 100`。
- 時間切れはマシンの混み具合で揺れる（ADR 0021 §2.4）。LM ありの数字には、`uptime` の load average を添える。
