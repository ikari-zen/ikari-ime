//! 入力モードに応じて READING を「かな + 未確定ローマ字」にする。

use ikari_input::{JisKanaComposer, RomajiTable, romaji_to_kana};

use crate::args::{InputMode, parse_jis_keys, resolve_input_mode};

#[derive(Debug, PartialEq, Eq)]
pub struct ComposedReading {
    /// 変換対象のかな（kana モードでは入力そのまま）
    pub kana: String,
    /// 未確定のローマ字（FR-07: 変換対象にしない）
    pub pending: String,
}

/// `table` は romaji モードのときだけ使う。jis の記法エラーは `Err`（引数の誤り扱い）。
pub fn compose_reading(
    mode: InputMode,
    text: &str,
    table: &RomajiTable,
) -> Result<ComposedReading, String> {
    match resolve_input_mode(mode, text) {
        InputMode::Kana | InputMode::Auto => Ok(ComposedReading {
            kana: text.to_string(),
            pending: String::new(),
        }),
        InputMode::Romaji => {
            let c = romaji_to_kana(table, text);
            Ok(ComposedReading {
                kana: c.kana,
                pending: c.pending,
            })
        }
        InputMode::Jis => {
            let mut composer = JisKanaComposer::new();
            for (key, shift) in parse_jis_keys(text)? {
                composer.push(key, shift);
            }
            Ok(ComposedReading {
                kana: composer.kana().to_string(),
                pending: String::new(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compose(mode: InputMode, text: &str) -> Result<ComposedReading, String> {
        compose_reading(mode, text, &RomajiTable::builtin())
    }

    #[test]
    fn auto_romaji() {
        let c = compose(InputMode::Auto, "hinshi").unwrap();
        assert_eq!(c.kana, "ひんし");
        assert_eq!(c.pending, "");
    }

    #[test]
    fn romaji_leaves_pending() {
        let c = compose(InputMode::Auto, "henk").unwrap();
        assert_eq!((c.kana.as_str(), c.pending.as_str()), ("へん", "k"));
    }

    #[test]
    fn auto_kana_is_passed_through() {
        let c = compose(InputMode::Auto, "ひんし").unwrap();
        assert_eq!(c.kana, "ひんし");
        let c = compose(InputMode::Kana, "ヒンシ").unwrap();
        assert_eq!(c.kana, "ヒンシ");
    }

    #[test]
    fn explicit_kana_with_ascii_is_not_converted() {
        let c = compose(InputMode::Kana, "ka").unwrap();
        assert_eq!(c.kana, "ka");
    }

    #[test]
    fn jis_mode() {
        let c = compose(InputMode::Jis, "t@+e").unwrap();
        assert_eq!(c.kana, "がぃ");
        assert!(compose(InputMode::Jis, "あ").is_err());
    }
}
