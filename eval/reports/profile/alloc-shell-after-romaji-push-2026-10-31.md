# ヒープ確保（dhat。定常と起動に分けた）

- 対象: 殻のキー（shell_perf --metric alloc。repeat 200）。測り手は dispatch ごとに新しい `Shell` を作るので、起動の費用（`Activate`）を分けて数える
- 測定日時（UTC）: 2026-10-31T09:36:09Z
- コミット: eaa4eed13735a2539059a5a6e8860954ab3bc5b0 （perf/input-romaji-push。作業ツリーの変更: なし）
- 負荷平均（1 分。測定の前）: 3.98
- 記録: `target/perf-tools/dhat-heap.json`（dhat/dh_view.html で開ける）

## 定常（打鍵ごと。最初の 1 打のあと）: `dispatch` 215200 回

| | p50 | p99 | max |
|---|---:|---:|---:|
| 確保の回数 | 11 | 126 | 180 |
| 確保のバイト数 | 352 | 7816 | 76764 |

確保の場所の上位（`shell_perf::steady_dispatch` の中。ikari の関数と行にまとめた。同期のエンジン要求を送った dispatch の確保も入る）

| 回数 | 1 回の dispatch あたり | バイト数 | 場所 |
|---:|---:|---:|---|
| 290800 | 1.35 | 290800 | `<ikari_shell_core::types::KeyKind as core::clone::Clone>::clone (shell-core/src/types.rs:77)` |
| 253400 | 1.18 | 2949000 | `<ikari_shell_core::types::CandidateItem as core::clone::Clone>::clone (shell-core/src/types.rs:232)` |
| 149600 | 0.70 | 3560800 | `<ikari_shell_core::diag::DiagEpisode>::event::{closure#0} (src/diag/mod.rs:336)` |
| 137200 | 0.64 | 1221400 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:775)` |
| 137200 | 0.64 | 10976000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:783)` |
| 117600 | 0.55 | 12249000 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec (prost-0.14.4/src/message.rs:65)` |
| 104800 | 0.49 | 991200 | `<ikari_shell_core::diag::DiagEpisode>::event::{closure#1} (src/diag/mod.rs:375)` |
| 104400 | 0.49 | 6536400 | `<ikari_shell_core::machine::ContextText as core::clone::Clone>::clone (src/machine/mod.rs:68)` |
| 104400 | 0.49 | 6536400 | `<ikari_shell_core::machine::ContextText>::to_proto (src/machine/mod.rs:101)` |
| 101600 | 0.47 | 445400 | `<ikari_shell_core::keymap::KeyChord>::new::<&str> (shell-core/src/keymap.rs:204)` |
| 88200 | 0.41 | 793000 | `<ikari_shell_core::machine::InputStateMachine>::prefetch_if_changed (src/machine/mod.rs:2089)` |
| 74800 | 0.35 | 712400 | `ikari_shell_core::machine::state::candidate_item (src/machine/state.rs:298)` |
| 63000 | 0.29 | 662400 | `<ikari_shell_core::machine::InputStateMachine>::prefetch_if_changed (src/machine/mod.rs:2085)` |
| 62800 | 0.29 | 5244000 | `<ikari_shell_core::scenario::ScenarioSurrounding>::to_host (shell-core/src/scenario.rs:154)` |
| 62400 | 0.29 | 2241600 | `ikari_shell_core::machine::suffix_graphemes (src/machine/mod.rs:2432)` |
| 48400 | 0.22 | 674800 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:771)` |
| 46200 | 0.21 | 652800 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:206)` |
| 39200 | 0.18 | 11520000 | `<ikari_shell_core::types::CandidateItem as <[_]>::to_vec_in::ConvertVec>::to_vec::<alloc::alloc::Global> (alloc/src/slice.rs:425)` |
| 38600 | 0.18 | 374400 | `<ikari_shell_core::types::MarkedText>::plain::<&str> (shell-core/src/types.rs:190)` |
| 38600 | 0.18 | 617600 | `ikari_shell_core::machine::marked_text (src/machine/mod.rs:2124)` |
| 38000 | 0.18 | 646000 | `<ikari_shell_core::types::CandidateItem as core::clone::Clone>::clone (shell-core/src/types.rs:233)` |
| 37200 | 0.17 | 5443200 | `<ikari_shell_core::machine::InputStateMachine>::enter_converting (src/machine/mod.rs:1572)` |
| 34570 | 0.16 | 919562 | `ikari_shell_core::menu::radio (shell-core/src/menu.rs:115)` |
| 34000 | 0.16 | 8992000 | `ikari_shell_core::machine::panel_model (src/machine/mod.rs:2166)` |
| 21800 | 0.10 | 7856000 | `<ikari_shell_core::machine::state::ConvertingState>::sync_focused (src/machine/state.rs:260)` |

