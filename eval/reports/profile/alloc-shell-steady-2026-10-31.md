# ヒープ確保（dhat。定常と起動に分けた）

- 対象: 殻のキー（shell_perf --metric alloc。repeat 200）。測り手は dispatch ごとに新しい `Shell` を作るので、起動の費用（`Activate`）を分けて数える
- 測定日時（UTC）: 2026-10-31T07:55:32Z
- コミット: 0ae658a2c66209848814da12db2c0db0b60bdf42 （perf/measure-steady-state。作業ツリーの変更: なし）
- 負荷平均（1 分。測定の前）: 3.02
- 記録: `target/perf-tools/dhat-heap.json`（dhat/dh_view.html で開ける）

## 定常（打鍵ごと。最初の 1 打のあと）: `dispatch` 215200 回

| | p50 | p99 | max |
|---|---:|---:|---:|
| 確保の回数 | 14 | 147 | 190 |
| 確保のバイト数 | 378 | 9243 | 76768 |

確保の場所の上位（`shell_perf::steady_dispatch` の中。ikari の関数と行にまとめた。同期のエンジン要求を送った dispatch の確保も入る）

| 回数 | 1 回の dispatch あたり | バイト数 | 場所 |
|---:|---:|---:|---|
| 765800 | 3.56 | 7464200 | `<ikari_shell_core::types::CandidateItem as core::clone::Clone>::clone (shell-core/src/types.rs:218)` |
| 290800 | 1.35 | 290800 | `<ikari_shell_core::types::KeyKind as core::clone::Clone>::clone (shell-core/src/types.rs:77)` |
| 207800 | 0.97 | 66880000 | `<ikari_shell_core::types::CandidateItem as <[_]>::to_vec_in::ConvertVec>::to_vec::<alloc::alloc::Global> (alloc/src/slice.rs:425)` |
| 149600 | 0.70 | 3560800 | `<ikari_shell_core::diag::DiagEpisode>::event::{closure#0} (src/diag/mod.rs:336)` |
| 137200 | 0.64 | 10976000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:772)` |
| 136000 | 0.63 | 1158000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:755)` |
| 135000 | 0.63 | 540000 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:131)` |
| 117600 | 0.55 | 12249000 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec (prost-0.14.4/src/message.rs:65)` |
| 113200 | 0.53 | 1924400 | `<ikari_shell_core::types::CandidateItem as core::clone::Clone>::clone (shell-core/src/types.rs:221)` |
| 104800 | 0.49 | 991200 | `<ikari_shell_core::diag::DiagEpisode>::event::{closure#1} (src/diag/mod.rs:375)` |
| 104400 | 0.49 | 6536400 | `<ikari_shell_core::machine::ContextText as core::clone::Clone>::clone (src/machine/mod.rs:68)` |
| 104400 | 0.49 | 6536400 | `<ikari_shell_core::machine::ContextText>::to_proto (src/machine/mod.rs:101)` |
| 101600 | 0.47 | 445400 | `<ikari_shell_core::keymap::KeyChord>::new::<&str> (shell-core/src/keymap.rs:204)` |
| 88200 | 0.41 | 793000 | `<ikari_shell_core::machine::InputStateMachine>::prefetch_if_changed (src/machine/mod.rs:2050)` |
| 88200 | 0.41 | 793000 | `<ikari_shell_core::machine::InputStateMachine>::prefetch_if_changed (src/machine/mod.rs:2053)` |
| 78200 | 0.36 | 14140800 | `<ikari_shell_core::machine::state::ConversionPage as <[_]>::to_vec_in::ConvertVec>::to_vec::<alloc::alloc::Global> (alloc/src/slice.rs:425)` |
| 76400 | 0.36 | 9168000 | `<ikari_shell_core::machine::state::ConvertingSegment as <[_]>::to_vec_in::ConvertVec>::to_vec::<alloc::alloc::Global> (alloc/src/slice.rs:425)` |
| 76400 | 0.36 | 1013600 | `<ikari_shell_core::machine::state::ConvertingSegment as core::clone::Clone>::clone (src/machine/state.rs:162)` |
| 76400 | 0.36 | 837800 | `<ikari_shell_core::machine::state::ConvertingSegment as core::clone::Clone>::clone (src/machine/state.rs:164)` |
| 76400 | 0.36 | 1013600 | `<ikari_shell_core::machine::state::ConvertingState as core::clone::Clone>::clone (src/machine/state.rs:220)` |
| 74800 | 0.35 | 712400 | `ikari_shell_core::machine::state::candidate_item (src/machine/state.rs:298)` |
| 63400 | 0.29 | 63400 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:756)` |
| 62800 | 0.29 | 5244000 | `<ikari_shell_core::scenario::ScenarioSurrounding>::to_host (shell-core/src/scenario.rs:154)` |
| 62400 | 0.29 | 2241600 | `ikari_shell_core::machine::suffix_graphemes (src/machine/mod.rs:2391)` |
| 61200 | 0.28 | 489600 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:135)` |

