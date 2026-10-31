# CPU プロファイル（samply）

- 対象: 殻のキー 1 回（shell_perf --metric p1）
- サンプル数: 4311（1000 Hz。全スレッドの合計）
- 数える範囲: `shell_perf::steady_dispatch` を含むサンプルだけ（それより外側は数えない）
- 測定日時（UTC）: 2026-10-31T08:10:50Z
- コミット: a1f01acc7996fbbef41e87aaaec110e89fcf6e99 （perf/shell-core-candidate-clones。作業ツリーの変更: あり）
- 負荷平均（1 分。測定の前）: 2.85
- 記録: `target/perf-tools/profile.json`（`samply load` でブラウザに開ける）

## 自分の時間（いちばん内側の関数）の上位

| 割合 | サンプル | 関数 |
|---:|---:|---|
| 27.0% | 1162 | `libsystem_malloc.dylib!（記号なし）` |
| 10.9% | 470 | `<unicode_segmentation::grapheme::Graphemes as core::iter::traits::iterator::Iterator>::next` |
| 10.7% | 461 | `libsystem_kernel.dylib!（記号なし）` |
| 9.7% | 418 | `shell_perf-77dbd1fded01a9a9!（記号なし）` |
| 8.6% | 372 | `libsystem_platform.dylib!（記号なし）` |
| 2.9% | 127 | `core::str::validations::run_utf8_validation` |
| 1.3% | 56 | `<ikari_shell_core::scenario::ScenarioSurrounding>::to_host` |
| 1.3% | 55 | `<alloc::string::String as core::clone::Clone>::clone` |
| 1.3% | 55 | `libsystem_pthread.dylib!（記号なし）` |
| 1.1% | 47 | `<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut, alloc::string::String, ikari_input::romaji::Rule, alloc::co…` |
| 1.1% | 47 | `__rustc::__rdl_alloc` |
| 1.0% | 45 | `<unicode_segmentation::grapheme::GraphemeCursor>::grapheme_category` |
| 0.9% | 38 | `<ikari_shell_core::controller::Shell>::finish` |
| 0.8% | 34 | `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write` |
| 0.7% | 31 | `<ikari_input::romaji::RomajiComposer>::push` |
| 0.6% | 27 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing` |
| 0.6% | 26 | `<ikari_proto::v1::Envelope as prost::message::Message>::encode_to_vec` |
| 0.6% | 26 | `<ikari_shell_core::keymap::KeyChord>::for_lookup` |
| 0.5% | 23 | `<ikari_shell_core::controller::Shell>::dispatch` |
| 0.5% | 22 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 0.5% | 22 | `<alloc::vec::Vec<u8> as prost::encoding::sealed::BytesAdapter>::replace_with::<bytes::buf::take::Take<&mut &mut &[u8]>>` |
| 0.5% | 20 | `<[ikari_shell_core::types::CandidateItem] as alloc::slice::SpecCloneIntoVec<ikari_shell_core::types::CandidateItem, alloc::alloc::Global>>::clone_into` |
| 0.5% | 20 | `<ikari_shell_core::machine::InputStateMachine>::dispatch` |
| 0.5% | 20 | `prost::encoding::string::merge::<&mut &[u8]>` |
| 0.4% | 19 | `<ikari_shell_core::machine::InputStateMachine>::handle` |

## 呼び出しの途中も含めた時間の上位

| 割合 | サンプル | 関数 |
|---:|---:|---|
| 100.0% | 4311 | `shell_perf::steady_dispatch` |
| 99.9% | 4308 | `<ikari_shell_core::controller::Shell>::dispatch` |
| 88.2% | 3801 | `<ikari_shell_core::machine::InputStateMachine>::handle` |
| 86.7% | 3739 | `<ikari_shell_core::machine::InputStateMachine>::dispatch` |
| 28.6% | 1234 | `libsystem_malloc.dylib!（記号なし）` |
| 27.5% | 1187 | `<ikari_shell_core::machine::InputStateMachine>::refresh_composing` |
| 27.2% | 1173 | `<ikari_shell_core::engine::EngineLink>::post` |
| 26.1% | 1126 | `shell_perf-77dbd1fded01a9a9!（記号なし）` |
| 19.0% | 821 | `<unicode_segmentation::grapheme::Graphemes as core::iter::traits::iterator::Iterator>::count` |
| 19.0% | 821 | `<unicode_segmentation::grapheme::Graphemes as core::iter::traits::iterator::Iterator>::fold::<usize, <unicode_segmentation::grapheme::Graphemes as co…` |
| 19.0% | 821 | `ikari_shell_core::machine::grapheme_count` |
| 18.3% | 788 | `<shell_perf::ScriptedTransport as ikari_shell_core::host::EngineTransport>::post` |
| 18.2% | 784 | `<alloc::string::String>::as_str` |
| 18.2% | 784 | `<alloc::vec::Vec<u8>>::as_slice` |
| 18.2% | 784 | `<ikari_shell_core::machine::InputStateMachine>::start_conversion` |
| 18.2% | 783 | `<alloc::raw_vec::RawVec<u8>>::ptr` |
| 18.2% | 783 | `<alloc::raw_vec::RawVecInner>::non_null::<u8>` |
| 18.2% | 783 | `<alloc::raw_vec::RawVecInner>::ptr::<u8>` |
| 18.2% | 783 | `<alloc::string::String as core::ops::deref::Deref>::deref` |
| 18.2% | 783 | `<alloc::vec::Vec<u8>>::as_ptr` |
| 17.3% | 747 | `<ikari_shell_core::engine::EngineLink>::prefetch` |
| 16.4% | 709 | `<ikari_shell_core::machine::InputStateMachine>::reset_composition` |
| 13.7% | 589 | `<ikari_proto::v1::Envelope as prost::message::Message>::decode::<&[u8]>` |
| 13.4% | 577 | `<ikari_shell_core::machine::InputStateMachine>::commit_converting` |
| 13.2% | 570 | `<ikari_proto::v1::envelope::Body>::merge::<&mut &[u8]>` |

