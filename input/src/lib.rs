//! ikari-input: 入力（ローマ字、JISかな、字種変換）。辞書に依存しない純粋な処理（ADR 0004 §3.3）。
//!
//! 殻（IME のプロセス）が辞書や llama.cpp を持たずにリンクできるよう、core から分けている。
//! `ikari-core` は再公開しない。利用側（cli、shell-ffi）は `ikari-input` に直接依存する。

mod kana_jis;
mod romaji;
mod script;

pub use kana_jis::{JisKanaComposer, JisKey};
pub use romaji::{Composition, RomajiComposer, RomajiTable, TableError, romaji_to_kana};
pub use script::{Script, convert_script, hiragana_to_katakana};
