# ヒープ確保（dhat。定常と起動に分けた）

- 対象: 殻のキー（shell_perf --metric alloc。repeat 200）。測り手は dispatch ごとに新しい `Shell` を作るので、起動の費用（`Activate`）を分けて数える
- 測定日時（UTC）: 2026-10-31T07:46:52Z
- コミット: 250d08d14a005a442387c795447fa049851b543e （perf/measure-steady-state。作業ツリーの変更: あり）
- 負荷平均（1 分。測定の前）: 2.32
- 記録: `target/perf-tools/dhat-heap.json`（dhat/dh_view.html で開ける）

## 定常（打鍵ごと。`Activate` 以外）: `dispatch` 250200 回

| | p50 | p99 | max |
|---|---:|---:|---:|
| 確保の回数 | 15 | 144 | 180 |
| 確保のバイト数 | 380 | 9174 | 76064 |

確保の場所の上位（`shell_perf::steady_dispatch` の中。ikari の関数と行にまとめた。同期のエンジン要求を送った dispatch の確保も入る）

| 回数 | 1 回の dispatch あたり | バイト数 | 場所 |
|---:|---:|---:|---|
| 765800 | 3.06 | 7464200 | `<ikari_shell_core::types::CandidateItem as core::clone::Clone>::clone (shell-core/src/types.rs:218)` |
| 355400 | 1.42 | 355400 | `<ikari_shell_core::types::KeyKind as core::clone::Clone>::clone (shell-core/src/types.rs:77)` |
| 349220 | 1.40 | 9289252 | `ikari_shell_core::menu::radio (shell-core/src/menu.rs:115)` |
| 288000 | 1.15 | 7488000 | `<ikari_shell_core::types::MenuItem as core::clone::Clone>::clone (shell-core/src/types.rs:333)` |
| 214101 | 0.86 | 8564040 | `<ikari_shell_core::controller::background::Background>::spawn (src/controller/background.rs:133)` |
| 207800 | 0.83 | 66880000 | `<ikari_shell_core::types::CandidateItem as <[_]>::to_vec_in::ConvertVec>::to_vec::<alloc::alloc::Global> (alloc/src/slice.rs:425)` |
| 163600 | 0.65 | 13088000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:772)` |
| 161000 | 0.64 | 644000 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:131)` |
| 161000 | 0.64 | 13617000 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec (prost-0.14.4/src/message.rs:65)` |
| 149600 | 0.60 | 3560800 | `<ikari_shell_core::diag::DiagEpisode>::event::{closure#0} (src/diag/mod.rs:336)` |
| 142734 | 0.57 | 3211515 | `<ikari_shell_core::controller::background::Background>::spawn (src/controller/background.rs:132)` |
| 139000 | 0.56 | 1167000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:755)` |
| 113200 | 0.45 | 1924400 | `<ikari_shell_core::types::CandidateItem as core::clone::Clone>::clone (shell-core/src/types.rs:221)` |
| 111400 | 0.45 | 6616200 | `<ikari_shell_core::machine::ContextText as core::clone::Clone>::clone (src/machine/mod.rs:68)` |
| 111400 | 0.45 | 6616200 | `<ikari_shell_core::machine::ContextText>::to_proto (src/machine/mod.rs:101)` |
| 107600 | 0.43 | 454600 | `<ikari_shell_core::keymap::KeyChord>::new::<&str> (shell-core/src/keymap.rs:204)` |
| 104800 | 0.42 | 991200 | `<ikari_shell_core::diag::DiagEpisode>::event::{closure#1} (src/diag/mod.rs:375)` |
| 93600 | 0.37 | 7717800 | `<ikari_shell_core::scenario::ScenarioSurrounding>::to_host (shell-core/src/scenario.rs:154)` |
| 93000 | 0.37 | 3214200 | `ikari_shell_core::machine::suffix_graphemes (src/machine/mod.rs:2391)` |
| 91200 | 0.36 | 802000 | `<ikari_shell_core::machine::InputStateMachine>::prefetch_if_changed (src/machine/mod.rs:2050)` |
| 91200 | 0.36 | 802000 | `<ikari_shell_core::machine::InputStateMachine>::prefetch_if_changed (src/machine/mod.rs:2053)` |
| 87200 | 0.35 | 697600 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:135)` |
| 86800 | 0.35 | 86800 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:756)` |
| 78200 | 0.31 | 14140800 | `<ikari_shell_core::machine::state::ConversionPage as <[_]>::to_vec_in::ConvertVec>::to_vec::<alloc::alloc::Global> (alloc/src/slice.rs:425)` |
| 76400 | 0.31 | 9168000 | `<ikari_shell_core::machine::state::ConvertingSegment as <[_]>::to_vec_in::ConvertVec>::to_vec::<alloc::alloc::Global> (alloc/src/slice.rs:425)` |

