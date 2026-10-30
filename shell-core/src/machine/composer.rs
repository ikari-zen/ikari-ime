//! 合成器（ローマ字、JIS かな）と字種変換。今の `Composing`・`ScriptConverting` と、その実装の `RustComposer`・`RustScripts`
//! （shell-ffi の `ikari_composer_*`・`ikari_convert_script`）。`shell-core` は `ikari-input` を直接使う（ADR 0041 §4.4）。

use std::sync::OnceLock;

use ikari_input::{JisKanaComposer, JisKey, RomajiComposer, RomajiTable, Script, convert_script};

use crate::keymap::KeymapAction;
use crate::types::InputMethodKind;

/// 合成器。読めないローマ字表のときは `Unavailable`（何も受け取らないので、キーはアプリに渡る。今の `UnavailableComposer`）。
pub enum Composer {
    /// `RomajiComposer` には空にする口も表を返す口もないので、作り直すための表を一緒に持つ
    Romaji {
        table: &'static RomajiTable,
        composer: RomajiComposer<'static>,
    },
    Jis(JisKanaComposer),
    Unavailable,
}

impl std::fmt::Debug for Composer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Romaji { .. } => f.write_str("Composer::Romaji"),
            Self::Jis(_) => f.write_str("Composer::Jis"),
            Self::Unavailable => f.write_str("Composer::Unavailable"),
        }
    }
}

/// 同梱のローマ字表（プロセスで 1 つ）。
fn builtin_table() -> &'static RomajiTable {
    static TABLE: OnceLock<RomajiTable> = OnceLock::new();
    TABLE.get_or_init(RomajiTable::builtin)
}

/// ローマ字表。同梱の表か、ユーザーの表（`romaji.tsv`）。
#[derive(Clone, Copy)]
pub struct RomajiSource {
    table: Option<&'static RomajiTable>,
}

impl RomajiSource {
    /// 同梱の表。
    pub fn builtin() -> Self {
        Self {
            table: Some(builtin_table()),
        }
    }

    /// ユーザーの表の文字列。空なら同梱の表、読めなければ使えない（今の `RustComposer` が nil になる場合と同じ）。
    ///
    /// 表は `RomajiComposer` が借りるので、プロセスの終わりまで手放さない（`Shell` を作るときに 1 回だけ読む。数 KB）。
    pub fn custom(tsv: &str) -> Self {
        if tsv.is_empty() {
            return Self::builtin();
        }
        Self {
            table: RomajiTable::parse(tsv)
                .ok()
                .map(|table| &*Box::leak(Box::new(table))),
        }
    }
}

impl std::fmt::Debug for RomajiSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.table.is_some() {
            "RomajiSource"
        } else {
            "RomajiSource(unavailable)"
        })
    }
}

impl Composer {
    pub fn new(method: InputMethodKind, romaji: RomajiSource) -> Self {
        match method {
            InputMethodKind::Romaji => match romaji.table {
                Some(table) => Self::Romaji {
                    table,
                    composer: RomajiComposer::new(table),
                },
                None => Self::Unavailable,
            },
            InputMethodKind::Kana => Self::Jis(JisKanaComposer::new()),
        }
    }

    pub fn kana(&self) -> &str {
        match self {
            Self::Romaji { composer, .. } => composer.kana(),
            Self::Jis(composer) => composer.kana(),
            Self::Unavailable => "",
        }
    }

    pub fn pending(&self) -> &str {
        match self {
            Self::Romaji { composer, .. } => composer.pending(),
            Self::Jis(_) | Self::Unavailable => "",
        }
    }

    /// 文字を 1 つ足す（ローマ字だけ。JIS かなの合成器は文字を受け取らない）。
    pub fn push_character(&mut self, character: char) {
        if let Self::Romaji { composer, .. } = self {
            composer.push(character);
        }
    }

    /// JIS の物理キー（かな入力だけ）。
    pub fn push_jis(&mut self, code: u8, shift: bool) {
        if let (Self::Jis(composer), Some(key)) = (self, JisKey::from_index(code)) {
            composer.push(key, shift);
        }
    }

    pub fn backspace(&mut self) -> bool {
        match self {
            Self::Romaji { composer, .. } => composer.backspace(),
            Self::Jis(composer) => composer.backspace(),
            Self::Unavailable => false,
        }
    }

    pub fn flush(&mut self) {
        if let Self::Romaji { composer, .. } = self {
            composer.flush();
        }
    }

    pub fn clear(&mut self) {
        match self {
            Self::Romaji { table, composer } => *composer = RomajiComposer::new(table),
            Self::Jis(composer) => *composer = JisKanaComposer::new(),
            Self::Unavailable => {}
        }
    }
}

/// 字種変換（F6〜F10）。字種変換でない操作は元の文字列。
pub fn convert_to_script(text: &str, action: KeymapAction) -> String {
    let script = match action {
        KeymapAction::ToHiragana => Script::Hiragana,
        KeymapAction::ToFullwidthKatakana => Script::FullwidthKatakana,
        KeymapAction::ToHalfwidthKatakana => Script::HalfwidthKatakana,
        KeymapAction::ToFullwidthAlnum => Script::FullwidthAlnum,
        KeymapAction::ToHalfwidthAlnum => Script::HalfwidthAlnum,
        _ => return text.to_string(),
    };
    convert_script(text, script)
}