## 起動直後の最初の 1 打（`Activate` 以外の最初のイベント）: `dispatch` 35000 回

| | p50 | p99 | max |
|---|---:|---:|---:|
| 確保の回数 | 19 | 118 | 146 |
| 確保のバイト数 | 1169 | 6566 | 29024 |

確保の場所の上位（`shell_perf::first_dispatch` の中。ikari の関数と行にまとめた。同期のエンジン要求を送った dispatch の確保も入る）

| 回数 | 1 回の dispatch あたり | バイト数 | 場所 |
|---:|---:|---:|---|
| 135800 | 3.88 | 3612280 | `ikari_shell_core::menu::radio (shell-core/src/menu.rs:115)` |
| 64600 | 1.85 | 64600 | `<ikari_shell_core::types::KeyKind as core::clone::Clone>::clone (shell-core/src/types.rs:77)` |
| 33004 | 0.94 | 1320160 | `<ikari_shell_core::controller::Shell>::accept (src/controller/mod.rs:907)` |
| 30800 | 0.88 | 2473800 | `<ikari_shell_core::scenario::ScenarioSurrounding>::to_host (shell-core/src/scenario.rs:154)` |
| 30600 | 0.87 | 972600 | `ikari_shell_core::machine::suffix_graphemes (src/machine/mod.rs:2432)` |
| 27160 | 0.78 | 19120640 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:126)` |
| 27160 | 0.78 | 814800 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:159)` |
| 27160 | 0.78 | 814800 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:194)` |
| 27160 | 0.78 | 407400 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:202)` |
| 26400 | 0.75 | 32400 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:775)` |
| 26400 | 0.75 | 2112000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:783)` |
| 26000 | 0.74 | 416000 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:197)` |
| 26000 | 0.74 | 208000 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:199)` |
| 23400 | 0.67 | 187200 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:773)` |
| 17600 | 0.50 | 371200 | `ikari_shell_core::settings::defaults::strings::{closure#0} (src/settings/defaults.rs:105)` |
| 7600 | 0.22 | 294800 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec (prost-0.14.4/src/message.rs:65)` |
| 7000 | 0.20 | 79800 | `<ikari_shell_core::machine::ContextText as core::clone::Clone>::clone (src/machine/mod.rs:68)` |
| 7000 | 0.20 | 79800 | `<ikari_shell_core::machine::ContextText>::to_proto (src/machine/mod.rs:101)` |
| 6400 | 0.18 | 254400 | `<ikari_shell_core::types::SettingsSnapshot as core::clone::Clone>::clone (shell-core/src/types.rs:561)` |
| 6000 | 0.17 | 9200 | `<ikari_shell_core::keymap::KeyChord>::new::<&str> (shell-core/src/keymap.rs:204)` |
| 5600 | 0.16 | 36200 | `<ikari_shell_core::machine::InputStateMachine>::prefetch_if_changed (src/machine/mod.rs:2085)` |
| 5000 | 0.14 | 40000 | `<ikari_shell_core::english::EnglishMode>::chord (src/english/mod.rs:242)` |
| 4800 | 0.14 | 4800 | `<ikari_shell_core::keymap::KeyChord>::for_lookup (shell-core/src/keymap.rs:167)` |
| 4600 | 0.13 | 102800 | `<ikari_shell_core::types::SelectedText as core::clone::Clone>::clone (shell-core/src/types.rs:155)` |
| 4000 | 0.11 | 61200 | `<ikari_proto::v1::Reconvert as core::clone::Clone>::clone (ikari-proto-8b1c353ff574f9ea/out/ikari.engine.v1.rs:364)` |

## 起動（`Activate`。`Shell::new` の直後と、入力先の切り替え）: `dispatch` 35800 回

| | p50 | p99 | max |
|---|---:|---:|---:|
| 確保の回数 | 63 | 70 | 103 |
| 確保のバイト数 | 4788 | 5069 | 5843 |

確保の場所の上位（`shell_perf::startup_dispatch` の中。ikari の関数と行にまとめた。同期のエンジン要求を送った dispatch の確保も入る）

| 回数 | 1 回の dispatch あたり | バイト数 | 場所 |
|---:|---:|---:|---|
| 283200 | 7.91 | 7363200 | `<ikari_shell_core::types::MenuItem as core::clone::Clone>::clone (shell-core/src/types.rs:352)` |
| 213486 | 5.96 | 8539440 | `<ikari_shell_core::controller::background::Background>::spawn (src/controller/background.rs:133)` |
| 178165 | 4.98 | 4739189 | `ikari_shell_core::menu::radio (shell-core/src/menu.rs:115)` |
| 142324 | 3.98 | 3202290 | `<ikari_shell_core::controller::background::Background>::spawn (src/controller/background.rs:132)` |
| 71162 | 1.99 | 355810 | `<ikari_shell_core::controller::background::Background>::spawn (src/controller/background.rs:118)` |
| 71162 | 1.99 | 10816624 | `<ikari_shell_core::controller::background::Background>::spawn (src/controller/background.rs:129)` |
| 35800 | 1.00 | 644000 | `<ikari_proto::v1::BeginSession as core::clone::Clone>::clone (ikari-proto-8b1c353ff574f9ea/out/ikari.engine.v1.rs:146)` |
| 35800 | 1.00 | 1073200 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec (prost-0.14.4/src/message.rs:65)` |
| 35800 | 1.00 | 644000 | `<ikari_shell_core::machine::SessionInfo>::to_proto (src/machine/mod.rs:127)` |
| 35633 | 1.00 | 25085632 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:126)` |
| 35633 | 1.00 | 1068990 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:159)` |
| 35633 | 1.00 | 1068990 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:194)` |
| 35633 | 1.00 | 534495 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:202)` |
| 35600 | 0.99 | 640400 | `<ikari_shell_core::controller::Shell>::make_client (src/controller/mod.rs:392)` |
| 35600 | 0.99 | 640400 | `<ikari_shell_core::controller::Shell>::make_client (src/controller/mod.rs:408)` |
| 35600 | 0.99 | 1424000 | `<ikari_shell_core::machine::InputStateMachine>::new (src/machine/mod.rs:223)` |
| 35400 | 0.99 | 11328000 | `<ikari_shell_core::controller::Shell>::finish (src/controller/mod.rs:882)` |
| 35400 | 0.99 | 19257600 | `<ikari_shell_core::engine::EngineLink>::begin_session (src/engine/mod.rs:189)` |
| 35400 | 0.99 | 24921600 | `<ikari_shell_core::types::MenuItem as <[_]>::to_vec_in::ConvertVec>::to_vec::<alloc::alloc::Global> (alloc/src/slice.rs:425)` |
| 35400 | 0.99 | 3823200 | `<shell_perf::EngineLog>::record (shell-core/benches/shell_perf.rs:777)` |
| 35400 | 0.99 | 2265600 | `ikari_shell_core::engine::lock::<()> (src/engine/mod.rs:66)` |
| 35400 | 0.99 | 2265600 | `ikari_shell_core::engine::lock::<ikari_shell_core::engine::LinkState> (src/engine/mod.rs:66)` |
| 34583 | 0.97 | 2213312 | `ikari_shell_core::controller::background::lock (src/controller/background.rs:86)` |
| 1984 | 0.06 | 79360 | `<ikari_shell_core::controller::Shell>::accept (src/controller/mod.rs:907)` |

