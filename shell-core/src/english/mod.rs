//! 英語のモード（ADR 0026 D1、D4、D5、§5.2、§5.5。今の `EnglishMode.swift`。レーン L-A1）。
//! OS にもエンジンにも依存しない純関数と値型。
//!
//! 英語のモードの殻は、打鍵をアプリにそのまま渡す。殻がするのは、モードの切り替えキーの判定と、
//! 英語の文脈のたまり（`EnglishContextPool`）の更新、ショートコード（`shortcode`）と郵便番号・Unicode 表記（`postal`）の口だけ。

use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

use crate::diag::{DiagEpisode, DiagFinishedEpisode};
use crate::keymap::{KeyChord, Keymap, KeymapAction};
use crate::postal::PostalMachine;
use crate::shortcode::{ShortcodeMachine, ShortcodeMode, ShortcodeTable};
use crate::types::{KeyInput, KeyKind, MarkedText, ShellAction, Utf16Len};

mod postal;
mod shortcode;

pub use shortcode::{EnglishShortcodeEnvironment, EnglishTextOutcome};

/// Bundle ID の頭（逆ドメインの形）。値の出どころは `macos/scripts/lib.sh` の `IKARI_ID_PREFIX`
/// （Swift は `Identity.idPrefix`）。他の人が自分の値で使うときは、ビルドの設定で差し替える（ADR 0036 §3）。
pub const ID_PREFIX: &str = "io.github.ikari-zen";
/// 製品（殻）の ID。他の識別子はこの後ろに足す。
pub const PRODUCT_ID: &str = "io.github.ikari-zen.inputmethod.ikari";

/// JIS の英数キー（`kVK_JIS_Eisu`）とかなキー（`kVK_JIS_Kana`）の keyCode。macOS がモードを切り替えたあと、
/// 同じキーが切り替え先にも届く。素通しにするとアプリに余計な文字が入る（S-A の T1）ので、どのモードでも受け止めて何もしない。
pub const EISU_KEY_CODE: u16 = 102;
pub const KANA_KEY_CODE: u16 = 104;

/// ikari の入力モード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputMode {
    Japanese,
    English,
}

/// モードの ID（Info.plist の `ComponentInputModeDict` の辞書のキー。ADR 0026 §5.1、§12）。
pub mod input_mode_id {
    /// 日本語。独自の辞書のキー（`TISInputSourceID` も同じ値）
    pub const JAPANESE: &str = "io.github.ikari-zen.inputmethod.ikari.Japanese";
    /// 英語。macOS の標準のキー。英数キーの行き先がこのキーで決まる（S-A の T1）。
    /// 入力ソースの ID（`TISInputSourceID`）は `ENGLISH_INPUT_SOURCE`
    pub const ENGLISH: &str = "com.apple.inputmethod.Roman";
    /// 英語のモードの `TISInputSourceID`
    pub const ENGLISH_INPUT_SOURCE: &str = "io.github.ikari-zen.inputmethod.ikari.English";
}

impl InputMode {
    /// モードの ID から。知らない ID は `None`。
    pub fn from_mode_id(mode_id: &str) -> Option<Self> {
        match mode_id {
            input_mode_id::JAPANESE => Some(Self::Japanese),
            input_mode_id::ENGLISH | input_mode_id::ENGLISH_INPUT_SOURCE => Some(Self::English),
            _ => None,
        }
    }

    pub fn mode_id(self) -> &'static str {
        match self {
            Self::Japanese => input_mode_id::JAPANESE,
            Self::English => input_mode_id::ENGLISH,
        }
    }

    /// 日本語 ⇄ 英語。
    pub fn toggled(self) -> Self {
        match self {
            Self::Japanese => Self::English,
            Self::English => Self::Japanese,
        }
    }
}

