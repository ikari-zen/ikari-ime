# CPU プロファイル（samply）

- 対象: 殻のキー 1 回（shell_perf --metric p1）
- サンプル数: 10283（1000 Hz。全スレッドの合計）
- 数える範囲: `ikari_shell_core::controller::Shell>::dispatch` を含むサンプルだけ（それより外側は数えない）
- 測定日時（UTC）: 2026-10-31T04:41:43Z
- コミット: 5020a52c9e8b115cf82fe9f4608e1f5a41f869fa （main。作業ツリーの変更: あり）
- 負荷平均（1 分。測定の前）: 2.82
- 記録: `target/perf-tools/profile.json`（`samply load` でブラウザに開ける）

## 自分の時間（いちばん内側の関数）の上位

| 割合 | サンプル | 関数 |
|---:|---:|---|
| 32.9% | 3388 | `libsystem_kernel.dylib!（記号なし）` |
| 19.5% | 2007 | `libsystem_malloc.dylib!（記号なし）` |
| 11.2% | 1151 | `libsystem_pthread.dylib!（記号なし）` |
| 6.4% | 657 | `<unicode_segmentation::grapheme::Graphemes as core::iter::traits::iterator::Iterator>::next` |
| 5.8% | 592 | `shell_perf-77dbd1fded01a9a9!（記号なし）` |
| 5.0% | 517 | `libsystem_platform.dylib!（記号なし）` |
| 1.5% | 153 | `core::str::validations::run_utf8_validation` |
| 1.3% | 134 | `<alloc::string::String as core::clone::Clone>::clone` |
| 0.8% | 78 | `<ikari_shell_core::scenario::ScenarioSurrounding>::to_host::{closure#0}` |
| 0.7% | 70 | `__rustc::__rdl_alloc` |
| 0.7% | 68 | `<unicode_segmentation::grapheme::GraphemeCursor>::grapheme_category` |
| 0.5% | 54 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, ikari_input::romaji::Rule, alloc::co…` |
| 0.5% | 51 | `alloc::alloc::alloc` |
| 0.5% | 50 | `<ikari_shell_core::controller::Shell>::finish` |
| 0.5% | 47 | `<ikari_shell_core::keymap::KeyChord>::for_lookup` |
| 0.4% | 45 | `<ikari_shell_core::machine::composer::Composer>::kana` |
| 0.4% | 36 | `<alloc::vec::Vec<u8>>::clear` |
| 0.3% | 28 | `<alloc::vec::Vec<ikari_shell_core::types::CandidateItem> as core::clone::Clone>::clone` |
| 0.3% | 28 | `<ikari_shell_core::controller::Shell>::dispatch` |
| 0.3% | 26 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec` |
| 0.2% | 24 | `ikari_shell_core::menu::build` |
| 0.2% | 23 | `<hashbrown::map::HashMap<&str, u32, std::hash::random::RandomState>>::rustc_entry` |
| 0.2% | 22 | `<ikari_shell_core::engine::EngineLink>::prepare` |
| 0.2% | 22 | `<ikari_shell_core::machine::InputStateMachine>::dispatch` |
| 0.2% | 22 | `<ikari_shell_core::types::MenuItem as core::slice::cmp::SlicePartialEq<ikari_shell_core::types::MenuItem>>::equal_same_length` |

## 呼び出しの途中も含めた時間の上位

| 割合 | サンプル | 関数 |
|---:|---:|---|
| 100.0% | 10283 | `<ikari_shell_core::controller::Shell>::dispatch` |
| 46.5% | 4779 | `<ikari_shell_core::machine::InputStateMachine>::handle` |
| 45.8% | 4705 | `<ikari_shell_core::machine::InputStateMachine>::dispatch` |
| 41.6% | 4282 | `libsystem_pthread.dylib!（記号なし）` |
| 40.6% | 4172 | `<ikari_shell_core::controller::background::Background>::spawn` |
| 39.8% | 4094 | `std::thread::lifecycle::spawn_unchecked::<<ikari_shell_core::controller::background::Background>::spawn::{closure#1}, ()>` |
| 38.2% | 3930 | `<std::sys::thread::unix::Thread>::new` |
| 32.9% | 3388 | `libsystem_kernel.dylib!（記号なし）` |
| 20.6% | 2120 | `libsystem_malloc.dylib!（記号なし）` |
| 15.7% | 1617 | `shell_perf-77dbd1fded01a9a9!（記号なし）` |
| 13.4% | 1377 | `<ikari_shell_core::engine::EngineLink>::post` |
| 11.3% | 1164 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing` |
| 11.3% | 1164 | `<ikari_shell_core::machine::composer::Composer>::kana` |
| 10.8% | 1112 | `ikari_shell_core::engine::codec::is_valid_reading` |
| 9.2% | 946 | `<shell_perf::ScriptedTransport as ikari_shell_core::host::EngineTransport>::post` |
| 8.9% | 917 | `ikari_shell_core::machine::grapheme_count` |
| 8.6% | 881 | `<alloc::string::String as core::clone::Clone>::clone` |
| 8.5% | 875 | `<ikari_shell_core::machine::InputStateMachine>::start_conversion` |
| 8.5% | 875 | `<str as unicode_segmentation::UnicodeSegmentation>::graphemes` |
| 8.5% | 875 | `unicode_segmentation::grapheme::new_graphemes` |
| 7.7% | 788 | `<ikari_shell_core::machine::InputStateMachine>::refresh_context` |
| 7.1% | 731 | `<ikari_shell_core::machine::InputStateMachine>::emit_episode` |
| 7.1% | 731 | `<ikari_shell_core::machine::InputStateMachine>::reset_composition` |
| 7.0% | 721 | `<unicode_segmentation::grapheme::Graphemes as core::iter::traits::iterator::Iterator>::next` |
| 6.7% | 694 | `<core::str::iter::Chars as core::iter::traits::iterator::Iterator>::count` |

