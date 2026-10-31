# CPU プロファイル（samply）

- 対象: 殻のキー 1 回（shell_perf --metric p1）
- サンプル数: 3748（1000 Hz。全スレッドの合計）
- 数える範囲: `shell_perf::typing_` を含むサンプルだけ（それより外側は数えない）
- 測定日時（UTC）: 2026-10-31T09:41:55Z
- コミット: eaa4eed13735a2539059a5a6e8860954ab3bc5b0 （perf/input-romaji-push。作業ツリーの変更: あり）
- 負荷平均（1 分。測定の前）: 2.84
- 記録: `target/perf-tools/profile.json`（`samply load` でブラウザに開ける）

## 自分の時間（いちばん内側の関数）の上位

| 割合 | サンプル | 関数 |
|---:|---:|---|
| 24.0% | 899 | `libsystem_malloc.dylib!（記号なし）` |
| 21.5% | 807 | `libsystem_kernel.dylib!（記号なし）` |
| 9.1% | 340 | `shell_perf-77dbd1fded01a9a9!（記号なし）` |
| 7.7% | 289 | `libsystem_platform.dylib!（記号なし）` |
| 3.6% | 135 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing` |
| 3.6% | 135 | `libsystem_pthread.dylib!（記号なし）` |
| 2.9% | 109 | `<ikari_input::romaji::RomajiComposer>::push` |
| 2.1% | 78 | `<ikari_input::romaji::RomajiTable>::lookup` |
| 2.0% | 74 | `core::str::validations::run_utf8_validation` |
| 1.6% | 61 | `<ikari_shell_core::keymap::KeyChord>::for_lookup` |
| 1.3% | 49 | `<ikari_shell_core::controller::Shell>::finish` |
| 1.0% | 39 | `<ikari_shell_core::controller::Shell>::dispatch` |
| 1.0% | 36 | `<alloc::vec::Vec<u8>>::clear` |
| 0.9% | 32 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 0.9% | 32 | `<alloc::string::String as core::clone::Clone>::clone` |
| 0.8% | 29 | `<ikari_shell_core::machine::InputStateMachine>::handle` |
| 0.7% | 28 | `__rustc::__rdl_alloc` |
| 0.7% | 27 | `prost::encoding::merge_loop::<ikari_proto::v1::Prefetch, prost::encoding::message::merge<ikari_proto::v1::Prefetch, &mut &[u8]>::{closure#0}, &mut &[…` |
| 0.7% | 25 | `<ikari_shell_core::machine::InputStateMachine>::dispatch` |
| 0.6% | 24 | `<ikari_proto::v1::Prefetch as prost::message::Message>::merge_field::<&mut &[u8]>` |
| 0.6% | 21 | `<core::slice::iter::Iter<u8> as core::iter::traits::iterator::Iterator>::fold::<usize, core::iter::adapters::map::map_fold<&u8, usize, usize, <core::…` |
| 0.5% | 20 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec` |
| 0.5% | 18 | `<ikari_proto::v1::Envelope as prost::message::Message>::decode::<&[u8]>` |
| 0.5% | 18 | `<unicode_segmentation::grapheme::Graphemes as core::iter::traits::iterator::Iterator>::next` |
| 0.5% | 17 | `<shell_perf::ScriptedTransport as ikari_shell_core::host::EngineTransport>::post` |
| 0.4% | 16 | `<ikari_proto::v1::envelope::Body>::merge::<&mut &[u8]>` |
| 0.4% | 15 | `<ikari_shell_core::engine::EngineLink>::post` |
| 0.4% | 15 | `<ikari_shell_core::engine::EngineLink>::prefetch` |
| 0.4% | 15 | `libsystem_c.dylib!（記号なし）` |
| 0.4% | 14 | `<hashbrown::map::HashMap<&str, u32, std::hash::random::RandomState>>::rustc_entry` |
| 0.4% | 14 | `prost::encoding::encode_key::<alloc::vec::Vec<u8>>` |
| 0.3% | 12 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds>>::reset` |
| 0.3% | 12 | `<ikari_shell_core::engine::EngineLink>::prepare` |
| 0.3% | 12 | `<ikari_shell_core::machine::ContextText>::truncated` |
| 0.3% | 12 | `<ikari_shell_core::machine::InputStateMachine>::refresh_context_on_early_keystroke` |
| 0.3% | 11 | `<*mut u8>::add` |
| 0.3% | 11 | `<core::slice::iter::Iter<u8> as core::iter::traits::iterator::Iterator>::next` |
| 0.3% | 11 | `<ikari_proto::v1::Prefetch as prost::message::Message>::encode_raw::<alloc::vec::Vec<u8>>` |
| 0.3% | 11 | `<ikari_shell_core::machine::InputStateMachine>::handle_idle` |
| 0.3% | 10 | `<ikari_shell_core::english::EnglishMode>::chord` |
| 0.3% | 10 | `<unicode_segmentation::grapheme::GraphemeCursor>::grapheme_category` |
| 0.2% | 9 | `<ikari_shell_core::types::KeyInput as core::clone::Clone>::clone` |
| 0.2% | 9 | `<ikari_shell_core::types::MenuItem as core::cmp::PartialEq>::eq` |
| 0.2% | 9 | `<std::sys::pal::unix::sync::mutex::Mutex>::lock` |
| 0.2% | 9 | `core::str::count::do_count_chars` |
| 0.2% | 8 | `<alloc::vec::Vec<u8>>::as_slice` |
| 0.2% | 8 | `<core::option::Option<ikari_proto::v1::SurroundingText>>::map::<ikari_proto::v1::SurroundingText, ikari_shell_core::engine::codec::clipped::{closure#…` |
| 0.2% | 8 | `<ikari_shell_core::controller::background::Background>::drain` |
| 0.2% | 8 | `core::ptr::drop_glue::<ikari_shell_core::machine::state::InputState>` |
| 0.2% | 7 | `<ikari_shell_core::machine::InputStateMachine>::refresh_context` |
| 0.2% | 7 | `?!（記号なし）` |
| 0.2% | 7 | `alloc::alloc::alloc` |
| 0.2% | 7 | `prost::encoding::string::merge::<&mut &[u8]>` |
| 0.2% | 6 | `alloc::fmt::format::format_inner` |
| 0.2% | 6 | `ikari_shell_core::english::is_printable` |
| 0.2% | 6 | `prost::encoding::merge_loop::<ikari_proto::v1::SurroundingText, prost::encoding::message::merge<ikari_proto::v1::SurroundingText, &mut &[u8]>::{closu…` |
| 0.1% | 5 | `<alloc::vec::into_iter::IntoIter<ikari_shell_core::controller::background::JobResult> as core::ops::drop::Drop>::drop` |
| 0.1% | 5 | `core::ptr::drop_glue::<core::option::Option<ikari_proto::v1::envelope::Body>>` |
| 0.1% | 5 | `core::ptr::drop_glue::<ikari_shell_core::types::KeyInput>` |
| 0.1% | 4 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, i64, alloc::collections::btree::node…` |
| 0.1% | 4 | `<ikari_shell_core::english::EnglishMode>::mode_switch_target` |
| 0.1% | 4 | `<ikari_shell_core::machine::composer::Composer>::clear` |
| 0.1% | 4 | `<shell_perf::EngineLog>::record` |
| 0.1% | 4 | `<shell_perf::FixedClock as ikari_shell_core::host::Clock>::now` |
| 0.1% | 4 | `<std::sys::pal::unix::time::Timespec>::now` |
| 0.1% | 4 | `shell_perf::typing_reading_dispatch` |
| 0.1% | 3 | `<alloc::collections::btree::map::BTreeMap<alloc::string::String, alloc::string::String>>::entry` |
| 0.1% | 3 | `<ikari_proto::v1::DiagEvent as prost::message::Message>::merge_field::<&mut &[u8]>` |
| 0.1% | 3 | `<ikari_proto::v1::envelope::Body>::encode::<alloc::vec::Vec<u8>>` |
| 0.1% | 3 | `core::ptr::drop_glue::<ikari_proto::v1::envelope::Body>` |

## 呼び出しの途中も含めた時間の上位

| 割合 | サンプル | 関数 |
|---:|---:|---|
| 99.9% | 3743 | `<ikari_shell_core::controller::Shell>::dispatch` |
| 84.2% | 3156 | `<ikari_shell_core::machine::InputStateMachine>::handle` |
| 79.5% | 2980 | `<ikari_shell_core::machine::InputStateMachine>::dispatch` |
| 72.5% | 2719 | `shell_perf::typing_reading_dispatch` |
| 60.6% | 2270 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing` |
| 46.9% | 1758 | `<ikari_shell_core::engine::EngineLink>::post` |
| 44.7% | 1674 | `<ikari_shell_core::engine::EngineLink>::prefetch` |
| 31.1% | 1165 | `<shell_perf::ScriptedTransport as ikari_shell_core::host::EngineTransport>::post` |
| 26.2% | 982 | `libsystem_malloc.dylib!（記号なし）` |
| 21.5% | 807 | `libsystem_kernel.dylib!（記号なし）` |
| 18.5% | 695 | `libsystem_pthread.dylib!（記号なし）` |
| 17.3% | 647 | `shell_perf::typing_pending_dispatch` |
| 16.1% | 602 | `<std::sys::pal::unix::sync::mutex::Mutex>::lock` |
| 15.9% | 595 | `shell_perf-77dbd1fded01a9a9!（記号なし）` |
| 11.3% | 423 | `<ikari_proto::v1::Envelope as prost::message::Message>::decode::<&[u8]>` |
| 10.6% | 399 | `<ikari_proto::v1::envelope::Body>::merge::<&mut &[u8]>` |
| 10.2% | 382 | `shell_perf::typing_edit_dispatch` |
| 8.4% | 316 | `<ikari_shell_core::controller::Shell>::finish` |
| 8.3% | 311 | `prost::encoding::merge_loop::<ikari_proto::v1::Prefetch, prost::encoding::message::merge<ikari_proto::v1::Prefetch, &mut &[u8]>::{closure#0}, &mut &[…` |
| 7.7% | 289 | `libsystem_platform.dylib!（記号なし）` |
| 7.5% | 280 | `<core::slice::iter::Iter<u8> as core::iter::traits::iterator::Iterator>::next` |
| 7.5% | 280 | `<core::str::iter::Chars as core::iter::traits::iterator::Iterator>::next` |
| 7.5% | 280 | `<ikari_shell_core::machine::InputStateMachine>::feed` |
| 7.5% | 280 | `core::str::validations::next_code_point::<core::slice::iter::Iter<u8>>` |
| 7.4% | 276 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec` |
| 7.3% | 272 | `prost::encoding::string::merge::<&mut &[u8]>` |
| 7.2% | 269 | `<ikari_input::romaji::RomajiComposer>::push` |
| 7.2% | 268 | `<ikari_proto::v1::Prefetch as prost::message::Message>::merge_field::<&mut &[u8]>` |
| 7.1% | 267 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 6.4% | 241 | `<ikari_shell_core::machine::InputStateMachine>::emit_episode` |
| 6.4% | 241 | `<ikari_shell_core::machine::InputStateMachine>::reset_composition` |
| 5.5% | 207 | `<ikari_shell_core::engine::EngineLink>::prepare` |
| 5.5% | 206 | `<alloc::string::String as core::clone::Clone>::clone` |
| 5.3% | 200 | `<std::sys::pal::unix::time::Timespec>::now` |
| 5.1% | 190 | `libsystem_c.dylib!（記号なし）` |
| 5.0% | 186 | `<alloc::vec::Vec<u8> as prost::encoding::sealed::BytesAdapter>::replace_with::<bytes::buf::take::Take<&mut &mut &[u8]>>` |
| 5.0% | 186 | `<alloc::vec::Vec<u8>>::clear` |
| 4.6% | 172 | `<ikari_shell_core::engine::EngineLink as ikari_shell_core::machine::engine::MachineEngine>::diag` |
| 3.8% | 142 | `<ikari_shell_core::engine::EngineLink>::diag` |
| 3.8% | 142 | `<ikari_shell_core::engine::EngineLink>::post_engine_counts_if_any` |
| 3.8% | 142 | `<std::sync::poison::mutex::Mutex<ikari_shell_core::engine::LinkState>>::lock` |
| 3.8% | 142 | `<std::sys::sync::mutex::pthread::Mutex>::lock` |
| 3.8% | 142 | `ikari_shell_core::engine::lock::<ikari_shell_core::engine::LinkState>` |
| 3.5% | 133 | `<ikari_shell_core::controller::background::Background>::drain` |
| 3.1% | 116 | `<ikari_proto::v1::Prefetch as prost::message::Message>::encode_raw::<alloc::vec::Vec<u8>>` |
| 3.1% | 116 | `<std::time::Instant>::elapsed` |
| 2.2% | 81 | `core::ptr::drop_glue::<ikari_proto::v1::envelope::Body>` |
| 2.1% | 79 | `<ikari_shell_core::types::KeyInput as core::clone::Clone>::clone` |
| 2.1% | 78 | `<ikari_input::romaji::RomajiTable>::lookup` |
| 2.0% | 74 | `core::str::converts::from_utf8` |
| 2.0% | 74 | `core::str::validations::run_utf8_validation` |
| 1.9% | 71 | `<ikari_shell_core::keymap::KeyChord>::for_lookup` |
| 1.9% | 70 | `<alloc::raw_vec::RawVecInner>::try_allocate_in` |
| 1.9% | 70 | `<alloc::raw_vec::RawVecInner>::with_capacity_in` |
| 1.8% | 66 | `prost::encoding::merge_loop::<ikari_proto::v1::DiagEvent, prost::encoding::message::merge<ikari_proto::v1::DiagEvent, &mut &[u8]>::{closure#0}, &mut …` |
| 1.7% | 63 | `<ikari_proto::v1::DiagEvent as prost::message::Message>::merge_field::<&mut &[u8]>` |
| 1.7% | 63 | `core::ptr::drop_glue::<ikari_shell_core::types::KeyInput>` |
| 1.5% | 56 | `<shell_perf::EngineLog>::record` |
| 1.4% | 52 | `<hashbrown::map::HashMap<&str, u32, std::hash::random::RandomState>>::rustc_entry` |
| 1.4% | 51 | `prost::encoding::encode_key::<alloc::vec::Vec<u8>>` |
| 1.3% | 47 | `<ikari_shell_core::diag::DiagEpisode>::event` |
| 1.3% | 47 | `<ikari_shell_core::diag::DiagEvent>::new` |
| 1.2% | 46 | `<ikari_shell_core::machine::InputStateMachine>::handle_idle` |
| 1.1% | 43 | `<alloc::alloc::Global as core::alloc::Allocator>::allocate` |
| 1.1% | 43 | `<alloc::alloc::Global>::alloc_impl` |
| 1.1% | 43 | `<alloc::alloc::Global>::alloc_impl_runtime` |
| 1.1% | 43 | `<alloc::raw_vec::RawVec<ikari_shell_core::types::MenuItem>>::with_capacity_in` |
| 1.1% | 43 | `<alloc::vec::Vec<ikari_shell_core::types::MenuItem>>::with_capacity` |
| 1.1% | 43 | `<alloc::vec::Vec<ikari_shell_core::types::MenuItem>>::with_capacity_in` |
| 1.1% | 43 | `alloc::alloc::alloc` |