/// 英語のモードの打鍵の分類（`EnglishMode::classify`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnglishKeyClass {
    /// secure（パスワード欄など。D5）。アプリに素通しにする。記録しない。文脈にも入れない
    PassThroughSecure,
    /// キー割り当ての `mode.*`。ショートコードの表示があれば確定してから、行き先のモードにする。キーは受け止める
    SwitchMode(InputMode),
    /// ショートコードの口（`EnglishMode::handle_text`）を通したあと、文脈のたまりに足して素通しにする
    Text,
}

/// 英語の文脈のたまり（ADR 0026 §5.5）。印字できる文字を足し、区切りで 1 件にして取り出す。
/// 取り出した文字列は、殻が `CommitText`（`reading` は空、`surface` はたまり）としてエンジンに送る。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnglishContextPool {
    text: String,
    scalar_count: usize,
}

impl EnglishContextPool {
    /// たまりの上限（Unicode のスカラーの数）。エンジンの `MAX_SURFACE_CHARS`（2000）に合わせる。数え方もエンジンと同じ（スカラー）
    pub const MAX_SCALARS: usize = 2000;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// たまりの最後の文字（`ShortcodeMachine` が語の頭を判定するのに読む。ADR 0027 §3.2）。空なら `None`（直前が分からない）。
    ///
    /// Swift の `Character`（見た目の 1 文字）ではなく最後の `char`。語の頭の判定（空白・改行・`(`・`[`・`{`）では
    /// 結合文字が続く極めて稀な場合を除き同じ結果になる。
    pub fn last_character(&self) -> Option<char> {
        self.text.chars().next_back()
    }

    /// 文字列を足す。上限に達したら、たまりを取り出して返す（区切り。たまりは空になる）。
    pub fn append(&mut self, addition: &str) -> Option<String> {
        if addition.is_empty() {
            return None;
        }
        self.text.push_str(addition);
        self.scalar_count += addition.chars().count();
        if self.scalar_count >= Self::MAX_SCALARS {
            self.flush()
        } else {
            None
        }
    }

    /// 末尾の 1 文字（見た目の 1 文字）を消す。空なら何もしない。
    pub fn remove_last(&mut self) {
        let start = last_grapheme_start(&self.text);
        let removed = self.text.split_off(start);
        self.scalar_count = self.scalar_count.saturating_sub(removed.chars().count());
    }

    /// たまりを取り出して空にする。空なら `None`（送らない）。
    pub fn flush(&mut self) -> Option<String> {
        if self.text.is_empty() {
            return None;
        }
        self.scalar_count = 0;
        Some(std::mem::take(&mut self.text))
    }

    /// 送らずに捨てる（secure になったとき）。
    pub fn discard(&mut self) {
        self.text.clear();
        self.scalar_count = 0;
    }
}

/// 英語のモードの打鍵の扱い。状態は文脈のたまりと、ショートコード・郵便番号の状態機械、診断の数え上げ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnglishMode {
    pool: EnglishContextPool,
    /// 絵文字のショートコード（ADR 0027 §3。B48）。英語のモードの規則で動く。処理は `english::shortcode`
    shortcode: ShortcodeMachine,
    /// 語頭の `te` + 郵便番号（ADR 0032 §9.1）と `u+` + 16 進。処理は `english::postal`
    postal: PostalMachine,
    /// いまのショートコードの合成 1 回の数（診断ログ。ADR 0034 §4.2 の `mode=shortcode_en`）。状態に入っていない間は `None`
    diag_episode: Option<DiagEpisode>,
    /// 終わった合成。コントローラが `take_finished_diag` で取り出して状態機械の `record_english_episode` に渡す
    finished_diag: Option<DiagFinishedEpisode>,
}

impl Default for EnglishMode {
    fn default() -> Self {
        Self::new()
    }
}

impl EnglishMode {
    pub fn new() -> Self {
        Self {
            pool: EnglishContextPool::new(),
            shortcode: ShortcodeMachine::new(
                Arc::new(ShortcodeTable::empty()),
                ShortcodeMode::English,
            ),
            postal: PostalMachine::new(),
            diag_episode: None,
            finished_diag: None,
        }
    }

    pub fn pool(&self) -> &EnglishContextPool {
        &self.pool
    }