## 起動直後の最初の 1 打（`Activate` 以外の最初のイベント）: `dispatch` 35000 回

| | p50 | p99 | max |
|---|---:|---:|---:|
| 確保の回数 | 19 | 117 | 144 |
| 確保のバイト数 | 1157 | 6563 | 29012 |

確保の場所の上位（`shell_perf::first_dispatch` の中。ikari の関数と行にまとめた。同期のエンジン要求を送った dispatch の確保も入る）

| 回数 | 1 回の dispatch あたり | バイト数 | 場所 |
|---:|---:|---:|---|
| 146140 | 4.18 | 3887324 | `ikari_shell_core::menu::radio (shell-core/src/menu.rs:115)` |
| 64600 | 1.85 | 64600 | `<ikari_shell_core::types::KeyKind as core::clone::Clone>::clone (shell-core/src/types.rs:77)` |
| 33042 | 0.94 | 1321680 | `<ikari_shell_core::controller::Shell>::accept (src/controller/mod.rs:907)` |
| 30800 | 0.88 | 2473800 | `<ikari_shell_core::scenario::ScenarioSurrounding>::to_host (shell-core/src/scenario.rs:154)` |
| 30600 | 0.87 | 972600 | `ikari_shell_core::machine::suffix_graphemes (src/machine/mod.rs:2391)` |
| 29228 | 0.84 | 20576512 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:126)` |
| 29228 | 0.84 | 876840 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:159)` |
| 29228 | 0.84 | 876840 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:194)` |
| 29228 | 0.84 | 438420 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:202)` |
| 26400 | 0.75 | 2112000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:772)` |
| 26000 | 0.74 | 104000 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:131)` |
| 26000 | 0.74 | 208000 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:135)` |
| 23400 | 0.67 | 23400 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:756)` |
| 17600 | 0.50 | 371200 | `ikari_shell_core::settings::defaults::strings::{closure#0} (src/settings/defaults.rs:105)` |
| 7600 | 0.22 | 294800 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec (prost-0.14.4/src/message.rs:65)` |
| 7000 | 0.20 | 79800 | `<ikari_shell_core::machine::ContextText as core::clone::Clone>::clone (src/machine/mod.rs:68)` |
| 7000 | 0.20 | 79800 | `<ikari_shell_core::machine::ContextText>::to_proto (src/machine/mod.rs:101)` |
| 6400 | 0.18 | 254400 | `<ikari_shell_core::types::SettingsSnapshot as core::clone::Clone>::clone (shell-core/src/types.rs:542)` |
| 6000 | 0.17 | 9200 | `<ikari_shell_core::keymap::KeyChord>::new::<&str> (shell-core/src/keymap.rs:204)` |
| 5000 | 0.14 | 40000 | `<ikari_shell_core::english::EnglishMode>::chord (src/english/mod.rs:242)` |
| 4800 | 0.14 | 4800 | `<ikari_shell_core::keymap::KeyChord>::for_lookup (shell-core/src/keymap.rs:167)` |
| 4600 | 0.13 | 102800 | `<ikari_shell_core::types::SelectedText as core::clone::Clone>::clone (shell-core/src/types.rs:155)` |
| 4000 | 0.11 | 61200 | `<ikari_proto::v1::Reconvert as core::clone::Clone>::clone (ikari-proto-8b1c353ff574f9ea/out/ikari.engine.v1.rs:364)` |
| 4000 | 0.11 | 36000 | `<ikari_proto::v1::SurroundingText as core::clone::Clone>::clone (ikari-proto-8b1c353ff574f9ea/out/ikari.engine.v1.rs:165)` |
| 4000 | 0.11 | 61200 | `<ikari_shell_core::machine::InputStateMachine>::start_reconversion (src/machine/mod.rs:1655)` |

## 起動（`Activate`。`Shell::new` の直後と、入力先の切り替え）: `dispatch` 35800 回

| | p50 | p99 | max |
|---|---:|---:|---:|
| 確保の回数 | 63 | 70 | 110 |
| 確保のバイト数 | 4788 | 5075 | 6440 |

確保の場所の上位（`shell_perf::startup_dispatch` の中。ikari の関数と行にまとめた。同期のエンジン要求を送った dispatch の確保も入る）

| 回数 | 1 回の dispatch あたり | バイト数 | 場所 |
|---:|---:|---:|---|
| 283200 | 7.91 | 7363200 | `<ikari_shell_core::types::MenuItem as core::clone::Clone>::clone (shell-core/src/types.rs:333)` |
| 213486 | 5.96 | 8539440 | `<ikari_shell_core::controller::background::Background>::spawn (src/controller/background.rs:133)` |
| 178160 | 4.98 | 4739056 | `ikari_shell_core::menu::radio (shell-core/src/menu.rs:115)` |
| 142324 | 3.98 | 3202290 | `<ikari_shell_core::controller::background::Background>::spawn (src/controller/background.rs:132)` |
| 71162 | 1.99 | 355810 | `<ikari_shell_core::controller::background::Background>::spawn (src/controller/background.rs:118)` |
| 71162 | 1.99 | 10816624 | `<ikari_shell_core::controller::background::Background>::spawn (src/controller/background.rs:129)` |
| 35800 | 1.00 | 644000 | `<ikari_proto::v1::BeginSession as core::clone::Clone>::clone (ikari-proto-8b1c353ff574f9ea/out/ikari.engine.v1.rs:146)` |
| 35800 | 1.00 | 1073200 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec (prost-0.14.4/src/message.rs:65)` |
| 35800 | 1.00 | 644000 | `<ikari_shell_core::machine::SessionInfo>::to_proto (src/machine/mod.rs:127)` |
| 35632 | 1.00 | 25084928 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:126)` |
| 35632 | 1.00 | 1068960 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:159)` |
| 35632 | 1.00 | 1068960 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:194)` |
| 35632 | 1.00 | 534480 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:202)` |
| 35600 | 0.99 | 640400 | `<ikari_shell_core::controller::Shell>::make_client (src/controller/mod.rs:392)` |
| 35600 | 0.99 | 640400 | `<ikari_shell_core::controller::Shell>::make_client (src/controller/mod.rs:408)` |
| 35600 | 0.99 | 1424000 | `<ikari_shell_core::machine::InputStateMachine>::new (src/machine/mod.rs:223)` |
| 35400 | 0.99 | 11328000 | `<ikari_shell_core::controller::Shell>::finish (src/controller/mod.rs:882)` |
| 35400 | 0.99 | 19257600 | `<ikari_shell_core::engine::EngineLink>::begin_session (src/engine/mod.rs:189)` |
| 35400 | 0.99 | 24921600 | `<ikari_shell_core::types::MenuItem as <[_]>::to_vec_in::ConvertVec>::to_vec::<alloc::alloc::Global> (alloc/src/slice.rs:425)` |
| 35400 | 0.99 | 3823200 | `<shell_perf::EngineLog>::record (shell-core/benches/shell_perf.rs:636)` |
| 35400 | 0.99 | 2265600 | `ikari_shell_core::engine::lock::<()> (src/engine/mod.rs:66)` |
| 35400 | 0.99 | 2265600 | `ikari_shell_core::engine::lock::<ikari_shell_core::engine::LinkState> (src/engine/mod.rs:66)` |
| 34454 | 0.96 | 2205056 | `ikari_shell_core::controller::background::lock (src/controller/background.rs:86)` |
| 1891 | 0.05 | 75640 | `<ikari_shell_core::controller::Shell>::accept (src/controller/mod.rs:907)` |

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

