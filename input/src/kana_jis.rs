//! JISかな入力（ADR 0001 §4.2）。標準の JIS かな配列をコード定数で持つ。

/// 宣言順の番号（`as u8`）が FFI の `jis_key`（`ikari_shell.h` の IKARI_JIS_*）になる。順序を変えない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum JisKey {
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Digit0,
    Minus,
    Caret,
    Yen,
    At,
    BracketLeft,
    Semicolon,
    Colon,
    BracketRight,
    Comma,
    Period,
    Slash,
    /// ろ
    Backslash,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
}

/// 濁点（゛）と半濁点（゜）。互換文字（U+309B, U+309C）を使う。
const DAKUTEN: char = '\u{309B}';
const HANDAKUTEN: char = '\u{309C}';
const YEN_SIGN: char = '\u{a5}';

/// 1キー分の定義。
struct KeyDef {
    key: JisKey,
    /// 無シフトの刻印
    legend: char,
    /// 無シフトで出る文字
    plain: char,
    /// シフトで出る文字。出ないキーは None
    shifted: Option<char>,
}

const fn def(key: JisKey, legend: char, plain: char, shifted: Option<char>) -> KeyDef {
    KeyDef {
        key,
        legend,
        plain,
        shifted,
    }
}

const LAYOUT: &[KeyDef] = &[
    def(JisKey::Digit1, '1', 'ぬ', None),
    def(JisKey::Digit2, '2', 'ふ', None),
    def(JisKey::Digit3, '3', 'あ', Some('ぁ')),
    def(JisKey::Digit4, '4', 'う', Some('ぅ')),
    def(JisKey::Digit5, '5', 'え', Some('ぇ')),
    def(JisKey::Digit6, '6', 'お', Some('ぉ')),
    def(JisKey::Digit7, '7', 'や', Some('ゃ')),
    def(JisKey::Digit8, '8', 'ゆ', Some('ゅ')),
    def(JisKey::Digit9, '9', 'よ', Some('ょ')),
    def(JisKey::Digit0, '0', 'わ', Some('を')),
    def(JisKey::Minus, '-', 'ほ', None),
    def(JisKey::Caret, '^', 'へ', None),
    def(JisKey::Yen, YEN_SIGN, 'ー', None),
    def(JisKey::At, '@', DAKUTEN, None),
    def(JisKey::BracketLeft, '[', HANDAKUTEN, Some('「')),
    def(JisKey::Semicolon, ';', 'れ', None),
    def(JisKey::Colon, ':', 'け', None),
    def(JisKey::BracketRight, ']', 'む', Some('」')),
    def(JisKey::Comma, ',', 'ね', Some('、')),
    def(JisKey::Period, '.', 'る', Some('。')),
    def(JisKey::Slash, '/', 'め', Some('・')),
    def(JisKey::Backslash, '\\', 'ろ', None),
    def(JisKey::A, 'a', 'ち', None),
    def(JisKey::B, 'b', 'こ', None),
    def(JisKey::C, 'c', 'そ', None),
    def(JisKey::D, 'd', 'し', None),
    def(JisKey::E, 'e', 'い', Some('ぃ')),
    def(JisKey::F, 'f', 'は', None),
    def(JisKey::G, 'g', 'き', None),
    def(JisKey::H, 'h', 'く', None),
    def(JisKey::I, 'i', 'に', None),
    def(JisKey::J, 'j', 'ま', None),
    def(JisKey::K, 'k', 'の', None),
    def(JisKey::L, 'l', 'り', None),
    def(JisKey::M, 'm', 'も', None),
    def(JisKey::N, 'n', 'み', None),
    def(JisKey::O, 'o', 'ら', None),
    def(JisKey::P, 'p', 'せ', None),
    def(JisKey::Q, 'q', 'た', None),
    def(JisKey::R, 'r', 'す', None),
    def(JisKey::S, 's', 'と', None),
    def(JisKey::T, 't', 'か', None),
    def(JisKey::U, 'u', 'な', None),
    def(JisKey::V, 'v', 'ひ', None),
    def(JisKey::W, 'w', 'て', None),
    def(JisKey::X, 'x', 'さ', None),
    def(JisKey::Y, 'y', 'ん', None),
    def(JisKey::Z, 'z', 'つ', Some('っ')),
];

/// 濁点を付けられるかなと、付けた結果。
const DAKUTEN_PAIRS: &[(char, char)] = &[
    ('か', 'が'),
    ('き', 'ぎ'),
    ('く', 'ぐ'),
    ('け', 'げ'),
    ('こ', 'ご'),
    ('さ', 'ざ'),
    ('し', 'じ'),
    ('す', 'ず'),
    ('せ', 'ぜ'),
    ('そ', 'ぞ'),
    ('た', 'だ'),
    ('ち', 'ぢ'),
    ('つ', 'づ'),
    ('て', 'で'),
    ('と', 'ど'),
    ('は', 'ば'),
    ('ひ', 'び'),
    ('ふ', 'ぶ'),
    ('へ', 'べ'),
    ('ほ', 'ぼ'),
    ('う', 'ゔ'),
];