    /// 英数・かなキー（受け止めるだけのキー）。
    pub fn absorbs_key_code(key_code: u16) -> bool {
        key_code == EISU_KEY_CODE || key_code == KANA_KEY_CODE
    }

    /// キーを分類する（ADR 0026 §5.2 の 4 と 5-2）。順は、secure → `mode.*` → 文字。
    /// secure は `input.secure`。`current` は今のモード（`mode.toggle` の行き先を決める）。
    pub fn classify(input: &KeyInput, keymap: &Keymap, current: InputMode) -> EnglishKeyClass {
        if input.secure {
            return EnglishKeyClass::PassThroughSecure;
        }
        match Self::mode_switch_target(input, keymap, current) {
            Some(target) => EnglishKeyClass::SwitchMode(target),
            None => EnglishKeyClass::Text,
        }
    }

    /// キー割り当ての `mode.*` に当たるキーなら、行き先のモード。日本語・英語のどちらのモードでも使う。
    pub fn mode_switch_target(
        input: &KeyInput,
        keymap: &Keymap,
        current: InputMode,
    ) -> Option<InputMode> {
        let chord = Self::chord(input)?;
        match keymap.action(&chord)? {
            KeymapAction::ModeToggle => Some(current.toggled()),
            KeymapAction::ModeEnglish => Some(InputMode::English),
            KeymapAction::ModeJapanese => Some(InputMode::Japanese),
            _ => None,
        }
    }

    /// 打鍵を、キー割り当ての表記（`KeyChord`）に直す。文字のキーは、Ctrl・Option・Command があるときだけ割り当てを引く
    /// （`machine` の割り当ての引き方と同じ。素の文字は割り当てにならない）。
    pub fn chord(input: &KeyInput) -> Option<KeyChord> {
        let named = |name: &str| Some(KeyChord::new(name, input.modifiers));
        match &input.key {
            KeyKind::Function { number } => named(&format!("f{number}")),
            KeyKind::Character { text } => {
                let mut chars = text.chars();
                let (Some(only), None) = (chars.next(), chars.next()) else {
                    return None;
                };
                if !has_command_modifiers(input) {
                    return None;
                }
                Some(KeyChord::new(
                    only.to_lowercase().collect::<String>(),
                    input.modifiers,
                ))
            }
            KeyKind::Space => named("space"),
            KeyKind::Enter => named("enter"),
            KeyKind::Escape => named("escape"),
            KeyKind::Backspace => named("backspace"),
            KeyKind::Left => named("left"),
            KeyKind::Right => named("right"),
            KeyKind::Up => named("up"),
            KeyKind::Down => named("down"),
            KeyKind::Jis { .. } | KeyKind::Other | KeyKind::Eisu | KeyKind::Kana => None,
        }
    }

    // MARK: 文脈のたまり（ADR 0026 §5.5）

    /// たまりの最後の文字（`ShortcodeMachine` に渡す）。
    pub fn last_character(&self) -> Option<char> {
        self.pool.last_character()
    }

    /// 素通しにする打鍵を、たまりに反映する。区切りになったら（Return、Tab、矢印、Escape、F キー、
    /// Ctrl・Option・Command 付き、制御文字、たまりが上限）、取り出したたまりを返す。送る側は `reading` を空にして `CommitText` にする。
    /// - 印字できる文字（Space を含む）: たまりに足す
    /// - Backspace（修飾キーなし、または Shift だけ）: 末尾の 1 文字を消す
    pub fn record(&mut self, input: &KeyInput) -> Option<String> {
        if has_command_modifiers(input) {
            return self.flush();
        }
        match &input.key {
            KeyKind::Character { text } => {
                if is_printable(text) {
                    self.pool.append(text)
                } else {
                    self.flush()
                }
            }
            KeyKind::Space => self.pool.append(" "),
            KeyKind::Backspace => {
                self.pool.remove_last();
                None
            }
            KeyKind::Enter
            | KeyKind::Escape
            | KeyKind::Left
            | KeyKind::Right
            | KeyKind::Up
            | KeyKind::Down
            | KeyKind::Function { .. }
            | KeyKind::Jis { .. }
            | KeyKind::Other
            | KeyKind::Eisu
            | KeyKind::Kana => self.flush(),
        }
    }