## 入力中: ローマ字の途中（読みが変わらず、先読みを送らない）: `dispatch` 188000 回

| | p50 | p99 | max |
|---|---:|---:|---:|
| 確保の回数 | 4 | 19 | 32 |
| 確保のバイト数 | 101 | 1700 | 2081 |

確保の場所の上位（`shell_perf::typing_pending_dispatch` の中。ikari の関数と行にまとめた。同期のエンジン要求を送った dispatch の確保も入る）

| 回数 | 1 回の dispatch あたり | バイト数 | 場所 |
|---:|---:|---:|---|
| 376000 | 2.00 | 376000 | `<ikari_shell_core::types::KeyKind as core::clone::Clone>::clone (shell-core/src/types.rs:77)` |
| 188000 | 1.00 | 3292000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:775)` |
| 188000 | 1.00 | 15040000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:783)` |
| 58910 | 0.31 | 1567006 | `ikari_shell_core::menu::radio (shell-core/src/menu.rs:115)` |
| 24000 | 0.13 | 192000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:773)` |
| 20000 | 0.11 | 320000 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:197)` |
| 20000 | 0.11 | 160000 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:199)` |
| 19241 | 0.10 | 769640 | `<ikari_shell_core::controller::Shell>::accept (src/controller/mod.rs:907)` |
| 11782 | 0.06 | 8294528 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:126)` |
| 11782 | 0.06 | 353460 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:159)` |
| 11782 | 0.06 | 353460 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:194)` |
| 11782 | 0.06 | 176730 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:202)` |

## 入力中: 読み（かな）が変わって先読みを送る: `dispatch` 244000 回

| | p50 | p99 | max |
|---|---:|---:|---:|
| 確保の回数 | 7 | 25 | 42 |
| 確保のバイト数 | 204 | 1754 | 2223 |

確保の場所の上位（`shell_perf::typing_reading_dispatch` の中。ikari の関数と行にまとめた。同期のエンジン要求を送った dispatch の確保も入る）

| 回数 | 1 回の dispatch あたり | バイト数 | 場所 |
|---:|---:|---:|---|
| 488000 | 2.00 | 488000 | `<ikari_shell_core::types::KeyKind as core::clone::Clone>::clone (shell-core/src/types.rs:77)` |
| 244000 | 1.00 | 8744000 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec (prost-0.14.4/src/message.rs:65)` |
| 244000 | 1.00 | 5328000 | `<ikari_shell_core::machine::InputStateMachine>::prefetch_if_changed (src/machine/mod.rs:2089)` |
| 244000 | 1.00 | 5352000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:775)` |
| 244000 | 1.00 | 19520000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:783)` |
| 96000 | 0.39 | 1820000 | `<ikari_shell_core::machine::InputStateMachine>::prefetch_if_changed (src/machine/mod.rs:2085)` |
| 80000 | 0.33 | 1984000 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:206)` |
| 80000 | 0.33 | 1984000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:771)` |
| 61075 | 0.25 | 1624595 | `ikari_shell_core::menu::radio (shell-core/src/menu.rs:115)` |
| 12215 | 0.05 | 8599360 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:126)` |
| 12215 | 0.05 | 366450 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:159)` |
| 12215 | 0.05 | 366450 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:194)` |
| 12215 | 0.05 | 183225 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:202)` |
| 4000 | 0.02 | 64000 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:197)` |
| 4000 | 0.02 | 32000 | `<ikari_input::romaji::RomajiComposer>::push (input/src/romaji.rs:199)` |
| 3851 | 0.02 | 154040 | `<ikari_shell_core::controller::Shell>::accept (src/controller/mod.rs:907)` |

## 入力中: Backspace、Esc: `dispatch` 16000 回

| | p50 | p99 | max |
|---|---:|---:|---:|
| 確保の回数 | 6 | 72 | 72 |
| 確保のバイト数 | 129 | 3293 | 3293 |

確保の場所の上位（`shell_perf::typing_edit_dispatch` の中。ikari の関数と行にまとめた。同期のエンジン要求を送った dispatch の確保も入る）

| 回数 | 1 回の dispatch あたり | バイト数 | 場所 |
|---:|---:|---:|---|
| 48000 | 3.00 | 1784000 | `<ikari_shell_core::diag::DiagEpisode>::event::{closure#0} (src/diag/mod.rs:336)` |
| 36000 | 2.25 | 2344000 | `<ikari_shell_core::diag::DiagEpisode>::event::{closure#1} (src/diag/mod.rs:375)` |
| 16000 | 1.00 | 1088000 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec (prost-0.14.4/src/message.rs:65)` |
| 16000 | 1.00 | 132000 | `<ikari_shell_core::keymap::KeyChord>::new::<&str> (shell-core/src/keymap.rs:204)` |
| 12000 | 0.75 | 72000 | `<ikari_shell_core::machine::InputStateMachine>::prefetch_if_changed (src/machine/mod.rs:2089)` |
| 12000 | 0.75 | 72000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:775)` |
| 12000 | 0.75 | 960000 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing (src/machine/mod.rs:783)` |
| 4000 | 0.25 | 40000 | `<ikari_shell_core::diag::DiagEvent>::new (src/diag/mod.rs:35)` |
| 4000 | 0.25 | 84000 | `<ikari_shell_core::machine::InputStateMachine>::composer_display (src/machine/mod.rs:742)` |
| 4000 | 0.25 | 320000 | `<ikari_shell_core::machine::InputStateMachine>::handle_composing (src/machine/mod.rs:722)` |
| 4000 | 0.25 | 40000 | `ikari_shell_core::engine::codec::diag_message (src/engine/codec.rs:129)` |
| 4000 | 0.25 | 1408000 | `ikari_shell_core::engine::codec::diag_message (src/engine/codec.rs:137)` |
| 4000 | 0.25 | 768000 | `ikari_shell_core::engine::codec::diag_message (src/engine/codec.rs:145)` |
| 5 | 0.00 | 133 | `ikari_shell_core::menu::radio (shell-core/src/menu.rs:115)` |
| 1 | 0.00 | 704 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:126)` |
| 1 | 0.00 | 30 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:159)` |
| 1 | 0.00 | 30 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:194)` |
| 1 | 0.00 | 15 | `ikari_shell_core::menu::build (shell-core/src/menu.rs:202)` |