## 起動（`Activate`。`Shell::new` の直後と、入力先の切り替え）: `dispatch` 35800 回

| | p50 | p99 | max |
|---|---:|---:|---:|
| 確保の回数 | 63 | 71 | 73 |
| 確保のバイト数 | 4788 | 5093 | 5843 |

確保の場所の上位（`shell_perf::startup_dispatch` の中。ikari の関数と行にまとめた。同期のエンジン要求を送った dispatch の確保も入る）

| 回数 | 1 回の dispatch あたり | バイト数 | 場所 |
|---:|---:|---:|---|

## キーごとの確保の回数（多い順。起動のイベントも含む）

```
# 確保の多いキー(確保の回数の中央値の降順の上位 25)  キー
     180  menu selects the keymap preset and applies it when idle #2.0 Key(KeyInput { key: Enter, modifiers: Modifiers { shift: false, control: false, option: false, com…
     169  numbers are one to three until expanded then one to nine #6.0 Key(KeyInput { key: Character { text: "5" }, modifiers: Modifiers { shift: false, control: false,…
     169  the setting sets how many are shown first #3.0 Key(KeyInput { key: Character { text: "5" }, modifiers: Modifiers { shift: false, control: false, option: false,…
     151  number key on a later page uses the cumulative index #5.0 Key(KeyInput { key: Character { text: "3" }, modifiers: Modifiers { shift: false, control: false, opt…
     151  typing a character while converting commits and starts a new composition #2.0 Key(KeyInput { key: Character { text: "k" }, modifiers: Modifiers { shift: false,…
     149  a digit shows the list instead of picking #3.0 Key(KeyInput { key: Character { text: "2" }, modifiers: Modifiers { shift: false, control: false, option: false,…
     149  failed page fetch keeps the current selection #5.0 Key(KeyInput { key: Enter, modifiers: Modifiers { shift: false, control: false, option: false, command: fals…
     149  number key selects and commits that candidate #2.0 Key(KeyInput { key: Character { text: "2" }, modifiers: Modifiers { shift: false, control: false, option: fa…
     147  JIS digit keys also select in kana mode #2.0 Key(KeyInput { key: Jis { code: 1 }, modifiers: Modifiers { shift: false, control: false, option: false, command: …
     147  command shortcut while converting commits first then passes through #2.0 Key(KeyInput { key: Character { text: "v" }, modifiers: Modifiers { shift: false, cont…
     147  typing a character commits all segments and starts a composition #4.0 Key(KeyInput { key: Character { text: "k" }, modifiers: Modifiers { shift: false, control…
     144  a number key commits the whole candidate even with two segments #2.0 Key(KeyInput { key: Character { text: "2" }, modifiers: Modifiers { shift: false, control:…
     144  a number key on two segments chooses and moves to the next segment #4.0 Key(KeyInput { key: Character { text: "2" }, modifiers: Modifiers { shift: false, contr…
     144  escape and backspace return to the whole reading #4.0 Key(KeyInput { key: Character { text: "2" }, modifiers: Modifiers { shift: false, control: false, option:…
     144  the IME commit composition commits what is shown #4.0 Key(KeyInput { key: Character { text: "2" }, modifiers: Modifiers { shift: false, control: false, option:…
     143  a command shortcut commits first then passes through #4.0 Key(KeyInput { key: Character { text: "c" }, modifiers: Modifiers { shift: false, control: false, opt…
     142  enter commits the top candidate while hidden #2.0 Key(KeyInput { key: Enter, modifiers: Modifiers { shift: false, control: false, option: false, command: false…
     142  enter commits the selected candidate #3.0 Key(KeyInput { key: Enter, modifiers: Modifiers { shift: false, control: false, option: false, command: false }, is_r…
     142  jis digit keys behave the same #4.0 Key(KeyInput { key: Jis { code: 1 }, modifiers: Modifiers { shift: false, control: false, option: false, command: false }, …
     140  an arrow with a command modifier while converting commits and passes through #4.0 Key(KeyInput { key: Left, modifiers: Modifiers { shift: false, control: false…
     138  enter commits the selected whole candidate with the legacy commit #3.0 Key(KeyInput { key: Enter, modifiers: Modifiers { shift: false, control: false, option: …
     138  a whole selection the segments can form is carried into the clause mode #4.0 Key(KeyInput { key: Enter, modifiers: Modifiers { shift: false, control: false, op…
     138  shift arrows resegment from the first segment #4.0 Key(KeyInput { key: Enter, modifiers: Modifiers { shift: false, control: false, option: false, command: fals…
     138  enter without visiting a segment sends its first candidate #4.0 Key(KeyInput { key: Enter, modifiers: Modifiers { shift: false, control: false, option: false, …
     138  choices across pages use the cumulative index #7.0 Key(KeyInput { key: Enter, modifiers: Modifiers { shift: false, control: false, option: false, command: fals…
```