    /// 区切り（モードの切り替え、`deactivateServer`、ショートコードの確定のあとなど）。たまりを取り出して空にする。
    pub fn flush(&mut self) -> Option<String> {
        self.pool.flush()
    }

    /// 確定した文字列（ショートコードの確定など）をたまりに足す。上限に達したら取り出す。
    pub fn append_committed(&mut self, text: &str) -> Option<String> {
        self.pool.append(text)
    }

    /// 送らずに捨てる（secure になったとき）。
    pub fn discard(&mut self) {
        self.pool.discard();
    }
}

/// Ctrl・Option・Command のどれかが付いている。
fn has_command_modifiers(input: &KeyInput) -> bool {
    input.modifiers.control || input.modifiers.option || input.modifiers.command
}

/// 制御文字、私用領域（矢印キーなどの U+F700 台）、未割り当て以外。`machine` の判定と同じ。
///
/// 未割り当て（Unicode の Cn）は、標準ライブラリに表がないため、非文字（U+FDD0〜FDEF、各面の末尾 2 つ）だけを弾く。
/// それ以外の未割り当ての文字は印字できる扱いになる（Swift の `generalCategory` との差。キーから届くことはまずない）。
pub fn is_printable(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| !(c.is_control() || is_private_use(c) || is_noncharacter(c)))
}

fn is_private_use(c: char) -> bool {
    matches!(c, '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}')
}

/// Unicode の非文字（U+FDD0〜FDEF と、各面の U+xFFFE・U+xFFFF）。
pub fn is_noncharacter(c: char) -> bool {
    let value = u32::from(c);
    (0xFDD0..=0xFDEF).contains(&value) || value & 0xFFFE == 0xFFFE
}

/// 下線の有無を指定した、強調つきのマーク（`highlighted_length` は全体）。
pub(crate) fn marked_text(text: &str, underlined: bool) -> ShellAction {
    ShellAction::SetMarkedText(MarkedText {
        highlighted_length: Utf16Len::of(text),
        underlined,
        ..MarkedText::plain(text)
    })
}

