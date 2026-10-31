# 定常（打鍵ごと）の費用と、起動の費用（2026-10-31。issue #161）

main 9f43fed の上に測り手の分割だけを足したコミットで測定。負荷平均は確保 3.0、CPU 2.8、P1 2.7（静かな状態の目安 3 未満。確保は少し超えた）。記録: `alloc-shell-steady-2026-10-31.md`、`cpu-shell-steady-2026-10-31.md`。

## 分かったこと

**P1（キー 1 回。3 回の範囲）**: 起動 p50 8.0〜8.3 us、p99 14〜16 us / 最初の 1 打 p50 750 ns、p99 9.4〜9.8 us / 定常 p50 875 ns、p99 7.2〜8.0 us。従来の全体の指標（p50 833 ns、p99 7.3〜8.2 us）は変わらない。

**確保の回数（1 回の dispatch）**:

| | p50 | p99 | max |
|---|---:|---:|---:|
| 起動（35800 回） | 63 | 70 | 110 |
| 最初の 1 打（35000 回） | 19 | 117 | 144 |
| 定常（215200 回） | 14 | 147 | 190 |

**定常の場所の上位（回/dispatch。`steady_dispatch` の中）**: `CandidateItem::clone` 3.56（+0.53 の 1 件）、`KeyKind::clone` 1.35、`CandidateItem` の `to_vec` 0.97（66 MB。バイト数が最大）、`DiagEpisode::event` 0.70 + 0.49、`refresh_composing` 0.64 + 0.63、`RomajiComposer::push` 0.63、`ConvertingState` まわり（`ConversionPage` の `to_vec` 0.36 / 14 MB、`ConvertingSegment` の clone と `to_vec` 0.36 × 3）。

**起動にだけ出るもの（定常の表にない）**: `menu::build`／`menu::radio`／`MenuItem::clone`、`Background::spawn`、`SettingsSnapshot::clone`（PR #168 のあとでも `make_client` に 1 回残る）、`begin_session`、`make_client`。以前の表で上位だった `menu::*`、`Background::spawn`、`MenuItem::clone` は、定常の費用ではなかった。`menu::radio` と `menu::build` は最初の 1 打にも出る（取り直しの結果でメニューが作り直される）。

**CPU（定常。samply、自分の時間）**: `malloc` 31.6%（記号なし）、`Graphemes::next` 9.2%、`run_utf8_validation` 3.3%。呼び出しを含む時間では `refresh_composing` 24.8%（うち `is_valid_reading` → `chars` の数え上げ 23.2%）、`EngineLink::post` 23.1%（うち替え玉の `ScriptedTransport::post` 16.7%）、`start_conversion` 17.3%。`Background::spawn` は定常の範囲では出ない（以前の 40.6% は起動の費用だった）。

## PR B の対象は、定常の打鍵ごとの費用として上位か

| 対象 | 定常で上位か | 呼び出し元（dhat の呼び出しの列） |
|---|---|---|
| `CandidateItem::clone`（3.56 + 0.53）と `to_vec`（0.97。66 MB） | 上位（回数 1 位、バイト数 1 位） | `handle_converting` mod.rs:864（最大）、`enter_converting` 1539、`sync_focused` state.rs:260、`select` 1103、`panel_model` 2122、`next_candidate` 981、`perform_segment_action` 1230 |
| `ConvertingState` の clone（`ConversionPage`／`ConvertingSegment` の `to_vec`、state.rs:162、164、220） | 中（各 0.36） | 上の `CandidateItem` の複製と重なる（同じ clone の枝） |
| `KeyKind::clone`（1.35） | 上位だが小さい（1 回 1 バイトほど。355 KB/測定） | `controller::japanese_key`（controller/mod.rs:458）と `machine::handle`（mod.rs:406）。候補ではなく**キーの変換**。PR B の対象ではなく、直す順番の 3（キー変換の `input.clone()`）に当たる |
| `menu::*`、`MenuItem::clone`、`Background::spawn`、`SettingsSnapshot::clone` | 定常では出ない | 起動の費用。本番の打鍵ごとの費用ではない |

PR B は `CandidateItem` の clone と `to_vec`、`ConvertingState` の clone に絞るのが妥当。`KeyKind` は別の PR（キー変換）。

## 推測と注意

- シナリオの手順は、変換中（候補の操作）に偏っている。本番の打鍵の平均より `CandidateItem` の割合が大きく出る可能性が高い（推測。本番の打鍵の内訳は未測定）。入力中（変換前）の打鍵だけなら、`refresh_composing` と `is_valid_reading` の方が上位になるはず。
- 定常の表には、替え玉のエンジン（`ScriptedTransport`）の確保と時間が入る（`EngineLink::post` のうち 16.7% は替え玉）。本番ではエンジンは別のプロセスで、この分は殻の費用ではない。
- 効きの見込み（回数の減る量）はここでは推測。直して再測定する。
