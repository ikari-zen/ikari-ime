# CPU プロファイル（samply）

- 対象: 殻のキー 1 回（shell_perf --metric p1）
- サンプル数: 4411（1000 Hz。全スレッドの合計）
- 数える範囲: `shell_perf::typing_` を含むサンプルだけ（それより外側は数えない）
- 測定日時（UTC）: 2026-10-31T09:20:22Z
- コミット: f76a62e6d781466700010e7514db82d82d974880 （perf/shell-core-typing。作業ツリーの変更: あり）
- 負荷平均（1 分。測定の前）: 2.86
- 記録: `target/perf-tools/profile.json`（`samply load` でブラウザに開ける）

## 自分の時間（いちばん内側の関数）の上位

| 割合 | サンプル | 関数 |
|---:|---:|---|
| 22.9% | 1011 | `libsystem_malloc.dylib!（記号なし）` |
| 15.8% | 695 | `libsystem_kernel.dylib!（記号なし）` |
| 15.1% | 666 | `libsystem_platform.dylib!（記号なし）` |
| 9.2% | 406 | `shell_perf-77dbd1fded01a9a9!（記号なし）` |
| 3.5% | 156 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, ikari_input::romaji::Rule, alloc::co…` |
| 3.1% | 136 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing` |
| 3.0% | 131 | `libsystem_pthread.dylib!（記号なし）` |
| 2.5% | 112 | `<ikari_input::romaji::RomajiComposer>::push` |
| 1.7% | 76 | `<ikari_shell_core::keymap::KeyChord>::for_lookup` |
| 1.5% | 66 | `core::str::validations::run_utf8_validation` |
| 1.2% | 51 | `<ikari_shell_core::controller::Shell>::finish` |
| 1.0% | 44 | `<ikari_shell_core::controller::Shell>::dispatch` |
| 0.9% | 40 | `<alloc::string::String as core::clone::Clone>::clone` |
| 0.9% | 38 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, ikari_input::romaji::Rule, alloc::co…` |
| 0.8% | 35 | `<alloc::vec::Vec<u8> as prost::encoding::sealed::BytesAdapter>::replace_with::<bytes::buf::take::Take<&mut &mut &[u8]>>` |
| 0.8% | 34 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec` |
| 0.8% | 34 | `<ikari_proto::v1::Prefetch as prost::message::Message>::merge_field::<&mut &[u8]>` |
| 0.7% | 29 | `prost::encoding::encode_key::<alloc::vec::Vec<u8>>` |
| 0.6% | 27 | `<ikari_proto::v1::Envelope as prost::message::Message>::decode::<&[u8]>` |
| 0.6% | 27 | `<ikari_shell_core::engine::EngineLink>::post` |
| 0.6% | 26 | `<ikari_shell_core::machine::InputStateMachine>::handle` |
| 0.6% | 26 | `__rustc::__rdl_alloc` |
| 0.6% | 25 | `<ikari_shell_core::engine::EngineLink>::prefetch` |
| 0.5% | 23 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, ikari_input::romaji::Rule, alloc::co…` |
| 0.5% | 23 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 0.5% | 22 | `prost::encoding::merge_loop::<ikari_proto::v1::Prefetch, prost::encoding::message::merge<ikari_proto::v1::Prefetch, &mut &[u8]>::{closure#0}, &mut &[…` |
| 0.5% | 20 | `<core::slice::iter::Iter<u8> as core::iter::traits::iterator::Iterator>::fold::<usize, core::iter::adapters::map::map_fold<&u8, usize, usize, <core::…` |
| 0.4% | 19 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&&str>` |
| 0.4% | 18 | `<ikari_shell_core::machine::InputStateMachine>::dispatch` |
| 0.4% | 17 | `<ikari_proto::v1::envelope::Body>::merge::<&mut &[u8]>` |
| 0.4% | 17 | `<ikari_shell_core::engine::EngineLink>::prepare` |
| 0.4% | 17 | `libsystem_c.dylib!（記号なし）` |
| 0.4% | 16 | `<shell_perf::ScriptedTransport as ikari_shell_core::host::EngineTransport>::post` |
| 0.3% | 15 | `<core::option::Option<ikari_shell_core::types::MenuCommand> as core::cmp::PartialEq>::eq` |
| 0.3% | 15 | `<hashbrown::map::HashMap<&str, u32, std::hash::random::RandomState>>::rustc_entry` |
| 0.3% | 14 | `core::ptr::drop_glue::<ikari_shell_core::types::KeyInput>` |
| 0.3% | 13 | `<ikari_proto::v1::Prefetch as prost::message::Message>::encode_raw::<alloc::vec::Vec<u8>>` |
| 0.3% | 12 | `<ikari_shell_core::machine::InputStateMachine>::refresh_context_on_early_keystroke` |
| 0.2% | 11 | `<ikari_shell_core::english::EnglishMode>::chord` |
| 0.2% | 10 | `alloc::alloc::alloc` |
| 0.2% | 10 | `alloc::fmt::format::format_inner` |
| 0.2% | 9 | `<ikari_shell_core::machine::InputStateMachine>::handle_idle` |
| 0.2% | 9 | `<std::sys::pal::unix::time::Timespec>::now` |
| 0.2% | 9 | `prost::encoding::string::merge::<&mut &[u8]>` |
| 0.2% | 8 | `<*mut u8>::add` |
| 0.2% | 8 | `<alloc::vec::into_iter::IntoIter<ikari_shell_core::controller::background::JobResult> as core::ops::drop::Drop>::drop` |
| 0.2% | 8 | `<core::option::Option<ikari_proto::v1::SurroundingText>>::map::<ikari_proto::v1::SurroundingText, ikari_shell_core::engine::codec::clipped::{closure#…` |
| 0.2% | 8 | `?!（記号なし）` |
| 0.2% | 8 | `ikari_shell_core::english::is_printable` |
| 0.2% | 7 | `<core::slice::iter::Iter<u8> as core::iter::traits::iterator::Iterator>::next` |
| 0.2% | 7 | `core::ptr::drop_glue::<core::option::Option<ikari_proto::v1::envelope::Body>>` |
| 0.2% | 7 | `core::str::count::do_count_chars` |
| 0.1% | 6 | `<ikari_shell_core::machine::ContextText>::truncated` |
| 0.1% | 6 | `<unicode_segmentation::grapheme::GraphemeCursor>::grapheme_category` |
| 0.1% | 6 | `<unicode_segmentation::grapheme::Graphemes as core::iter::traits::iterator::Iterator>::next` |
| 0.1% | 6 | `core::ptr::drop_glue::<ikari_proto::v1::envelope::Body>` |
| 0.1% | 6 | `prost::encoding::merge_loop::<ikari_proto::v1::SurroundingText, prost::encoding::message::merge<ikari_proto::v1::SurroundingText, &mut &[u8]>::{closu…` |
| 0.1% | 5 | `<ikari_shell_core::controller::Shell>::sync_settings` |
| 0.1% | 5 | `<ikari_shell_core::controller::background::Background>::drain` |
| 0.1% | 5 | `<ikari_shell_core::english::EnglishMode>::mode_switch_target` |
| 0.1% | 5 | `<ikari_shell_core::machine::composer::Composer>::clear` |
| 0.1% | 5 | `<ikari_shell_core::types::KeyKind as core::clone::Clone>::clone` |
| 0.1% | 5 | `<shell_perf::FixedClock as ikari_shell_core::host::Clock>::now` |
| 0.1% | 5 | `<std::sys::pal::unix::sync::mutex::Mutex>::lock` |
| 0.1% | 5 | `<std::sys::time::unix::Instant>::now` |
| 0.1% | 5 | `std::sys::alloc::unix::dealloc` |
| 0.1% | 4 | `<alloc::collections::btree::map::entry::VacantEntry<alloc::string::String, i64>>::insert_entry` |
| 0.1% | 4 | `<alloc::vec::Vec<ikari_shell_core::types::ShellAction> as alloc::vec::spec_extend::SpecExtend<ikari_shell_core::types::ShellAction, alloc::vec::into_…` |
| 0.1% | 4 | `core::ptr::drop_glue::<ikari_shell_core::machine::state::InputState>` |
| 0.1% | 4 | `prost::encoding::message::merge::<ikari_proto::v1::DiagNum, &mut &[u8]>` |

## 呼び出しの途中も含めた時間の上位

| 割合 | サンプル | 関数 |
|---:|---:|---|
| 99.9% | 4408 | `<ikari_shell_core::controller::Shell>::dispatch` |
| 86.4% | 3813 | `<ikari_shell_core::machine::InputStateMachine>::handle` |
| 82.7% | 3647 | `<ikari_shell_core::machine::InputStateMachine>::dispatch` |
| 70.8% | 3124 | `shell_perf::typing_reading_dispatch` |
| 48.8% | 2153 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing` |
| 38.1% | 1679 | `<ikari_shell_core::engine::EngineLink>::post` |
| 36.3% | 1603 | `<ikari_shell_core::engine::EngineLink>::prefetch` |
| 25.4% | 1122 | `libsystem_malloc.dylib!（記号なし）` |
| 24.9% | 1098 | `<shell_perf::ScriptedTransport as ikari_shell_core::host::EngineTransport>::post` |
| 24.7% | 1088 | `<core::slice::iter::Iter<u8> as core::iter::traits::iterator::Iterator>::next` |
| 24.7% | 1088 | `<core::str::iter::Chars as core::iter::traits::iterator::Iterator>::next` |
| 24.7% | 1088 | `<ikari_shell_core::machine::InputStateMachine>::feed` |
| 24.7% | 1088 | `core::str::validations::next_code_point::<core::slice::iter::Iter<u8>>` |
| 24.5% | 1081 | `<ikari_input::romaji::RomajiComposer>::push` |
| 21.0% | 928 | `shell_perf::typing_pending_dispatch` |
| 15.8% | 695 | `libsystem_kernel.dylib!（記号なし）` |
| 15.1% | 667 | `shell_perf-77dbd1fded01a9a9!（記号なし）` |
| 15.1% | 666 | `libsystem_platform.dylib!（記号なし）` |
| 13.0% | 572 | `libsystem_pthread.dylib!（記号なし）` |
| 11.6% | 510 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, ikari_input::romaji::Rule, alloc::co…` |
| 10.5% | 463 | `<std::sys::pal::unix::sync::mutex::Mutex>::lock` |
| 10.2% | 449 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, ikari_input::romaji::Rule, alloc::co…` |
| 9.5% | 421 | `<ikari_proto::v1::Envelope as prost::message::Message>::decode::<&[u8]>` |
| 8.8% | 389 | `<ikari_proto::v1::envelope::Body>::merge::<&mut &[u8]>` |
| 8.1% | 359 | `shell_perf::typing_edit_dispatch` |
| 6.8% | 298 | `<ikari_shell_core::controller::Shell>::finish` |
| 6.5% | 287 | `prost::encoding::merge_loop::<ikari_proto::v1::Prefetch, prost::encoding::message::merge<ikari_proto::v1::Prefetch, &mut &[u8]>::{closure#0}, &mut &[…` |
| 5.6% | 246 | `<ikari_proto::v1::Prefetch as prost::message::Message>::merge_field::<&mut &[u8]>` |
| 5.5% | 242 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec` |
| 5.4% | 239 | `prost::encoding::string::merge::<&mut &[u8]>` |
| 5.4% | 238 | `<ikari_shell_core::machine::InputStateMachine>::reset_composition` |
| 4.6% | 202 | `<alloc::string::String as core::clone::Clone>::clone` |
| 4.4% | 194 | `<ikari_shell_core::engine::EngineLink>::prepare` |
| 4.3% | 191 | `<std::sys::pal::unix::time::Timespec>::now` |
| 4.0% | 176 | `libsystem_c.dylib!（記号なし）` |
| 3.8% | 167 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 3.7% | 163 | `<ikari_shell_core::engine::EngineLink as ikari_shell_core::machine::engine::MachineEngine>::diag` |
| 3.6% | 160 | `<alloc::vec::Vec<u8> as prost::encoding::sealed::BytesAdapter>::replace_with::<bytes::buf::take::Take<&mut &mut &[u8]>>` |
| 3.1% | 138 | `<ikari_shell_core::engine::EngineLink>::diag` |
| 3.1% | 135 | `<alloc::raw_vec::RawVecInner>::try_allocate_in` |
| 3.1% | 135 | `<alloc::raw_vec::RawVecInner>::with_capacity_in` |
| 2.8% | 123 | `<std::sys::time::unix::Instant>::now` |
| 2.8% | 123 | `<std::time::Instant>::now` |
| 2.8% | 122 | `<std::time::Instant>::elapsed` |
| 2.7% | 120 | `<alloc::alloc::Global as core::alloc::Allocator>::allocate` |
| 2.7% | 120 | `<alloc::alloc::Global>::alloc_impl` |
| 2.7% | 120 | `<alloc::alloc::Global>::alloc_impl_runtime` |
| 2.7% | 120 | `alloc::alloc::alloc` |
| 2.2% | 96 | `<ikari_proto::v1::Prefetch as prost::message::Message>::encode_raw::<alloc::vec::Vec<u8>>` |
| 2.0% | 90 | `<ikari_shell_core::machine::InputStateMachine>::handle_idle` |
| 1.9% | 83 | `<shell_perf::EngineLog>::record` |
| 1.9% | 82 | `<ikari_shell_core::keymap::KeyChord>::for_lookup` |
| 1.8% | 80 | `<hashbrown::map::HashMap<&str, u32, std::hash::random::RandomState>>::rustc_entry` |
| 1.8% | 78 | `<ikari_shell_core::controller::background::Background>::drain` |
| 1.8% | 78 | `<ikari_shell_core::types::KeyInput as core::clone::Clone>::clone` |
| 1.8% | 78 | `<ikari_shell_core::types::KeyKind as core::clone::Clone>::clone` |
| 1.8% | 78 | `core::ptr::drop_glue::<ikari_shell_core::types::KeyInput>` |
| 1.7% | 77 | `prost::encoding::merge_loop::<ikari_proto::v1::DiagEvent, prost::encoding::message::merge<ikari_proto::v1::DiagEvent, &mut &[u8]>::{closure#0}, &mut …` |
| 1.7% | 76 | `<ikari_proto::v1::DiagEvent as prost::message::Message>::merge_field::<&mut &[u8]>` |
| 1.5% | 66 | `core::str::converts::from_utf8` |
| 1.5% | 66 | `core::str::validations::run_utf8_validation` |
| 1.4% | 61 | `core::ptr::drop_glue::<ikari_proto::v1::envelope::Body>` |
| 1.3% | 58 | `<[u8] as alloc::borrow::ToOwned>::to_owned` |
| 1.3% | 58 | `<[u8]>::to_vec` |
| 1.3% | 58 | `<[u8]>::to_vec_in::<alloc::alloc::Global>` |
| 1.3% | 58 | `<alloc::raw_vec::RawVec<u8>>::with_capacity_in` |
| 1.3% | 58 | `<alloc::string::String as core::convert::From<&str>>::from` |
| 1.3% | 58 | `<alloc::vec::Vec<u8>>::with_capacity_in` |
| 1.3% | 58 | `<ikari_shell_core::diag::DiagEpisode>::event` |
| 1.3% | 58 | `<ikari_shell_core::diag::DiagEvent>::new` |