/// 末尾の「見た目の 1 文字」（拡張書記素クラスタ。Swift の `Character`）の始まりのバイト位置。空なら 0。
fn last_grapheme_start(text: &str) -> usize {
    text.grapheme_indices(true)
        .next_back()
        .map_or(0, |(start, _)| start)
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::types::{KeyInput, KeyKind, Modifiers, SettingsSnapshot};

    /// 既定の設定の一式（`SettingsSnapshot::default()`。重複を持たない）。
    pub fn default_settings() -> SettingsSnapshot {
        SettingsSnapshot::default()
    }

    pub fn key(kind: KeyKind) -> KeyInput {
        key_with(kind, Modifiers::NONE)
    }

    pub fn key_with(kind: KeyKind, modifiers: Modifiers) -> KeyInput {
        KeyInput {
            key: kind,
            modifiers,
            is_repeat: false,
            secure: false,
            layout_text: None,
        }
    }

    pub fn ch(text: &str) -> KeyInput {
        key(KeyKind::Character { text: text.into() })
    }

    pub fn ch_with(text: &str, modifiers: Modifiers) -> KeyInput {
        key_with(KeyKind::Character { text: text.into() }, modifiers)
    }

    pub fn mods(shift: bool, control: bool, option: bool, command: bool) -> Modifiers {
        Modifiers {
            shift,
            control,
            option,
            command,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::test_support::*;
    use super::*;
    use crate::types::Modifiers;

    const MACOS: &str = include_str!("../../../macos/Resources/keymap/macos.tsv");
    const MS_IME: &str = include_str!("../../../macos/Resources/keymap/ms-ime.tsv");

    #[test]
    fn mode_ids_map_to_modes() {
        assert_eq!(
            InputMode::from_mode_id("io.github.ikari-zen.inputmethod.ikari.Japanese"),
            Some(InputMode::Japanese)
        );
        // 英語のモードは macOS の標準の辞書のキーで通知される（S-A の T1）。入力ソースの ID でも読む
        assert_eq!(
            InputMode::from_mode_id("com.apple.inputmethod.Roman"),
            Some(InputMode::English)
        );
        assert_eq!(
            InputMode::from_mode_id("io.github.ikari-zen.inputmethod.ikari.English"),
            Some(InputMode::English)
        );
        assert_eq!(
            InputMode::from_mode_id("com.apple.inputmethod.Japanese"),
            None
        );
        assert_eq!(InputMode::from_mode_id(""), None);
    }

    #[test]
    fn mode_ids_round_trip_and_toggle() {
        for mode in [InputMode::Japanese, InputMode::English] {
            assert_eq!(InputMode::from_mode_id(mode.mode_id()), Some(mode));
        }
        assert_eq!(InputMode::Japanese.toggled(), InputMode::English);
        assert_eq!(InputMode::English.toggled(), InputMode::Japanese);
        assert_eq!(input_mode_id::JAPANESE, format!("{PRODUCT_ID}.Japanese"));
        assert_eq!(
            input_mode_id::ENGLISH_INPUT_SOURCE,
            format!("{PRODUCT_ID}.English")
        );
        assert!(PRODUCT_ID.starts_with(ID_PREFIX));
    }

    #[test]
    fn eisu_and_kana_keys_are_absorbed() {
        assert!(EnglishMode::absorbs_key_code(102));
        assert!(EnglishMode::absorbs_key_code(104));
        assert!(!EnglishMode::absorbs_key_code(0));
        // macOS の space の keyCode
        assert!(!EnglishMode::absorbs_key_code(49));
    }

    fn chord(text: &str) -> KeyChord {
        KeyChord::parse(text).unwrap()
    }

    #[test]
    fn presets_bind_no_mode_action() {
        for tsv in [MACOS, MS_IME] {
            let keymap = Keymap::from_preset(tsv, &BTreeMap::new()).unwrap();
            assert!(!keymap.bindings.values().any(|a| a.is_mode_switch()));
        }
        let mode_actions: Vec<KeymapAction> = KeymapAction::ALL
            .into_iter()
            .filter(|a| a.is_mode_switch())
            .collect();
        assert_eq!(
            mode_actions,
            [
                KeymapAction::ModeToggle,
                KeymapAction::ModeEnglish,
                KeymapAction::ModeJapanese
            ]
        );
        assert!(mode_actions.iter().all(|a| !a.is_script_conversion()));
        assert_eq!(KeymapAction::ModeToggle.name(), "mode.toggle");
        assert_eq!(KeymapAction::ModeEnglish.name(), "mode.english");
        assert_eq!(KeymapAction::ModeJapanese.name(), "mode.japanese");
    }

    fn settings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn settings_can_bind_mode_actions() {
        let overrides = settings(&[
            ("mode.toggle", "ctrl+;"),
            ("mode.english", "f13"),
            ("mode.japanese", "f14"),
        ]);
        let keymap = Keymap::from_settings(MACOS, &overrides).unwrap();
        assert_eq!(
            keymap.action(&chord("f13")),
            Some(KeymapAction::ModeEnglish)
        );
        assert_eq!(
            keymap.action(&chord("f14")),
            Some(KeymapAction::ModeJapanese)
        );
        // 既定の割り当て（半角カタカナ）を奪う
        assert_eq!(
            keymap.action(&chord("ctrl+;")),
            Some(KeymapAction::ModeToggle)
        );
    }

    #[test]
    fn mode_keys_resolve_in_both_modes() {
        let overrides = settings(&[
            ("mode.toggle", "f13"),
            ("mode.english", "f14"),
            ("mode.japanese", "ctrl+shift+j"),
        ]);
        let keymap = Keymap::from_settings(MACOS, &overrides).unwrap();
        let f = |n: u8| key(KeyKind::Function { number: n });
        let ctrl_shift_j = ch_with("j", mods(true, true, false, false));
        let target =
            |input: &KeyInput, current| EnglishMode::mode_switch_target(input, &keymap, current);
        assert_eq!(
            target(&f(13), InputMode::Japanese),
            Some(InputMode::English)
        );
        assert_eq!(
            target(&f(13), InputMode::English),
            Some(InputMode::Japanese)
        );
        assert_eq!(target(&f(14), InputMode::English), Some(InputMode::English));
        assert_eq!(
            target(&f(14), InputMode::Japanese),
            Some(InputMode::English)
        );
        assert_eq!(
            target(&ctrl_shift_j, InputMode::English),
            Some(InputMode::Japanese)
        );
        // 割り当てのないキー、素の文字は何もしない
        assert_eq!(target(&f(15), InputMode::English), None);
        assert_eq!(target(&ch("j"), InputMode::English), None);
        // 既定（割り当てなし）では、どのキーも切り替えにならない
        let plain = Keymap::from_preset(MACOS, &BTreeMap::new()).unwrap();
        assert_eq!(
            EnglishMode::mode_switch_target(&f(13), &plain, InputMode::English),
            None
        );
    }

    fn classify_keymap() -> Keymap {
        Keymap::new(
            [
                (KeyChord::plain("f13"), KeymapAction::ModeToggle),
                (KeyChord::plain("f6"), KeymapAction::ToHiragana),
            ]
            .into_iter()
            .collect(),
        )
    }

    #[test]
    fn secure_is_passed_through_before_anything_else() {
        // secure なら、mode.* のキーも文字も素通し（ADR 0026 D5）
        let keymap = classify_keymap();
        for kind in [
            KeyKind::Function { number: 13 },
            KeyKind::Character { text: "a".into() },
        ] {
            let mut input = key(kind);
            input.secure = true;
            assert_eq!(
                EnglishMode::classify(&input, &keymap, InputMode::English),
                EnglishKeyClass::PassThroughSecure
            );
        }
    }

    #[test]
    fn mode_key_switches() {
        let input = key(KeyKind::Function { number: 13 });
        assert_eq!(
            EnglishMode::classify(&input, &classify_keymap(), InputMode::English),
            EnglishKeyClass::SwitchMode(InputMode::Japanese)
        );
    }

    #[test]
    fn other_keys_are_text() {
        for kind in [
            KeyKind::Character { text: "a".into() },
            KeyKind::Space,
            KeyKind::Enter,
            KeyKind::Left,
            KeyKind::Function { number: 6 },
            KeyKind::Other,
        ] {
            assert_eq!(
                EnglishMode::classify(&key(kind.clone()), &classify_keymap(), InputMode::English),
                EnglishKeyClass::Text,
                "{kind:?}"
            );
        }
    }

    fn press(mode: &mut EnglishMode, kind: KeyKind, modifiers: Modifiers) -> Option<String> {
        mode.record(&key_with(kind, modifiers))
    }

    fn type_text(mode: &mut EnglishMode, text: &str) {
        for character in text.chars() {
            assert_eq!(
                press(
                    mode,
                    KeyKind::Character {
                        text: character.to_string()
                    },
                    Modifiers::NONE
                ),
                None
            );
        }
    }

    fn character(text: &str) -> KeyKind {
        KeyKind::Character { text: text.into() }
    }

    #[test]
    fn printable_characters_accumulate_and_space_is_included() {
        let mut mode = EnglishMode::new();
        type_text(&mut mode, "hello");
        assert_eq!(press(&mut mode, KeyKind::Space, Modifiers::NONE), None);
        type_text(&mut mode, "World");
        assert_eq!(
            press(&mut mode, character("!"), mods(true, false, false, false)),
            None
        );
        assert_eq!(mode.pool().text(), "hello World!");
        assert_eq!(mode.last_character(), Some('!'));
    }

    #[test]
    fn shifted_letters_are_text_but_chords_are_separators() {
        let mut mode = EnglishMode::new();
        type_text(&mut mode, "ab");
        assert_eq!(
            press(&mut mode, character("C"), mods(true, false, false, false)),
            None
        );
        assert_eq!(mode.pool().text(), "abC");
        for modifiers in [
            mods(false, true, false, false),
            mods(false, false, true, false),
            mods(false, false, false, true),
            mods(true, false, false, true),
        ] {
            let mut m = EnglishMode::new();
            type_text(&mut m, "ab");
            assert_eq!(
                press(&mut m, character("c"), modifiers).as_deref(),
                Some("ab"),
                "{modifiers:?}"
            );
            assert!(m.pool().is_empty());
        }
    }

    #[test]
    fn backspace_removes_the_last_character() {
        let mut mode = EnglishMode::new();
        type_text(&mut mode, "abc");
        assert_eq!(press(&mut mode, KeyKind::Backspace, Modifiers::NONE), None);
        assert_eq!(mode.pool().text(), "ab");
        // 結合文字や絵文字は見た目の 1 文字ごと
        mode.append_committed("👨\u{200D}👩\u{200D}👧");
        assert_eq!(press(&mut mode, KeyKind::Backspace, Modifiers::NONE), None);
        assert_eq!(mode.pool().text(), "ab");
        assert_eq!(
            press(
                &mut mode,
                KeyKind::Backspace,
                mods(true, false, false, false)
            ),
            None
        );
        assert_eq!(mode.pool().text(), "a");
        // Option+Backspace は語の削除なので、たまりを区切る
        assert_eq!(
            press(
                &mut mode,
                KeyKind::Backspace,
                mods(false, false, true, false)
            )
            .as_deref(),
            Some("a")
        );
    }

    #[test]
    fn backspace_removes_whole_clusters() {
        for cluster in ["e\u{0301}", "👍🏽", "🇯🇵", "❤\u{FE0F}", "\r\n", "か\u{3099}"] {
            let mut pool = EnglishContextPool::new();
            pool.append("ab");
            pool.append(cluster);
            pool.remove_last();
            assert_eq!(pool.text(), "ab", "{cluster:?}");
        }
        // 国旗が 2 つ並んでも 1 つずつ
        let mut flags = EnglishContextPool::new();
        flags.append("🇯🇵🇺🇸");
        flags.remove_last();
        assert_eq!(flags.text(), "🇯🇵");
        flags.remove_last();
        assert!(flags.is_empty());
        // スカラーの数も合わせて減る
        let mut counted = EnglishContextPool::new();
        counted.append("a👨\u{200D}👩\u{200D}👧");
        counted.remove_last();
        assert_eq!(counted.text(), "a");
        assert_eq!(counted.append(&"b".repeat(1998)), None);
        assert!(counted.append("c").is_some());
    }

    #[test]
    fn backspace_on_an_empty_pool_does_nothing() {
        let mut mode = EnglishMode::new();
        assert_eq!(press(&mut mode, KeyKind::Backspace, Modifiers::NONE), None);
        assert!(mode.pool().is_empty());
        assert_eq!(mode.last_character(), None);
        assert_eq!(mode.flush(), None);
    }

    #[test]
    fn separators_emit_the_pool_as_one_item() {
        let kinds = [
            KeyKind::Enter,
            KeyKind::Left,
            KeyKind::Right,
            KeyKind::Up,
            KeyKind::Down,
            KeyKind::Escape,
            KeyKind::Function { number: 5 },
            KeyKind::Other,
            KeyKind::Jis { code: 22 },
            character("\t"),
            character("\u{19}"),
            character("\u{F700}"),
            character(""),
        ];
        for kind in kinds {
            let mut mode = EnglishMode::new();
            type_text(&mut mode, "hi there");
            assert_eq!(
                press(&mut mode, kind.clone(), Modifiers::NONE).as_deref(),
                Some("hi there"),
                "{kind:?}"
            );
            assert!(mode.pool().is_empty());
            // たまりが空のときの区切りは何も出さない（空の履歴を送らない）
            assert_eq!(press(&mut mode, kind.clone(), Modifiers::NONE), None);
        }
    }

    #[test]
    fn mode_switch_and_deactivate_flush_through_flush() {
        let mut mode = EnglishMode::new();
        type_text(&mut mode, "abc");
        assert_eq!(mode.flush().as_deref(), Some("abc"));
        assert_eq!(mode.flush(), None);
        // ショートコードで確定した文字列（絵文字を含む）もたまりに足す
        assert_eq!(mode.append_committed("👀"), None);
        type_text(&mut mode, "x");
        assert_eq!(mode.pool().text(), "👀x");
        assert_eq!(mode.last_character(), Some('x'));
    }

    #[test]
    fn discard_drops_without_emitting() {
        let mut mode = EnglishMode::new();
        type_text(&mut mode, "secret");
        mode.discard();
        assert!(mode.pool().is_empty());
        assert_eq!(mode.flush(), None);
    }

    #[test]
    fn pool_is_emitted_when_it_reaches_the_limit() {
        let mut mode = EnglishMode::new();
        let limit = EnglishContextPool::MAX_SCALARS;
        assert_eq!(limit, 2000);
        for _ in 0..(limit - 1) {
            assert_eq!(press(&mut mode, character("a"), Modifiers::NONE), None);
        }
        assert_eq!(mode.pool().text().chars().count(), limit - 1);
        let emitted = press(&mut mode, character("a"), Modifiers::NONE);
        assert_eq!(emitted.map(|s| s.chars().count()), Some(limit));
        assert!(mode.pool().is_empty());
        // 次の 1 文字から新しいたまり
        assert_eq!(press(&mut mode, character("b"), Modifiers::NONE), None);
        assert_eq!(mode.pool().text(), "b");
    }

    #[test]
    fn limit_counts_unicode_scalars_like_the_engine() {
        // 家族の絵文字は見た目 1 文字だが 5 スカラー。エンジンの上限（スカラー）を超えないよう、スカラーで数える
        let mut pool = EnglishContextPool::new();
        let family = "👨\u{200D}👩\u{200D}👧";
        assert_eq!(family.chars().count(), 5);
        let mut emitted = None;
        let mut added = 0;
        while emitted.is_none() {
            emitted = pool.append(family);
            added += 1;
        }
        assert_eq!(added, EnglishContextPool::MAX_SCALARS / 5);
        assert_eq!(
            emitted.map(|s| s.chars().count()),
            Some(EnglishContextPool::MAX_SCALARS)
        );
        // 巨大な 1 回の追加でも、取り出す（呼ぶ側が上限を超えた 1 件を送る。エンジンは 2000 超を断る）
        let mut big = EnglishContextPool::new();
        assert_eq!(
            big.append(&"x".repeat(2500)).map(|s| s.chars().count()),
            Some(2500)
        );
    }

    #[test]
    fn empty_addition_is_ignored() {
        let mut pool = EnglishContextPool::new();
        assert_eq!(pool.append(""), None);
        assert!(pool.is_empty());
    }

    #[test]
    fn printable_rejects_control_private_use_and_empty() {
        assert!(is_printable("a"));
        assert!(is_printable("あ"));
        assert!(is_printable("😀"));
        assert!(!is_printable(""));
        assert!(!is_printable("\u{19}"));
        assert!(!is_printable("\t"));
        assert!(!is_printable("\u{F700}"));
        assert!(!is_printable("\u{FFFF}"));
        assert!(!is_printable("a\u{0}"));
    }

    #[test]
    fn default_settings_helper_matches_the_documented_defaults() {
        let settings = default_settings();
        assert!(!settings.emoji_shortcode);
        assert!(
            settings
                .emoji_shortcode_exclude
                .contains(&"com.apple.Terminal".to_string())
        );
        assert!(
            !settings
                .emoji_shortcode_exclude
                .contains(&"com.apple.TextEdit".to_string())
        );
    }
}