/// 半濁点を付けられるかなと、付けた結果。
const HANDAKUTEN_PAIRS: &[(char, char)] = &[
    ('は', 'ぱ'),
    ('ひ', 'ぴ'),
    ('ふ', 'ぷ'),
    ('へ', 'ぺ'),
    ('ほ', 'ぽ'),
];

fn lookup_pair(pairs: &[(char, char)], base: char) -> Option<char> {
    pairs.iter().find(|(b, _)| *b == base).map(|(_, c)| *c)
}

impl JisKey {
    /// 全キー。添字が FFI の番号（宣言順）。
    pub const ALL: [JisKey; 48] = [
        JisKey::Digit1,
        JisKey::Digit2,
        JisKey::Digit3,
        JisKey::Digit4,
        JisKey::Digit5,
        JisKey::Digit6,
        JisKey::Digit7,
        JisKey::Digit8,
        JisKey::Digit9,
        JisKey::Digit0,
        JisKey::Minus,
        JisKey::Caret,
        JisKey::Yen,
        JisKey::At,
        JisKey::BracketLeft,
        JisKey::Semicolon,
        JisKey::Colon,
        JisKey::BracketRight,
        JisKey::Comma,
        JisKey::Period,
        JisKey::Slash,
        JisKey::Backslash,
        JisKey::A,
        JisKey::B,
        JisKey::C,
        JisKey::D,
        JisKey::E,
        JisKey::F,
        JisKey::G,
        JisKey::H,
        JisKey::I,
        JisKey::J,
        JisKey::K,
        JisKey::L,
        JisKey::M,
        JisKey::N,
        JisKey::O,
        JisKey::P,
        JisKey::Q,
        JisKey::R,
        JisKey::S,
        JisKey::T,
        JisKey::U,
        JisKey::V,
        JisKey::W,
        JisKey::X,
        JisKey::Y,
        JisKey::Z,
    ];

    /// FFI の番号から引く。範囲外は None。
    pub fn from_index(index: u8) -> Option<JisKey> {
        Self::ALL.get(usize::from(index)).copied()
    }

    /// JIS 配列の無シフト刻印（'¥' はー、'\\' はろ）。
    pub fn from_legend(c: char) -> Option<JisKey> {
        LAYOUT.iter().find(|d| d.legend == c).map(|d| d.key)
    }

    fn definition(self) -> Option<&'static KeyDef> {
        LAYOUT.iter().find(|d| d.key == self)
    }
}

pub struct JisKanaComposer {
    kana: String,
}

impl JisKanaComposer {
    pub fn new() -> Self {
        Self {
            kana: String::new(),
        }
    }

    /// ゛(@) と ゜([) は直前のかなと合成する（か→が、は→ぱ、う→ゔ）。
    /// 合成できなければ ゛/゜ をそのまま残す。シフトで文字が出ないキーは何もしない。
    pub fn push(&mut self, key: JisKey, shift: bool) {
        let Some(def) = key.definition() else {
            return;
        };
        let Some(ch) = (if shift { def.shifted } else { Some(def.plain) }) else {
            return;
        };
        let pairs = match ch {
            DAKUTEN => DAKUTEN_PAIRS,
            HANDAKUTEN => HANDAKUTEN_PAIRS,
            _ => {
                self.kana.push(ch);
                return;
            }
        };
        let combined = self
            .kana
            .chars()
            .next_back()
            .and_then(|last| lookup_pair(pairs, last));
        match combined {
            Some(c) => {
                self.kana.pop();
                self.kana.push(c);
            }
            None => self.kana.push(ch),
        }
    }

    /// 末尾の1文字を削る。削れたら true。
    pub fn backspace(&mut self) -> bool {
        self.kana.pop().is_some()
    }

    pub fn kana(&self) -> &str {
        &self.kana
    }
}

