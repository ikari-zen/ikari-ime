# CPU プロファイル（samply）

- 対象: 殻のキー 1 回（shell_perf --metric p1）
- サンプル数: 5067（1000 Hz。全スレッドの合計）
- 数える範囲: `shell_perf::typing_` を含むサンプルだけ（それより外側は数えない）
- 測定日時（UTC）: 2026-10-31T09:09:44Z
- コミット: ef3d321ae895087668082d6dbecce40ac3f8c7e9 （perf/measure-typing。作業ツリーの変更: あり）
- 負荷平均（1 分。測定の前）: 2.57
- 記録: `target/perf-tools/profile.json`（`samply load` でブラウザに開ける）

## 自分の時間（いちばん内側の関数）の上位

| 割合 | サンプル | 関数 |
|---:|---:|---|
| 22.4% | 1134 | `libsystem_malloc.dylib!（記号なし）` |
| 15.4% | 778 | `libsystem_kernel.dylib!（記号なし）` |
| 11.3% | 574 | `libsystem_platform.dylib!（記号なし）` |
| 10.0% | 506 | `<unicode_segmentation::grapheme::Graphemes as core::iter::traits::iterator::Iterator>::next` |
| 7.7% | 390 | `shell_perf-77dbd1fded01a9a9!（記号なし）` |
| 3.3% | 168 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, ikari_input::romaji::Rule, alloc::co…` |
| 2.5% | 125 | `libsystem_pthread.dylib!（記号なし）` |
| 2.4% | 122 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing` |
| 2.4% | 120 | `<ikari_input::romaji::RomajiComposer>::push` |
| 1.5% | 74 | `<ikari_shell_core::keymap::KeyChord>::for_lookup` |
| 1.2% | 62 | `<unicode_segmentation::grapheme::GraphemeCursor>::grapheme_category` |
| 1.2% | 60 | `core::str::validations::run_utf8_validation` |
| 0.9% | 45 | `<alloc::vec::Vec<u8> as prost::encoding::sealed::BytesAdapter>::replace_with::<bytes::buf::take::Take<&mut &mut &[u8]>>` |
| 0.9% | 45 | `<ikari_shell_core::controller::Shell>::dispatch` |
| 0.9% | 44 | `<alloc::string::String as core::clone::Clone>::clone` |
| 0.8% | 43 | `__rustc::__rdl_alloc` |
| 0.6% | 30 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, ikari_input::romaji::Rule, alloc::co…` |
| 0.6% | 30 | `<ikari_shell_core::machine::InputStateMachine>::handle` |
| 0.5% | 27 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, ikari_input::romaji::Rule, alloc::co…` |
| 0.5% | 27 | `<ikari_shell_core::controller::Shell>::finish` |
| 0.5% | 27 | `<ikari_shell_core::machine::InputStateMachine>::dispatch` |
| 0.5% | 25 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec` |
| 0.5% | 23 | `alloc::fmt::format::format_inner` |
| 0.4% | 22 | `<alloc::string::String as core::fmt::Write>::write_str` |
| 0.4% | 22 | `prost::encoding::merge_loop::<ikari_proto::v1::Prefetch, prost::encoding::message::merge<ikari_proto::v1::Prefetch, &mut &[u8]>::{closure#0}, &mut &[…` |
| 0.4% | 21 | `<ikari_proto::v1::envelope::Body>::merge::<&mut &[u8]>` |
| 0.4% | 20 | `<ikari_shell_core::engine::EngineLink>::post` |
| 0.4% | 20 | `core::fmt::write` |
| 0.4% | 19 | `<ikari_proto::v1::Envelope as prost::message::Message>::decode::<&[u8]>` |
| 0.4% | 19 | `libsystem_c.dylib!（記号なし）` |
| 0.4% | 18 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 0.4% | 18 | `<ikari_shell_core::english::EnglishMode>::chord` |
| 0.3% | 17 | `<ikari_proto::v1::Prefetch as prost::message::Message>::merge_field::<&mut &[u8]>` |
| 0.3% | 17 | `<ikari_shell_core::engine::EngineLink>::prepare` |
| 0.3% | 16 | `<ikari_shell_core::types::MenuItem as core::slice::cmp::SlicePartialEq<ikari_shell_core::types::MenuItem>>::equal_same_length` |
| 0.3% | 15 | `<alloc::vec::into_iter::IntoIter<ikari_shell_core::controller::background::JobResult> as core::ops::drop::Drop>::drop` |
| 0.3% | 15 | `core::ptr::drop_glue::<ikari_shell_core::machine::state::InputState>` |
| 0.3% | 14 | `<core::slice::iter::Iter<u8> as core::iter::traits::iterator::Iterator>::fold::<usize, core::iter::adapters::map::map_fold<&u8, usize, usize, <core::…` |
| 0.3% | 14 | `<ikari_proto::v1::Prefetch as prost::message::Message>::encode_raw::<alloc::vec::Vec<u8>>` |
| 0.2% | 12 | `<core::fmt::Formatter>::pad` |
| 0.2% | 12 | `<hashbrown::map::HashMap<&str, u32, std::hash::random::RandomState>>::rustc_entry` |
| 0.2% | 12 | `<shell_perf::ScriptedTransport as ikari_shell_core::host::EngineTransport>::post` |
| 0.2% | 12 | `ikari_shell_core::menu::build` |
| 0.2% | 11 | `<*mut u8>::add` |
| 0.2% | 11 | `<ikari_shell_core::engine::EngineLink>::prefetch` |
| 0.2% | 11 | `prost::encoding::message::encode::<ikari_proto::v1::Prefetch, alloc::vec::Vec<u8>>` |
| 0.2% | 10 | `<ikari_shell_core::machine::InputStateMachine>::refresh_context_on_early_keystroke` |
| 0.2% | 10 | `<std::sys::pal::unix::sync::mutex::Mutex>::lock` |
| 0.2% | 10 | `core::str::count::do_count_chars` |
| 0.2% | 10 | `ikari_shell_core::engine::codec::clipped` |
| 0.2% | 9 | `<ikari_shell_core::english::EnglishMode>::mode_switch_target` |
| 0.2% | 9 | `prost::encoding::string::merge::<&mut &[u8]>` |
| 0.2% | 8 | `<ikari_proto::v1::SurroundingText as prost::message::Message>::encode_raw::<alloc::vec::Vec<u8>>` |
| 0.2% | 8 | `<ikari_shell_core::controller::background::Background>::drain` |
| 0.1% | 7 | `<core::slice::iter::Iter<u8> as core::iter::traits::iterator::Iterator>::next` |
| 0.1% | 7 | `<ikari_shell_core::types::KeyInput as core::clone::Clone>::clone` |
| 0.1% | 6 | `<alloc::collections::btree::node::Handle<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Mut, alloc::string::String…` |
| 0.1% | 6 | `<ikari_shell_core::scenario::ScenarioSurrounding>::to_host` |
| 0.1% | 6 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&&str>` |
| 0.1% | 6 | `core::ptr::drop_glue::<ikari_shell_core::types::KeyInput>` |
| 0.1% | 6 | `ikari_shell_core::english::is_printable` |
| 0.1% | 5 | `<ikari_shell_core::machine::InputStateMachine>::handle_idle` |
| 0.1% | 5 | `<ikari_shell_core::machine::InputStateMachine>::refresh_context` |
| 0.1% | 5 | `<std::sys::pal::unix::time::Timespec>::sub_timespec` |
| 0.1% | 5 | `prost::encoding::merge_loop::<ikari_proto::v1::SurroundingText, prost::encoding::message::merge<ikari_proto::v1::SurroundingText, &mut &[u8]>::{closu…` |
| 0.1% | 4 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 0.1% | 4 | `<alloc::vec::Vec<ikari_shell_core::scenario::ScriptFailure>>::as_slice` |
| 0.1% | 4 | `<ikari_shell_core::controller::Shell>::sync_settings` |
| 0.1% | 4 | `<ikari_shell_core::machine::ContextText>::truncated` |
| 0.1% | 4 | `<shell_perf::EngineLog>::record` |

## 呼び出しの途中も含めた時間の上位

| 割合 | サンプル | 関数 |
|---:|---:|---|
| 99.9% | 5060 | `<ikari_shell_core::controller::Shell>::dispatch` |
| 89.2% | 4521 | `<ikari_shell_core::machine::InputStateMachine>::handle` |
| 86.1% | 4365 | `<ikari_shell_core::machine::InputStateMachine>::dispatch` |
| 69.5% | 3524 | `shell_perf::typing_reading_dispatch` |
| 60.2% | 3052 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing` |
| 32.6% | 1653 | `<ikari_shell_core::engine::EngineLink>::post` |
| 31.8% | 1611 | `<ikari_shell_core::engine::EngineLink>::prefetch` |
| 24.7% | 1253 | `shell_perf::typing_pending_dispatch` |
| 24.0% | 1218 | `libsystem_malloc.dylib!（記号なし）` |
| 22.6% | 1147 | `<shell_perf::ScriptedTransport as ikari_shell_core::host::EngineTransport>::post` |
| 19.1% | 970 | `<core::slice::iter::Iter<u8> as core::iter::traits::iterator::Iterator>::next` |
| 19.1% | 970 | `<core::str::iter::Chars as core::iter::traits::iterator::Iterator>::next` |
| 19.1% | 970 | `<ikari_shell_core::machine::InputStateMachine>::feed` |
| 19.1% | 970 | `core::str::validations::next_code_point::<core::slice::iter::Iter<u8>>` |
| 19.0% | 963 | `<ikari_input::romaji::RomajiComposer>::push` |
| 15.4% | 778 | `libsystem_kernel.dylib!（記号なし）` |
| 12.7% | 642 | `libsystem_pthread.dylib!（記号なし）` |
| 11.3% | 574 | `libsystem_platform.dylib!（記号なし）` |
| 11.1% | 561 | `<unicode_segmentation::grapheme::Graphemes as core::iter::traits::iterator::Iterator>::next` |
| 11.1% | 560 | `<std::sys::pal::unix::sync::mutex::Mutex>::lock` |
| 11.0% | 559 | `shell_perf-77dbd1fded01a9a9!（記号なし）` |
| 8.9% | 452 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, ikari_input::romaji::Rule, alloc::co…` |
| 7.8% | 395 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, ikari_input::romaji::Rule, alloc::co…` |
| 7.1% | 362 | `<ikari_proto::v1::Envelope as prost::message::Message>::decode::<&[u8]>` |
| 6.8% | 343 | `<ikari_proto::v1::envelope::Body>::merge::<&mut &[u8]>` |
| 5.7% | 290 | `shell_perf::typing_edit_dispatch` |
| 5.6% | 285 | `alloc::fmt::format::format_inner` |
| 5.3% | 269 | `prost::encoding::merge_loop::<ikari_proto::v1::Prefetch, prost::encoding::message::merge<ikari_proto::v1::Prefetch, &mut &[u8]>::{closure#0}, &mut &[…` |
| 5.2% | 262 | `core::fmt::write` |
| 4.9% | 246 | `<ikari_shell_core::controller::Shell>::finish` |
| 4.6% | 235 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 4.5% | 227 | `<alloc::string::String as core::fmt::Write>::write_str` |
| 4.4% | 225 | `<ikari_proto::v1::Prefetch as prost::message::Message>::merge_field::<&mut &[u8]>` |
| 4.4% | 224 | `prost::encoding::string::merge::<&mut &[u8]>` |
| 4.3% | 219 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec` |
| 4.2% | 215 | `<std::sys::pal::unix::time::Timespec>::now` |
| 4.1% | 208 | `libsystem_c.dylib!（記号なし）` |
| 3.7% | 188 | `<alloc::string::String as core::clone::Clone>::clone` |
| 3.3% | 165 | `<alloc::raw_vec::RawVecInner>::finish_grow` |
| 3.2% | 162 | `<alloc::sync::Arc<dyn ikari_shell_core::host::Clock> as core::ops::deref::Deref>::deref` |
| 3.2% | 162 | `<ikari_shell_core::machine::InputStateMachine>::emit_episode` |
| 3.2% | 162 | `<ikari_shell_core::machine::InputStateMachine>::now` |
| 3.2% | 162 | `<ikari_shell_core::machine::InputStateMachine>::reset_composition` |
| 3.0% | 152 | `<alloc::vec::Vec<u8> as prost::encoding::sealed::BytesAdapter>::replace_with::<bytes::buf::take::Take<&mut &mut &[u8]>>` |
| 2.8% | 142 | `<ikari_shell_core::engine::EngineLink>::prepare` |
| 2.5% | 129 | `<std::time::Instant>::elapsed` |
| 2.1% | 106 | `<ikari_shell_core::engine::EngineLink as ikari_shell_core::machine::engine::MachineEngine>::diag` |
| 2.0% | 100 | `<ikari_proto::v1::Prefetch as prost::message::Message>::encode_raw::<alloc::vec::Vec<u8>>` |
| 1.8% | 91 | `<ikari_shell_core::engine::EngineLink>::diag` |
| 1.8% | 91 | `<ikari_shell_core::engine::EngineLink>::post_engine_counts_if_any` |
| 1.8% | 91 | `<std::sync::poison::mutex::Mutex<ikari_shell_core::engine::LinkState>>::lock` |
| 1.8% | 91 | `<std::sys::sync::mutex::pthread::Mutex>::lock` |
| 1.8% | 91 | `ikari_shell_core::engine::lock::<ikari_shell_core::engine::LinkState>` |
| 1.7% | 84 | `<ikari_shell_core::types::KeyInput as core::clone::Clone>::clone` |
| 1.6% | 81 | `<ikari_shell_core::controller::background::Background>::drain` |
| 1.6% | 81 | `<ikari_shell_core::keymap::KeyChord>::for_lookup` |
| 1.5% | 75 | `<ikari_shell_core::machine::InputStateMachine>::handle_idle` |
| 1.3% | 67 | `<shell_perf::EngineLog>::record` |
| 1.2% | 63 | `<hashbrown::map::HashMap<&str, u32, std::hash::random::RandomState>>::rustc_entry` |
| 1.2% | 62 | `<unicode_segmentation::grapheme::GraphemeCursor>::grapheme_category` |
| 1.2% | 61 | `core::ptr::drop_glue::<ikari_shell_core::machine::state::InputState>` |
| 1.2% | 60 | `core::ptr::drop_glue::<ikari_shell_core::types::KeyInput>` |
| 1.2% | 60 | `core::str::converts::from_utf8` |
| 1.2% | 60 | `core::str::validations::run_utf8_validation` |
| 1.0% | 51 | `core::ptr::drop_glue::<ikari_proto::v1::envelope::Body>` |
| 0.9% | 48 | `<ikari_proto::v1::DiagEvent as prost::message::Message>::merge_field::<&mut &[u8]>` |
| 0.9% | 48 | `prost::encoding::merge_loop::<ikari_proto::v1::DiagEvent, prost::encoding::message::merge<ikari_proto::v1::DiagEvent, &mut &[u8]>::{closure#0}, &mut …` |
| 0.9% | 47 | `<ikari_shell_core::diag::DiagEpisode>::event` |
| 0.9% | 45 | `<std::hash::random::RandomState as core::hash::BuildHasher>::hash_one::<&&str>` |
| 0.8% | 43 | `__rustc::__rdl_alloc` |

