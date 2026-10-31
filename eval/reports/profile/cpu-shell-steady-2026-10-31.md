# CPU プロファイル（samply）

- 対象: 殻のキー 1 回（shell_perf --metric p1）
- サンプル数: 4957（1000 Hz。全スレッドの合計）
- 数える範囲: `shell_perf::steady_dispatch` を含むサンプルだけ（それより外側は数えない）
- 測定日時（UTC）: 2026-10-31T08:01:19Z
- コミット: 0ae658a2c66209848814da12db2c0db0b60bdf42 （perf/measure-steady-state。作業ツリーの変更: あり）
- 負荷平均（1 分。測定の前）: 2.84
- 記録: `target/perf-tools/profile.json`（`samply load` でブラウザに開ける）

## 自分の時間（いちばん内側の関数）の上位

| 割合 | サンプル | 関数 |
|---:|---:|---|
| 31.6% | 1566 | `libsystem_malloc.dylib!（記号なし）` |
| 9.5% | 472 | `libsystem_kernel.dylib!（記号なし）` |
| 9.2% | 455 | `<unicode_segmentation::grapheme::Graphemes as core::iter::traits::iterator::Iterator>::next` |
| 9.0% | 446 | `shell_perf-77dbd1fded01a9a9!（記号なし）` |
| 7.7% | 383 | `libsystem_platform.dylib!（記号なし）` |
| 3.3% | 163 | `core::str::validations::run_utf8_validation` |
| 1.8% | 90 | `<alloc::string::String as core::clone::Clone>::clone` |
| 1.4% | 70 | `<ikari_shell_core::scenario::ScenarioSurrounding>::to_host` |
| 1.3% | 63 | `alloc::alloc::alloc` |
| 1.3% | 62 | `__rustc::__rdl_alloc` |
| 1.3% | 62 | `libsystem_pthread.dylib!（記号なし）` |
| 0.9% | 44 | `<unicode_segmentation::grapheme::GraphemeCursor>::grapheme_category` |
| 0.8% | 39 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing` |
| 0.8% | 38 | `<alloc::vec::Vec<u8> as prost::encoding::sealed::BytesAdapter>::replace_with::<bytes::buf::take::Take<&mut &mut &[u8]>>` |
| 0.7% | 36 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, ikari_input::romaji::Rule, alloc::co…` |
| 0.7% | 35 | `<ikari_shell_core::controller::Shell>::finish` |
| 0.7% | 33 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 0.6% | 32 | `<alloc::vec::Vec<ikari_shell_core::types::CandidateItem> as core::clone::Clone>::clone` |
| 0.6% | 30 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec` |
| 0.6% | 29 | `<ikari_shell_core::keymap::KeyChord>::for_lookup` |
| 0.5% | 25 | `<hashbrown::map::HashMap<&str, u32, std::hash::random::RandomState>>::rustc_entry` |
| 0.4% | 22 | `<ikari_shell_core::machine::InputStateMachine>::dispatch` |
| 0.4% | 18 | `<alloc::vec::Vec<ikari_shell_core::machine::state::ConvertingSegment>>::as_slice` |
| 0.4% | 18 | `<ikari_shell_core::machine::InputStateMachine>::handle` |
| 0.3% | 17 | `<alloc::collections::btree::map::BTreeMap<alloc::string::String, i64>>::entry` |

## 呼び出しの途中も含めた時間の上位

| 割合 | サンプル | 関数 |
|---:|---:|---|
| 100.0% | 4957 | `shell_perf::steady_dispatch` |
| 100.0% | 4956 | `<ikari_shell_core::controller::Shell>::dispatch` |
| 89.0% | 4414 | `<ikari_shell_core::machine::InputStateMachine>::handle` |
| 87.6% | 4341 | `<ikari_shell_core::machine::InputStateMachine>::dispatch` |
| 32.7% | 1620 | `libsystem_malloc.dylib!（記号なし）` |
| 25.8% | 1281 | `shell_perf-77dbd1fded01a9a9!（記号なし）` |
| 24.8% | 1228 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing` |
| 23.2% | 1149 | `<core::str::iter::Chars as core::iter::traits::iterator::Iterator>::count` |
| 23.2% | 1149 | `core::str::count::count_chars` |
| 23.2% | 1149 | `ikari_shell_core::engine::codec::chars` |
| 23.2% | 1149 | `ikari_shell_core::engine::codec::is_valid_reading` |
| 23.1% | 1143 | `<ikari_shell_core::engine::EngineLink>::post` |
| 17.3% | 859 | `<ikari_shell_core::machine::InputStateMachine>::start_conversion` |
| 16.7% | 828 | `<shell_perf::ScriptedTransport as ikari_shell_core::host::EngineTransport>::post` |
| 15.1% | 747 | `<ikari_shell_core::engine::EngineLink>::prefetch` |
| 13.7% | 680 | `<ikari_shell_core::machine::InputStateMachine>::emit_episode` |
| 13.7% | 680 | `<ikari_shell_core::machine::InputStateMachine>::now` |
| 13.7% | 680 | `<ikari_shell_core::machine::InputStateMachine>::reset_composition` |
| 12.8% | 634 | `<alloc::string::String as core::clone::Clone>::clone` |
| 12.7% | 629 | `<alloc::alloc::Global as core::alloc::Allocator>::allocate` |
| 12.7% | 629 | `<alloc::alloc::Global>::alloc_impl` |
| 12.7% | 629 | `<alloc::alloc::Global>::alloc_impl_runtime` |
| 12.7% | 629 | `alloc::alloc::alloc` |
| 12.5% | 622 | `<ikari_proto::v1::Envelope as prost::message::Message>::decode::<&[u8]>` |
| 12.5% | 622 | `<ikari_proto::v1::Envelope as prost::message::Message>::merge::<&mut &[u8]>` |