impl Default for JisKanaComposer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_matches_declaration_order() {
        for (i, key) in JisKey::ALL.iter().enumerate() {
            assert_eq!(*key as usize, i);
            assert_eq!(JisKey::from_index(i as u8), Some(*key));
        }
        assert_eq!(JisKey::from_index(JisKey::ALL.len() as u8), None);
    }

    fn type_keys(keys: &[(char, bool)]) -> String {
        let mut c = JisKanaComposer::new();
        for &(legend, shift) in keys {
            c.push(JisKey::from_legend(legend).expect("legend"), shift);
        }
        c.kana().to_string()
    }

    fn plain(s: &str) -> String {
        let keys: Vec<(char, bool)> = s.chars().map(|c| (c, false)).collect();
        type_keys(&keys)
    }

    #[test]
    fn basic_keys() {
        assert_eq!(plain("tg"), "かき");
        assert_eq!(plain("4"), "う");
        assert_eq!(plain("\u{a5}"), "ー");
        assert_eq!(plain("\\"), "ろ");
        assert_eq!(plain("-"), "ほ");
        assert_eq!(plain("^"), "へ");
        assert_eq!(plain("]"), "む");
        assert_eq!(plain(":"), "け");
        assert_eq!(plain(";"), "れ");
        assert_eq!(plain(",./"), "ねるめ");
        assert_eq!(plain("qwertyuiop"), "たていすかんなにらせ");
    }

    #[test]
    fn home_and_bottom_rows() {
        assert_eq!(plain("asdfghjkl"), "ちとしはきくまのり");
        assert_eq!(plain("zxcvbnm"), "つさそひこみも");
    }

    #[test]
    fn legend_lookup() {
        assert_eq!(JisKey::from_legend('a'), Some(JisKey::A));
        assert_eq!(JisKey::from_legend('1'), Some(JisKey::Digit1));
        assert_eq!(JisKey::from_legend('0'), Some(JisKey::Digit0));
        assert_eq!(JisKey::from_legend('\u{a5}'), Some(JisKey::Yen));
        assert_eq!(JisKey::from_legend('\\'), Some(JisKey::Backslash));
        assert_eq!(JisKey::from_legend('@'), Some(JisKey::At));
        assert_eq!(JisKey::from_legend('['), Some(JisKey::BracketLeft));
        assert_eq!(JisKey::from_legend('A'), None);
        assert_eq!(JisKey::from_legend('あ'), None);
        assert_eq!(JisKey::from_legend('+'), None);
    }

    #[test]
    fn every_key_is_defined_once() {
        for d in LAYOUT {
            assert_eq!(JisKey::from_legend(d.legend), Some(d.key));
            assert!(d.key.definition().is_some());
        }
        // 48 キー（数字10 + 記号11 + 英字26 + ろ）
        assert_eq!(LAYOUT.len(), 48);
    }

    #[test]
    fn shifted_small_kana() {
        assert_eq!(type_keys(&[('e', true)]), "ぃ");
        assert_eq!(type_keys(&[('3', true)]), "ぁ");
        assert_eq!(type_keys(&[('4', true)]), "ぅ");
        assert_eq!(type_keys(&[('5', true)]), "ぇ");
        assert_eq!(type_keys(&[('6', true)]), "ぉ");
        assert_eq!(type_keys(&[('7', true)]), "ゃ");
        assert_eq!(type_keys(&[('8', true)]), "ゅ");
        assert_eq!(type_keys(&[('9', true)]), "ょ");
        assert_eq!(type_keys(&[('z', true)]), "っ");
        assert_eq!(type_keys(&[('0', true)]), "を");
    }

    #[test]
    fn shifted_symbols() {
        assert_eq!(type_keys(&[(',', true)]), "、");
        assert_eq!(type_keys(&[('.', true)]), "。");
        assert_eq!(type_keys(&[('/', true)]), "・");
        assert_eq!(type_keys(&[('[', true)]), "「");
        assert_eq!(type_keys(&[(']', true)]), "」");
    }

    #[test]
    fn shift_without_kana_is_ignored() {
        assert_eq!(type_keys(&[('t', false), ('a', true)]), "か");
    }

    #[test]
    fn dakuten_combines() {
        assert_eq!(plain("t@"), "が");
        assert_eq!(plain("f@"), "ば");
        assert_eq!(plain("4@"), "ゔ");
        assert_eq!(plain("q@"), "だ");
        assert_eq!(plain("z@"), "づ");
    }

    #[test]
    fn handakuten_combines() {
        assert_eq!(plain("f["), "ぱ");
        assert_eq!(plain("v["), "ぴ");
        assert_eq!(plain("2["), "ぷ");
        assert_eq!(plain("^["), "ぺ");
        assert_eq!(plain("-["), "ぽ");
    }

    #[test]
    fn mark_after_uncombinable_stays() {
        // あ は濁点を取れない
        assert_eq!(plain("3@"), "あ\u{309B}");
        // か は半濁点を取れない
        assert_eq!(plain("t["), "か\u{309C}");
        // 先頭の濁点
        assert_eq!(plain("@"), "\u{309B}");
    }

    #[test]
    fn dakuten_twice_does_not_stack() {
        assert_eq!(plain("t@@"), "が\u{309B}");
    }

    #[test]
    fn backspace_removes_composed_char_whole() {
        let mut c = JisKanaComposer::new();
        c.push(JisKey::T, false);
        c.push(JisKey::At, false);
        assert_eq!(c.kana(), "が");
        assert!(c.backspace());
        assert_eq!(c.kana(), "");
        assert!(!c.backspace());
    }
}
