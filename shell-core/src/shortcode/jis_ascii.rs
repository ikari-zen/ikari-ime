//! かな入力（JIS）のキー → ASCII の表（ADR 0027 §3.1、§9.4。今の `JisKeyAscii.swift`）。
//!
//! かな入力の `KeyKind::Jis { code }`（`ikari_input::JisKey` の番号）を、ショートコードの名前を読むために
//! 刻印の英字・数字・記号に直す。名前の文字は `a-z 0-9 _ + -`（§3.3）。Shift で `_`、`+`。

/// けのキー（`:`）。入りと閉じのキー（`KeyCodes` の `jisKeys` の 16）。
pub const COLON_KEY: u8 = 16;
/// 数字の段（`1`〜`9`、`0`）。`JisKey` の 0〜9
const DIGIT_KEYS: std::ops::RangeInclusive<u8> = 0..=9;
/// `-` のキー（11 番の `^` ではなく 10 番）
const HYPHEN_KEY: u8 = 10;
/// `;` のキー。Shift で `+`
const PLUS_KEY: u8 = 15;
/// `ろ` のキー。Shift で `_`
const UNDERSCORE_KEY: u8 = 21;
/// A〜Z（`JisKey` の 22 から アルファベット順）
const LETTER_KEYS: std::ops::RangeInclusive<u8> = 22..=47;

/// 刻印の文字。`shifted` は Shift が付いているか。対応がなければ `None`。
pub fn character_for_jis_key(key: u8, shifted: bool) -> Option<char> {
    match key {
        // 数字の段は Shift で別のかな（ぬふあ…）になる。名前の文字ではない
        k if DIGIT_KEYS.contains(&k) => (!shifted).then(|| char::from(b'0' + (k + 1) % 10)),
        HYPHEN_KEY => (!shifted).then_some('-'),
        PLUS_KEY => shifted.then_some('+'),
        UNDERSCORE_KEY => shifted.then_some('_'),
        COLON_KEY => Some(':'),
        // Shift 付きの英字は大文字（名前の文字ではない）なので対応しない
        k if LETTER_KEYS.contains(&k) => {
            (!shifted).then(|| char::from(b'a' + (k - LETTER_KEYS.start())))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_are_the_key_tops() {
        for (offset, letter) in ('a'..='z').enumerate() {
            let key = 22 + u8::try_from(offset).unwrap();
            assert_eq!(character_for_jis_key(key, false), Some(letter));
            assert_eq!(character_for_jis_key(key, true), None);
        }
    }

    #[test]
    fn digits_and_symbols() {
        for (index, digit) in "1234567890".chars().enumerate() {
            let key = u8::try_from(index).unwrap();
            assert_eq!(character_for_jis_key(key, false), Some(digit));
            assert_eq!(character_for_jis_key(key, true), None);
        }
        assert_eq!(character_for_jis_key(10, false), Some('-'));
        assert_eq!(character_for_jis_key(15, true), Some('+'));
        assert_eq!(character_for_jis_key(21, true), Some('_'));
        assert_eq!(character_for_jis_key(15, false), None);
        assert_eq!(character_for_jis_key(21, false), None);
        assert_eq!(character_for_jis_key(COLON_KEY, false), Some(':'));
        assert_eq!(character_for_jis_key(COLON_KEY, true), Some(':'));
        for key in [11, 12, 13, 14, 17, 18, 19, 20, 48, 255] {
            assert_eq!(character_for_jis_key(key, false), None);
        }
    }
}
