# カバレッジの記録

- 日付: 2026-10-31
- コミット: ee86296（作業ツリーに未コミットの変更あり）
- 測り方: `cargo xtask cov` で測定。`cargo llvm-cov --workspace`（xtask を除く）、集計の除外 `(xtask|/tests/|tests\.rs|benches|build\.rs|generated|\.pb\.rs)`、行カバレッジ
- 機能つき（llama.cpp, Metal）: 含む
- 実データ（check --real のテスト）: 含む
- 対象: 全体
- 失敗した実データのテスト（測定は続け、実行した分は数えている）: --release -p ikari-core --test segment_real -- --ignored、--release -p ikari-core --test left_context_real -- --ignored、-p ikari-engine --features lm-metal -- --ignored、-p ikari-lm --features metal --test real_model --test real_model_edge -- --ignored

## crate ごと（行カバレッジ。低い順）

| crate | グループ | 行数 | 未実行 | カバレッジ |
| --- | --- | ---: | ---: | ---: |
| lm | 変換 | 956 | 126 | 86.8% |
| stack |  | 633 | 68 | 89.3% |
| cli |  | 1375 | 129 | 90.6% |
| shell-ffi |  | 1938 | 178 | 90.8% |
| dict-builder |  | 6479 | 588 | 90.9% |
| eval |  | 4395 | 321 | 92.7% |
| import |  | 1721 | 106 | 93.8% |
| engine-ffi |  | 178 | 7 | 96.1% |
| shell-core |  | 12652 | 495 | 96.1% |
| engine | 変換 | 4384 | 126 | 97.1% |
| dict-format | 変換 | 2131 | 60 | 97.2% |
| core | 変換 | 15042 | 410 | 97.3% |
| input | 変換 | 740 | 15 | 98.0% |
| **変換のロジックの小計**（core、engine、input、lm、dict-format） |  | 23253 | 737 | 96.8% |
| **全体** |  | 52624 | 2629 | 95.0% |

## 低いファイル（60 行以上で 70% 未満。未実行の行が多い順）

| ファイル | 行数 | 未実行 | カバレッジ |
| --- | ---: | ---: | ---: |
| eval/src/main.rs | 426 | 152 | 64.3% |
| dict-builder/src/main.rs | 117 | 117 | 0.0% |
| dict-builder/src/verify.rs | 307 | 115 | 62.5% |
| lm/src/llama.rs | 287 | 95 | 66.9% |
| cli/src/session.rs | 207 | 80 | 61.4% |
| eval/src/segment_engine.rs | 188 | 77 | 59.0% |

## メモ（手で足した）

- 条件: `cargo xtask cov --real`（`--lm` を含む。Metal）。辞書とモデルは、メインのチェックアウトの `data/` を環境変数（`IKARI_DICT_DIR`、`IKARI_ZENZ_MODEL`、`IKARI_EDGE_MODEL`）で指して読むだけで使った
- 機能なしの記録（coverage-2026-10-31.md）との差: lm 94.9 → 86.8（`lm/src/llama.rs`（287 行、66.9%）が測定に入るため。llama.cpp の経路が測れるようになった結果で、悪化ではない）、engine-ffi 83.1 → 96.1、stack 88.1 → 89.3。変換のロジックの小計は 97.1 → 96.8（lm の行数が増えた分）
- 失敗した実データのテスト 4 件は、測定は続けた。原因（確認済みのもの）: `segment_real` と `engine` の性能のテストは p99 が 2ms の上限を超えた（計装つきで、別のレーンが CPU を使っている最中に流したため。負荷平均は約 11）。`lm/tests/real_model.rs` の `warm_up_makes_the_first_rank_meet_the_deadline` は「LM が間に合わなかった」で落ちた（同じく負荷の疑い。未確認）。`left_context_real` は候補の内容（板/版、器/機）の不一致で、負荷ではない。メインの `data/build/dict` の辞書がこのコミットのポリシーと一致していない可能性があるが、未確認。いずれも `scripts/check.sh --real` での合否の確認は別途要る
- 失敗したテストも、実行した分はカバレッジに入る
