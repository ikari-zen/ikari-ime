# 候補と変換の状態の複製を減らした前後（issue #161、直す順番の 1 の後半。PR B）

- 前: `alloc-shell-steady-2026-10-31.md`、`cpu-shell-steady-2026-10-31.md`（PR #169 のコミット。コード側は main c15a0e9 と同じ）
- 後: `alloc-shell-after-candidate-clones-2026-10-31.md`、`cpu-shell-after-candidate-clones-2026-10-31.md`（`perf/shell-core-candidate-clones`）
- 負荷平均（測定の前）: 前 確保 3.02、CPU 2.84 / 後 確保 2.52、CPU 2.85、P1 約 3.0（静かな状態の目安 3 に近い。時間は参考）
- **効果は変換中の操作に限る見込み**。シナリオは変換中の操作に偏っていて、本番の打鍵より `CandidateItem` の割合が大きく出る（入力中の `refresh_composing`、`is_valid_reading` は対象外）

## 直したこと（公開の型は変えていない）
1. `handle_converting` が打鍵ごとに `ConvertingState` 全体を複製していたのをやめた（Esc と Backspace の戻す処理だけが複製する）。
2. `next_candidate` の複製をやめた。値を先に取り出し、変える段で `self.state` を直接書き換える。
3. `select`、`show_page`、`reveal_candidates_if_hidden` が、`panel_model`／`marked_text` を呼ぶためだけに状態を複製していた。両方を `self` を借りない関数にして、借用のまま呼ぶ。
4. `enter_converting` と字種変換で、`candidates` を作ってすぐ `sync_focused` が同じものをもう一度複製していた。前者を消した。
5. `sync_focused` を `clone_from` にし、`CandidateItem` に手書きの `clone_from`（文字列の領域を使い回す）を足した。`clone` の結果は同じ。

## 確保の回数（1 回の dispatch。定常 215200 回）
| | 前 p50 | 後 p50 | 前 p99 | 後 p99 | 前 max | 後 max |
|---|---:|---:|---:|---:|---:|---:|
| 確保の回数 | 14 | 13 | 147 | 126 | 190 | 180 |
| 確保のバイト数 | 378 | 378 | 9243 | 7816 | 76768 | 77123 |

| 場所（回/dispatch） | 前 | 後 |
|---|---:|---:|
| `CandidateItem::clone`（types.rs の derive） | 3.56 + 0.53 | 1.18 + 0.18 |
| `CandidateItem` の `to_vec`（バイト数） | 0.97（66.9 MB） | 0.18（11.5 MB） |
| `ConversionPage`／`ConvertingSegment` の `to_vec` と clone、`ConvertingState::clone` | 各 0.36（計 約 1.8） | 上位 25 から消えた |
| `KeyKind::clone`（対象外） | 1.35 | 1.35 |

## 時間
| | 前 | 後 |
|---|---:|---:|
| P1 定常 p50（3 回） | 875 ns | 750 ns |
| P1 定常 p99（3 回） | 7.2〜8.0 us | 7.5 us |
| P1 全体 p50 | 833 ns | 750 ns |
| CPU のサンプル数（定常。同じ負荷） | 4957 | 4311（-13%） |
| CPU で malloc の自分の時間 | 31.6% | 27.0% |

## 残り・やらなかったこと
- `CandidateItem::clone` の 1.18 回は、`panel_model`（候補ウィンドウの動作が候補の所有を求める。公開の `CandidatePanelModel`）と、`sync_focused` の平らな写し（公開のフィールド `ConvertingState.candidates`）による。減らすには公開の型の変更（`Arc` など）が要る。止めて、判断を仰ぐ。
- `perform_segment_action`、`enter_segment_mode`、`move_focus` など文節の操作（mod.rs の 1230 以降）の `ConvertingState` の複製は残した。文節の操作は頻度が低く、シナリオでも上位に出ないため。
- `KeyKind::clone` はキー変換の別の PR（#161 の順番 3）。
